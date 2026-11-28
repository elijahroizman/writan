// Shared setup for the stress scripts: a browser on a VITE_TEST_HOOKS build with a fake Tauri
// bridge, so app-only code (saving, autosave, the close prompt, opening links) runs for real.
//
// Paths the app opens are real paths on this machine: read_file serves them from disk. Writes go
// to an in-memory overlay (window.__fake.files) that later reads see; nothing on disk changes.
// window.__fake.failWrite = 'message' makes every write_file reject with that message.
import { chromium, webkit } from 'playwright'
import { existsSync, mkdirSync, readFileSync, statSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

export const BASE = process.env.WRITAN_URL ?? 'http://localhost:1420/'
export const ENGINE = process.env.ENGINE ?? 'chromium'
export const TMP = join(tmpdir(), 'writan-stress')

// `scrollbars` shows classic scrollbars, which take room as on a Mac with a mouse or on Windows.
// `scale` is 2, as on a Retina Mac, so screenshots are sharp: at 1x a line of the document is 11
// pixels tall and looks faint and blurred on the screen of whoever reads it. SCALE=1 overrides it.
export async function launch({ scale = Number(process.env.SCALE) || 2, viewport = { width: 1400, height: 900 }, tauri = true, init, scrollbars = false, timezoneId, locale } = {}) {
  const browser = await (ENGINE === 'webkit' ? webkit : chromium).launch(ENGINE === 'webkit' ? {} : { args: ['--no-sandbox', '--enable-precise-memory-info'], ignoreDefaultArgs: scrollbars ? ['--hide-scrollbars'] : [] })
  mkdirSync(TMP, { recursive: true })
  const context = await browser.newContext({ viewport, deviceScaleFactor: scale, timezoneId, locale })
  const page = await context.newPage()
  page.setDefaultTimeout(15000)
  const logs = []
  page.on('console', (m) => {
    if (['error', 'warning'].includes(m.type())) logs.push(`[${m.type()}] ${m.text().slice(0, 300)}`)
  })
  page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message.slice(0, 300)}${process.env.STACKS ? '\n' + e.stack : ''}`))
  page.on('dialog', (d) => {
    logs.push(`[native-dialog] ${d.type()} ${d.message().slice(0, 100)}`)
    void d.dismiss()
  })
  // Popups are navigations away from the app (window.open in browser mode).
  context.on('page', (p) => logs.push(`[popup] ${p.url()}`))
  await context.route(/\/__fs(?:\?|$|\/)/, async (route) => {
    const url = new URL(route.request().url())
    let filePath = url.searchParams.get('path')
    if (filePath === null) {
      filePath = decodeURIComponent(url.pathname.slice('/__fs'.length))
      if (process.platform === 'win32' && /^\/[A-Za-z]:/.test(filePath)) filePath = filePath.slice(1)
    }
    if (url.searchParams.has('info')) {
      const exists = existsSync(filePath)
      const st = exists ? statSync(filePath) : null
      return route.fulfill({ json: { exists, isDir: !!st?.isDirectory(), size: st?.size ?? 0, modifiedMs: st ? Math.round(st.mtimeMs) : 0 } })
    }
    if (!existsSync(filePath)) return route.fulfill({ status: 404, body: 'missing' })
    return route.fulfill({ body: readFileSync(filePath), contentType: 'application/octet-stream' })
  })
  await page.addInitScript(() => localStorage.getItem('workspace.setupDone') || localStorage.setItem('workspace.setupDone', '1'))
  if (tauri) await page.addInitScript(fakeTauri)
  if (init) await page.addInitScript(init)
  return { browser, context, page, logs }
}

/** Runs in the page before the app: a stand-in for window.__TAURI_INTERNALS__. */
function fakeTauri() {
  let cb = 0
  const fake = {
    files: new Map(),
    writes: [],
    opened: [],
    calls: [],
    listeners: {},
    failWrite: null,
    failRead: null,
    writeDelay: 0,
    clock: Date.now(),
    savePath: null,
    destroyed: false,
    infoOverride: new Map(),
    emit(event, payload) {
      for (const id of fake.listeners[event] ?? []) window[`_${id}`]?.({ event, id: 0, payload })
    },
  }
  window.__fake = fake
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} }
  const fsUrl = (path, info = false) => `/__fs?path=${encodeURIComponent(path)}${info ? '&info=1' : ''}`
  fake.fsUrl = fsUrl
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
    plugins: {},
    transformCallback: (fn) => {
      const id = ++cb
      window[`_${id}`] = fn
      return id
    },
    unregisterCallback: () => {},
    convertFileSrc: (p) => p,
    invoke: async (cmd, args, opts) => {
      fake.calls.push(cmd)
      switch (cmd) {
        case 'read_file': {
          if (fake.failRead) throw fake.failRead
          if (fake.files.has(args.path)) return fake.files.get(args.path).buffer.slice(0)
          const res = await fetch(fsUrl(args.path))
          if (!res.ok) throw `No such file or directory (os error 2): ${args.path}`
          return await res.arrayBuffer()
        }
        case 'path_info': {
          if (fake.infoOverride.has(args.path)) return fake.infoOverride.get(args.path)
          if (fake.files.has(args.path)) return { exists: true, isDir: false, size: fake.files.get(args.path).length, modifiedMs: fake.files.get(args.path).mtime }
          return (await fetch(fsUrl(args.path, true))).json()
        }
        case 'write_file': {
          const path = decodeURIComponent(opts.headers['x-path'])
          if (fake.failWrite) throw fake.failWrite
          const bytes = new Uint8Array(args)
          if (fake.writeDelay) await new Promise((r) => setTimeout(r, fake.writeDelay))
          if (opts?.headers?.['x-no-clobber'] && fake.files.has(path)) throw 'Choose a new file name for the OCR copy.'
          bytes.mtime = ++fake.clock
          fake.files.set(path, bytes)
          fake.writes.push({ path, size: bytes.length, at: Date.now() })
          return null
        }
        case 'plugin:opener|open_url':
          fake.opened.push(args.url)
          // The real opener's default scope: http(s), mailto and tel only.
          if (!/^(https?:|mailto:|tel:)/i.test(args.url)) throw `Not allowed to open url ${args.url}`
          return null
        case 'plugin:dialog|save':
          return fake.savePath
        case 'plugin:dialog|open':
          return null
        case 'plugin:event|listen':
          ;(fake.listeners[args.event] ??= []).push(args.handler)
          return args.handler
        case 'plugin:event|unlisten':
          return null
        case 'plugin:window|destroy':
          fake.destroyed = true
          return null
        case 'take_pending_opens':
        case 'installed_plugins':
        case 'history_list':
          return []
        case 'history_usage':
          return 0
        case 'ocr_list_packs':
          return { installed: [], totalBytes: 0 }
        case 'cloud_folders':
          return []
        case 'release_notes':
        case 'plugin:updater|check':
          throw 'offline'
        default:
          return null
      }
    },
  }
}

export async function open(page, paths) {
  await page.evaluate((p) => window.__workspace.useWorkspace.getState().openPaths(p), Array.isArray(paths) ? paths : [paths])
}

/**
 * Saves a screenshot of `target` (a selector or a locator) to `path`, or of the whole window
 * without one. Crop to the part the picture is about: the whole window at 2x is 2800 pixels wide,
 * and chat apps and models shrink a picture that big, which blurs it again.
 */
export async function shot(page, path, target) {
  const el = typeof target === 'string' ? page.locator(target).first() : target
  await (el ?? page).screenshot({ path, animations: 'disabled' })
}

export const tabs = (page) =>
  page.evaluate(() => window.__workspace.useWorkspace.getState().tabs.map((t) => ({ id: t.id, kind: t.kind, title: t.title, path: t.path, dirty: t.dirty })))

export const active = (page) => page.evaluate(() => window.__workspace.useWorkspace.getState().activeId)

/** Visible text of the app's own dialog, or null. */
export const dialogText = (page) => page.evaluate(() => document.querySelector('.dialog, [role=dialog], [role=alertdialog]')?.textContent ?? null)

export async function start(page) {
  await page.goto(BASE)
  await page.waitForFunction(() => window.__workspace, null, { timeout: 15000 })
  // Close the welcome tour if it shows.
  await page.waitForTimeout(500)
  check('page identity', page.url() === BASE && /Writan|Workspace/i.test(await page.title()), `${page.url()} | ${await page.title()}`)
  check('app rendered without a framework overlay', await page.locator('.app').count() > 0 && await page.locator('vite-error-overlay').count() === 0)
}

let failures = 0
export function check(name, ok, detail = '') {
  if (!ok) failures++
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? '  — ' + detail : ''}`)
}
export const done = async (browser, logs) => {
  if (logs?.length) console.log(logs.slice(0, 30).join('\n'))
  check('no unhandled page errors', !logs?.some((l) => l.startsWith('[pageerror]')))
  await browser.close()
  process.exitCode = failures ? 1 : 0
}
