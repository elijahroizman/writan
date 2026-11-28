// A file changed by another app after Writan opened it: autosave must not overwrite it, and
// Save asks first. Runs against a VITE_TEST_HOOKS build with a faked Tauri backend.
//   node tests/stress/native/save-conflict.mjs http://localhost:4173/ [chromium|webkit]
import * as pw from 'playwright'

const url = process.argv[2] ?? 'http://localhost:4173/'
const browser = await pw[process.argv[3] ?? 'chromium'].launch()
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } })
const errors = []
page.on('pageerror', (e) => errors.push(e.message))
page.on('requestfailed', (r) => errors.push(`request failed: ${r.url()} ${r.failure()?.errorText}`))
await page.addInitScript(() => {
  localStorage.setItem('workspace.setupDone', '2')
  localStorage.setItem('workspace.settings', JSON.stringify({ version: 2, autosave: { enabled: true, delay: 1, kinds: {} }, history: true }))
  const enc = new TextEncoder()
  const files = new Map([['/docs/notes.md', { bytes: enc.encode('# Notes\n\nFirst.\n'), mtime: 1000 }]])
  const writes = []
  let cb = 0
  let opened = false
  window.__fake = {
    writes,
    files,
    outside(path, text) {
      const f = files.get(path)
      files.set(path, { bytes: enc.encode(text), mtime: f.mtime + 5000 })
    },
    text: (path) => new TextDecoder().decode(files.get(path).bytes),
  }
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
    transformCallback: (fn) => { const id = ++cb; window[`_${id}`] = fn; return id },
    unregisterCallback: () => {},
    convertFileSrc: (p) => p,
    invoke: async (cmd, args, opts) => {
      switch (cmd) {
        case 'path_info': {
          const f = files.get(args.path)
          return { exists: !!f, isDir: false, size: f?.bytes.length ?? 0, modifiedMs: f?.mtime ?? 0 }
        }
        case 'read_file': {
          const f = files.get(args.path)
          if (!f) throw `Couldn't open “${args.path}”`
          return f.bytes.slice().buffer
        }
        case 'write_file': {
          const path = decodeURIComponent(opts.headers['x-path'])
          const prev = files.get(path)
          files.set(path, { bytes: new Uint8Array(args), mtime: (prev?.mtime ?? 0) + 1000 })
          writes.push(path)
          return null
        }
        case 'take_pending_opens': {
          if (opened) return []
          opened = true
          return ['/docs/notes.md']
        }
        case 'plugin:event|listen': return ++cb
        case 'installed_plugins': case 'history_list': case 'list_dir': case 'walk_dir': return []
        default: return null
      }
    },
  }
})

const fail = (msg) => { console.log('FAIL', msg); process.exitCode = 1 }
const fake = (fn, ...a) => page.evaluate(([fn, a]) => window.__fake[fn](...a), [fn, a])
const writes = () => page.evaluate(() => window.__fake.writes.length)

await page.goto(url)
await page.waitForFunction(() => {
  const ws = window.__workspace
  const tab = ws?.useWorkspace.getState().tabs[0]
  const editor = tab && ws.getEditor(tab.id)
  if (!editor) return false
  editor.viewMode('write')
  return true
}, null, { timeout: 15000 })
await page.waitForSelector('.cm-content', { timeout: 15000 })
await page.click('.cm-content')
await page.keyboard.press('Control+End')
await page.keyboard.type('Mine one.')
await page.waitForTimeout(2500)
if ((await writes()) !== 1) fail(`autosave of an unchanged file: ${await writes()} writes`)

// Another app saves the file, then the user keeps typing.
await fake('outside', '/docs/notes.md', '# Notes\n\nTheirs.\n')
await page.keyboard.type(' Mine two.')
await page.waitForTimeout(2500)
if ((await writes()) !== 1) fail('autosave overwrote a file changed outside Writan')
if (!(await fake('text', '/docs/notes.md')).includes('Theirs.')) fail('outside change lost')
const dirty = await page.evaluate(() => window.__workspace.useWorkspace.getState().tabs[0].dirty)
if (!dirty) fail('tab not dirty after a refused autosave')

// Save asks; Cancel keeps their version, Save Anyway writes ours.
await page.keyboard.press('Control+s')
await page.waitForSelector('.dialog', { timeout: 5000 })
const title = await page.textContent('.dialog')
if (!/changed on disk/.test(title)) fail(`dialog says: ${title}`)
await page.getByRole('button', { name: 'Cancel' }).click()
await page.waitForTimeout(300)
if ((await writes()) !== 1) fail('Cancel still saved')
await page.keyboard.press('Control+s')
await page.getByRole('button', { name: 'Save Anyway' }).click()
await page.waitForTimeout(500)
if ((await writes()) !== 2) fail('Save Anyway did not save')
if (!(await fake('text', '/docs/notes.md')).includes('Mine two.')) fail('our version not written')

// After that, autosave carries on normally.
await page.click('.cm-content')
await page.keyboard.press('Control+End')
await page.keyboard.type(' Three.')
await page.waitForTimeout(2500)
if ((await writes()) !== 3) fail(`autosave after Save Anyway: ${await writes()} writes`)

if (errors.length) fail(`page errors: ${errors.join(' | ')}`)
console.log(process.exitCode ? 'save-conflict: FAILED' : 'save-conflict: ok')
await browser.close()
