import { execFileSync, spawn, spawnSync } from 'node:child_process'
import { once } from 'node:events'
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { createServer as createHttpServer } from 'node:http'
import os from 'node:os'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'
import { gzipSync } from 'node:zlib'
import { chromium } from 'playwright'

const root = fileURLToPath(new URL('../../', import.meta.url))
const fixturesDir = path.resolve(root, '.cache/perf-fixtures')
const tessdataDir = process.env.WRITAN_TESSDATA_DIR || path.resolve(root, '.cache/tessdata')
const HOST = '127.0.0.1'
const PORT = Number(process.env.WRITAN_PERF_PORT || 4173)
const BASE = `http://${HOST}:${PORT}/`

const args = process.argv.slice(2)
const argVal = (flag, def = null) => {
  const i = args.indexOf(flag)
  return i >= 0 && args[i + 1] ? args[i + 1] : def
}
const label = argVal('--label', 'run')
const outFile = path.resolve(root, argVal('--out', `.cache/perf-results/${label}.json`))
const skipRust = args.includes('--skip-rust')
const onlyFilter = argVal('--only', null)?.split(',').map((s) => s.trim()).filter(Boolean) ?? null
const want = (name) => !onlyFilter || onlyFilter.includes(name)

function summarize(samples) {
  const sorted = [...samples].sort((a, b) => a - b)
  const n = sorted.length
  if (!n) return { median: null, p95: null, min: null, max: null, mean: null, reps: 0, samples: [] }
  const mid = Math.floor(n / 2)
  const median = n % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2
  const p95 = sorted[Math.min(n - 1, Math.floor((n - 1) * 0.95))]
  const mean = sorted.reduce((a, b) => a + b, 0) / n
  const r = (v) => Math.round(v * 100) / 100
  return {
    median: r(median),
    p95: r(p95),
    min: r(sorted[0]),
    max: r(sorted[n - 1]),
    mean: r(mean),
    reps: n,
    samples: samples.map(r),
  }
}

function summarizeFrames(dts) {
  const s = summarize(dts)
  const over16 = dts.filter((d) => d > 16.7).length
  const over33 = dts.filter((d) => d > 33.3).length
  const r = (v) => Math.round(v * 100) / 100
  return {
    ...s,
    frames: dts.length,
    pctOver16_7ms: dts.length ? r((over16 / dts.length) * 100) : 0,
    pctOver33_3ms: dts.length ? r((over33 / dts.length) * 100) : 0,
  }
}

async function previewReady(url) {
  try {
    const res = await fetch(url, { signal: AbortSignal.timeout(1000) })
    await res.body?.cancel()
    return res.ok
  } catch {
    return false
  }
}

async function ensurePreview() {
  if (await previewReady(BASE)) return async () => {}
  if (!existsSync(path.join(root, 'dist/index.html'))) {
    throw new Error('Missing dist/index.html. Run VITE_TEST_HOOKS=1 npm run build first.')
  }
  const child = spawn(process.execPath, ['node_modules/vite/bin/vite.js', 'preview', '--host', HOST, '--port', String(PORT), '--strictPort'], {
    cwd: root,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  const closed = once(child, 'close').catch(() => {})
  let output = ''
  child.stdout.on('data', (c) => { output += c })
  child.stderr.on('data', (c) => { output += c })
  const stop = async () => {
    if (child.pid) {
      if (process.platform === 'win32') spawnSync('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore' })
      else child.kill('SIGKILL')
    }
    await Promise.race([closed, delay(2000)])
  }
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    if (await previewReady(BASE)) return stop
    if (child.exitCode !== null) break
    await delay(100)
  }
  await stop()
  throw new Error(`Vite preview failed on ${BASE}:\n${output}`)
}

function fakeTauriInit() {
  let cb = 0
  const fake = {
    files: new Map(),
    writes: [],
    listeners: {},
    clock: Date.now(),
    fsUrl: (p, info = false) => `/__fs?path=${encodeURIComponent(p)}${info ? '&info=1' : ''}`,
  }
  window.__fake = fake
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} }
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
      switch (cmd) {
        case 'read_file': {
          if (fake.files.has(args.path)) return fake.files.get(args.path).buffer.slice(0)
          const res = await fetch(fake.fsUrl(args.path))
          if (!res.ok) throw `No such file: ${args.path}`
          return await res.arrayBuffer()
        }
        case 'path_info': {
          if (fake.files.has(args.path)) return { exists: true, isDir: false, size: fake.files.get(args.path).length, modifiedMs: fake.files.get(args.path).mtime }
          return (await fetch(fake.fsUrl(args.path, true))).json()
        }
        case 'write_file': {
          const p = decodeURIComponent(opts.headers['x-path'])
          const bytes = new Uint8Array(args)
          bytes.mtime = ++fake.clock
          fake.files.set(p, bytes)
          fake.writes.push({ path: p, size: bytes.length, at: Date.now() })
          return null
        }
        case 'plugin:event|listen':
          (fake.listeners[args.event] ??= []).push(args.handler)
          return args.handler
        case 'plugin:event|unlisten':
        case 'plugin:window|destroy':
        case 'set_window_chrome':
        case 'sync_menu':
          return null
        case 'take_pending_opens':
        case 'installed_plugins':
        case 'history_list':
        case 'cloud_folders':
          return []
        case 'history_usage':
          return 0
        case 'ocr_list_packs':
          return { installed: [], totalBytes: 0 }
        case 'release_notes':
        case 'plugin:updater|check':
          throw 'offline'
        default:
          return null
      }
    },
  }
}

function longTaskInit() {
  window.__perfLongTasks = []
  try {
    const obs = new PerformanceObserver((list) => {
      for (const e of list.getEntries()) {
        window.__perfLongTasks.push({ start: e.startTime, duration: e.duration })
      }
    })
    obs.observe({ type: 'longtask', buffered: true })
  } catch {}
}

async function setupContext(browser, { recordNetwork = false, initExtra } = {}) {
  const context = await browser.newContext({ viewport: { width: 1400, height: 900 }, deviceScaleFactor: 2 })
  const page = await context.newPage()
  page.setDefaultTimeout(60_000)
  const cdp = await context.newCDPSession(page)
  await cdp.send('Performance.enable')

  const net = { requests: 0, bytes: 0, urls: [] }
  if (recordNetwork) {
    page.on('response', async (res) => {
      const u = res.url()
      if (!u.startsWith(BASE) || u.includes('/__fs')) return
      net.requests++
      net.urls.push(path.posix.basename(new URL(u).pathname) || 'index.html')
      try {
        const buf = await res.body()
        net.bytes += buf.length
      } catch {}
    })
  }

  await context.route(/\/__fs(?:\?|$)/, async (route) => {
    const u = new URL(route.request().url())
    const filePath = u.searchParams.get('path')
    if (!filePath) return route.fulfill({ status: 400, body: 'missing path' })
    if (u.searchParams.has('info')) {
      const exists = existsSync(filePath)
      const st = exists ? statSync(filePath) : null
      return route.fulfill({ json: { exists, isDir: !!st?.isDirectory(), size: st?.size ?? 0, modifiedMs: st ? Math.round(st.mtimeMs) : 0 } })
    }
    if (!existsSync(filePath)) return route.fulfill({ status: 404, body: 'missing' })
    return route.fulfill({ body: readFileSync(filePath), contentType: 'application/octet-stream' })
  })

  await page.addInitScript(() => localStorage.setItem('workspace.setupDone', '2'))
  await page.addInitScript(fakeTauriInit)
  await page.addInitScript(longTaskInit)
  if (initExtra) await page.addInitScript(initExtra)
  return { context, page, cdp, net }
}

async function getPerfMetrics(page, cdp) {
  await cdp.send('HeapProfiler.collectGarbage').catch(() => {})
  const { metrics } = await cdp.send('Performance.getMetrics')
  const map = Object.fromEntries(metrics.map((m) => [m.name, m.value]))
  const jsMem = await page.evaluate(() => (performance.memory ? {
    usedJSHeapSize: performance.memory.usedJSHeapSize,
    totalJSHeapSize: performance.memory.totalJSHeapSize,
  } : null))
  return {
    jsHeapUsedMB: Math.round(((map.JSHeapUsedSize ?? jsMem?.usedJSHeapSize ?? 0) / (1024 * 1024)) * 100) / 100,
    jsHeapTotalMB: Math.round(((map.JSHeapTotalSize ?? jsMem?.totalJSHeapSize ?? 0) / (1024 * 1024)) * 100) / 100,
    domNodes: map.Nodes ?? null,
    jsEventListeners: map.JSEventListeners ?? null,
  }
}

async function closeAllTabs(page) {
  await page.evaluate(async () => {
    const ws = window.__workspace.useWorkspace.getState()
    // Reset dirty flags only on untouched benchmark cleanup? Actually closeTab prompts if dirty, so clear dirty or discard:
    // Mark clean only in test cleanup if needed so benchmark tabs close deterministically without modal prompts.
    for (const t of [...ws.tabs]) ws.markDirty(t.id, false)
    await ws.closeAll()
  })
  await page.waitForFunction(() => window.__workspace.useWorkspace.getState().tabs.length === 0)
}

function measureAssets() {
  const distAssets = path.join(root, 'dist/assets')
  const files = readdirSync(distAssets).sort()
  const scan = (predicate) => {
    let bytes = 0
    let gzipBytes = 0
    const matched = []
    for (const f of files) {
      if (!predicate(f)) continue
      const buf = readFileSync(path.join(distAssets, f))
      const gz = gzipSync(buf).length
      bytes += buf.length
      gzipBytes += gz
      matched.push({ file: f, bytes: buf.length, gzipBytes: gz })
    }
    return { bytes, gzipBytes, files: matched }
  }
  return {
    totalDistBytes: files.reduce((s, f) => s + statSync(path.join(distAssets, f)).size, 0),
    entryJs: scan((f) => /^index-.*\.js$/.test(f)),
    entryCss: scan((f) => /^index-.*\.css$/.test(f)),
    officeWasm: scan((f) => /^office_wasm_bg.*\.wasm$/.test(f)),
    pdfExportWasm: scan((f) => /^office_pdf_wasm_bg.*\.wasm$/.test(f)),
    pdfiumWasm: scan((f) => /^pdfium.*\.wasm$/.test(f)),
    docxEditor: scan((f) => /^DocxEditor-.*\.js$/.test(f)),
    pptxEditor: scan((f) => /^PptxEditor-.*\.js$/.test(f)),
    sheetEditor: scan((f) => /^SheetEditor-.*\.js$/.test(f)),
    pdfEditor: scan((f) => /^PdfEditor-.*\.js$/.test(f)),
    markdownEditor: scan((f) => /^MarkdownEditor-.*\.js$/.test(f)),
    textEditor: scan((f) => /^TextEditor-.*\.js$/.test(f)),
    latexMath: scan((f) => /^latexMath-.*\.js$/.test(f)),
    libheif: scan((f) => /^libheif-bundle-.*\.js$/.test(f)),
  }
}

function measureRustExamples() {
  const out = {}
  for (const name of ['word-10p.docx', 'word-100p.docx', 'word-500p.docx']) {
    const fx = path.join(fixturesDir, name)
    const openMs = [], layoutMs = [], saveMs = [], coreSaveMs = []
    const editUs = [], keyLayoutUs = [], paintUs = [], overlayUs = []
    let pages = 0
    const parseDur = (s, targetUnit) => {
      const m = /^([\d.]+)(µs|ms|s|ns)$/.exec(s)
      if (!m) return Number.NaN
      const v = Number(m[1])
      const u = m[2]
      const inNs = u === 'ns' ? v : u === 'µs' ? v * 1e3 : u === 'ms' ? v * 1e6 : v * 1e9
      return targetUnit === 'ms' ? inNs / 1e6 : inNs / 1e3
    }
    for (let rep = 0; rep < 5; rep++) {
      const tOut = execFileSync('cargo', ['run', '--release', '--manifest-path', 'crates/Cargo.toml', '-p', 'office-word', '--example', 'timing', '--', fx], { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] })
      const tm = /open (\S+) layout (\S+) save (\S+) \(core only (\S+)\) pages (\d+)/.exec(tOut)
      if (tm) {
        openMs.push(parseDur(tm[1], 'ms'))
        layoutMs.push(parseDur(tm[2], 'ms'))
        saveMs.push(parseDur(tm[3], 'ms'))
        coreSaveMs.push(parseDur(tm[4], 'ms'))
        pages = Number(tm[5])
      }
      const kOut = execFileSync('cargo', ['run', '--release', '--manifest-path', 'crates/Cargo.toml', '-p', 'office-word', '--example', 'typing', '--', fx], { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] })
      const km = /per key: edit (\S+) layout (\S+) paint (\S+) overlay\+state (\S+)/.exec(kOut)
      if (km) {
        editUs.push(parseDur(km[1], 'us'))
        keyLayoutUs.push(parseDur(km[2], 'us'))
        paintUs.push(parseDur(km[3], 'us'))
        overlayUs.push(parseDur(km[4], 'us'))
      }
    }
    out[name] = {
      pages,
      open_ms: summarize(openMs),
      layout_ms: summarize(layoutMs),
      save_ms: summarize(saveMs),
      core_save_ms: summarize(coreSaveMs),
      key_edit_us: summarize(editUs),
      key_layout_us: summarize(keyLayoutUs),
      key_paint_us: summarize(paintUs),
      key_overlay_state_us: summarize(overlayUs),
    }
  }
  return out
}

async function scrollContainer(page, selector, steps = 60) {
  return await page.evaluate(async ({ selector, steps }) => {
    const el = document.querySelector(selector)
    if (!el) throw new Error(`Missing scroll element: ${selector}`)
    const maxScroll = Math.max(100, el.scrollHeight - el.clientHeight)
    const dts = []
    let prev = performance.now()
    for (let i = 1; i <= steps; i++) {
      el.scrollTop = Math.round((maxScroll * i) / steps)
      await new Promise((r) => requestAnimationFrame((now) => {
        dts.push(now - prev)
        prev = now
        r()
      }))
    }
    el.scrollTop = 0
    await new Promise((r) => requestAnimationFrame(r))
    return dts.slice(1) // drop initial setup frame
  }, { selector, steps })
}

async function measureStartup(browser) {
  const coldTti = [], coldNetBytes = [], coldReqs = [], coldLongCount = [], coldLongTotal = []
  const firstDocMs = []
  const warmTti = []
  let startupFiles = []

  for (let i = 0; i < 5; i++) {
    const { context, page, net } = await setupContext(browser, { recordNetwork: true })
    const t0 = performance.now()
    await page.goto(BASE)
    await page.waitForSelector('.app')
    await page.waitForFunction(() => !!window.__workspace)
    coldTti.push(performance.now() - t0)
    // Wait 3.5s to capture idle preload & startup long tasks
    await page.waitForTimeout(3500)
    const lt = await page.evaluate(() => window.__perfLongTasks || [])
    coldNetBytes.push(net.bytes)
    coldReqs.push(net.requests)
    if (i === 0) startupFiles = net.urls
    coldLongCount.push(lt.length)
    coldLongTotal.push(lt.reduce((s, x) => s + x.duration, 0))
    await context.close()
  }

  // Warm reloads in one context
  {
    const { context, page } = await setupContext(browser)
    await page.goto(BASE)
    await page.waitForFunction(() => !!window.__workspace)
    for (let i = 0; i < 5; i++) {
      const t0 = performance.now()
      await page.reload()
      await page.waitForSelector('.app')
      await page.waitForFunction(() => !!window.__workspace)
      warmTti.push(performance.now() - t0)
    }
    await context.close()
  }

  // Cold first document visible & editable (word-10p.docx)
  const word10 = path.join(fixturesDir, 'word-10p.docx')
  for (let i = 0; i < 5; i++) {
    const { context, page } = await setupContext(browser)
    const t0 = performance.now()
    await page.goto(`${BASE}?open=${encodeURIComponent(word10)}`)
    await page.waitForFunction(() => {
      const c = document.querySelector('.view[data-active=true] .docx-page-content')
      return c && c.textContent && c.textContent.trim().length > 20 && !!window.__docxEditor
    })
    firstDocMs.push(performance.now() - t0)
    await context.close()
  }

  return {
    cold_tti_ms: summarize(coldTti),
    warm_tti_ms: summarize(warmTti),
    cold_network_bytes: summarize(coldNetBytes),
    cold_requests: summarize(coldReqs),
    cold_long_tasks_count: summarize(coldLongCount),
    cold_long_tasks_total_ms: summarize(coldLongTotal),
    first_doc_word10p_ms: summarize(firstDocMs),
    startup_files_sample: startupFiles,
  }
}

async function measureWord(browser) {
  const { context, page } = await setupContext(browser)
  await page.goto(BASE)
  await page.waitForFunction(() => !!window.__workspace)
  const results = {}

  for (const name of ['word-10p.docx', 'word-100p.docx', 'word-500p.docx']) {
    const file = path.join(fixturesDir, name)
    const openMs = []
    let pages = 0
    for (let rep = 0; rep < 5; rep++) {
      const t0 = performance.now()
      await page.evaluate((p) => window.__workspace.useWorkspace.getState().openPaths([p]), file)
      await page.waitForFunction(() => {
        const c = document.querySelector('.view[data-active=true] .docx-page-content')
        return c && c.textContent && c.textContent.trim().length > 20 && !!window.__docxEditor
      })
      openMs.push(performance.now() - t0)
      pages = await page.evaluate(() => window.__docxEditor.state.page.total)
      if (rep < 4 || name === 'word-10p.docx') await closeAllTabs(page)
    }

    let typing = null
    let scroll = null
    let saveMs = null

    if (name !== 'word-10p.docx') {
      // Typing 30 chars
      await page.locator('.view[data-active=true] .docx-page-content').first().click()
      await page.waitForTimeout(100)
      const keyDurations = []
      const wallStart = performance.now()
      for (let k = 0; k < 30; k++) {
        await page.evaluate(() => {
          window.__kDone = new Promise((resolve) => {
            window.addEventListener('keydown', () => {
              const t0 = performance.now()
              requestAnimationFrame(() => requestAnimationFrame(() => resolve(performance.now() - t0)))
            }, { once: true, capture: true })
          })
        })
        await page.keyboard.press('KeyA')
        keyDurations.push(await page.evaluate(() => window.__kDone))
      }
      const wall30 = performance.now() - wallStart
      typing = { key_to_paint_ms: summarize(keyDurations), wall_30_keys_ms: Math.round(wall30 * 100) / 100 }

      if (name === 'word-500p.docx') {
        const dts = await scrollContainer(page, '.view[data-active=true] .docx-viewport', 60)
        scroll = summarizeFrames(dts)
      }

      const saves = []
      for (let rep = 0; rep < 5; rep++) {
        const dt = await page.evaluate(async () => {
          const id = window.__workspace.useWorkspace.getState().activeId
          const t0 = performance.now()
          const b = await window.__workspace.getEditor(id).getBytes()
          return { dt: performance.now() - t0, len: b.length }
        })
        saves.push(dt.dt)
      }
      saveMs = summarize(saves)
      await closeAllTabs(page)
    }

    results[name] = { pages, open_ms: summarize(openMs), typing, scroll, save_ms: saveMs }
  }

  await context.close()
  return results
}

async function measureSheets(browser) {
  const { context, page } = await setupContext(browser)
  await page.goto(BASE)
  await page.waitForFunction(() => !!window.__workspace)
  const results = {}

  for (const name of ['sheets-10k.xlsx', 'sheets-50k.xlsx', 'sheets-formulas.xlsx', 'sheets-50k.csv']) {
    const file = path.join(fixturesDir, name)
    const openMs = []
    for (let rep = 0; rep < 5; rep++) {
      const t0 = performance.now()
      await page.evaluate((p) => window.__workspace.useWorkspace.getState().openPaths([p]), file)
      await page.waitForFunction(() => !!window.__sheets && (document.querySelector('.view[data-active=true] .sg-cells')?.childElementCount ?? 0) > 0)
      openMs.push(performance.now() - t0)
      if (rep < 4 || name === 'sheets-10k.xlsx' || name === 'sheets-50k.csv') await closeAllTabs(page)
    }

    let recalc = null
    let scroll = null
    let saveMs = null

    if (name === 'sheets-formulas.xlsx') {
      const recalcs = []
      for (let k = 0; k < 20; k++) {
        const val = String(1 + ((k % 9) + 1) * 0.1)
        const dt = await page.evaluate(async (v) => {
          const t0 = performance.now()
          window.__sheets.run({ cmd: 'input', sheet: 0, row: 0, col: 1, text: v })
          await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))
          return performance.now() - t0
        }, val)
        recalcs.push(dt)
      }
      recalc = summarize(recalcs)
    }

    if (name === 'sheets-50k.xlsx') {
      scroll = summarizeFrames(await scrollContainer(page, '.view[data-active=true] .sg-scroller', 60))
    }

    if (name === 'sheets-50k.xlsx' || name === 'sheets-formulas.xlsx') {
      const saves = []
      for (let rep = 0; rep < 5; rep++) {
        saves.push(await page.evaluate(async () => {
          const id = window.__workspace.useWorkspace.getState().activeId
          const t0 = performance.now()
          await window.__workspace.getEditor(id).getBytes()
          return performance.now() - t0
        }))
      }
      saveMs = summarize(saves)
      await closeAllTabs(page)
    }

    results[name] = { open_ms: summarize(openMs), recalc_ms: recalc, scroll, save_ms: saveMs }
  }

  await context.close()
  return results
}

async function measureSlides(browser) {
  const { context, page } = await setupContext(browser)
  await page.goto(BASE)
  await page.waitForFunction(() => !!window.__workspace)
  const results = {}

  for (const name of ['slides-20.pptx', 'slides-150.pptx']) {
    const file = path.join(fixturesDir, name)
    const openMs = []
    let slideCount = 0
    for (let rep = 0; rep < 5; rep++) {
      const t0 = performance.now()
      await page.evaluate((p) => window.__workspace.useWorkspace.getState().openPaths([p]), file)
      await page.waitForFunction(() => !!window.__slides && (document.querySelector('.view[data-active=true] .slide-surface')?.childElementCount ?? 0) > 0)
      openMs.push(performance.now() - t0)
      slideCount = await page.evaluate(() => window.__slides.slides.length)
      if (rep < 4 || name === 'slides-20.pptx') await closeAllTabs(page)
    }

    let switchMs = null
    let addSlideMs = null
    let saveMs = null
    if (name === 'slides-150.pptx') {
      const switches = []
      for (let k = 0; k < 20; k++) {
        const idx = (k * 17) % slideCount
        switches.push(await page.evaluate(async (i) => {
          const t0 = performance.now()
          window.__slides.run({ cmd: 'setActive', index: i })
          await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))
          return performance.now() - t0
        }, idx))
      }
      switchMs = summarize(switches)

      const adds = []
      for (let k = 0; k < 5; k++) {
        adds.push(await page.evaluate(async () => {
          const t0 = performance.now()
          window.__slides.run({ cmd: 'addSlide', after: 0, layout: null })
          await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))
          return performance.now() - t0
        }))
      }
      addSlideMs = summarize(adds)

      const saves = []
      for (let rep = 0; rep < 5; rep++) {
        saves.push(await page.evaluate(async () => {
          const id = window.__workspace.useWorkspace.getState().activeId
          const t0 = performance.now()
          await window.__workspace.getEditor(id).getBytes()
          return performance.now() - t0
        }))
      }
      saveMs = summarize(saves)
      await closeAllTabs(page)
    }

    results[name] = { slideCount, open_ms: summarize(openMs), switch_ms: switchMs, add_slide_ms: addSlideMs, save_ms: saveMs }
  }

  await context.close()
  return results
}

async function measurePdf(browser) {
  const { context, page } = await setupContext(browser)
  await page.goto(BASE)
  await page.waitForFunction(() => !!window.__workspace)
  const results = {}

  for (const name of ['pdf-100p.pdf', 'pdf-500p.pdf']) {
    const file = path.join(fixturesDir, name)
    const openMs = []
    for (let rep = 0; rep < 5; rep++) {
      const t0 = performance.now()
      await page.evaluate((p) => window.__workspace.useWorkspace.getState().openPaths([p]), file)
      await page.waitForFunction(() => !!document.querySelector('.view[data-active=true] .page[data-loaded="true"] canvas.page-base'), null, { timeout: 60_000 })
      openMs.push(performance.now() - t0)
      if (rep < 4 || name === 'pdf-100p.pdf') await closeAllTabs(page)
    }

    let scroll = null
    let searchMs = null
    if (name === 'pdf-500p.pdf') {
      scroll = summarizeFrames(await scrollContainer(page, '.view[data-active=true] .pdf-container', 60))

      const searches = []
      for (let rep = 0; rep < 5; rep++) {
        await page.keyboard.press('Control+f')
        const input = page.locator('.view[data-active=true] .find-bar input[aria-label="Find"]')
        await input.waitFor()
        await input.fill('')
        const t0 = performance.now()
        await input.fill('NeedlePage490Token')
        await page.locator('.view[data-active=true] .find-bar .count').getByText(/1 of \d+/).waitFor({ timeout: 60_000 })
        searches.push(performance.now() - t0)
        await page.keyboard.press('Escape')
      }
      searchMs = summarize(searches)
      await closeAllTabs(page)
    }

    results[name] = { open_ms: summarize(openMs), scroll, search_page490_ms: searchMs }
  }

  await context.close()
  return results
}

async function measureExportAndOcr(browser) {
  // Start local asset server for tessdata packs
  const server = createHttpServer((req, res) => {
    const pathname = new URL(req.url, 'http://127.0.0.1/').pathname
    const m = /^\/packs\/([a-z]+)\.traineddata$/.exec(pathname)
    res.setHeader('Access-Control-Allow-Origin', '*')
    if (!m) { res.writeHead(404).end(); return }
    const fp = path.join(tessdataDir, `${m[1]}.traineddata`)
    if (!existsSync(fp)) { res.writeHead(404).end(); return }
    res.writeHead(200, { 'Content-Type': 'application/octet-stream' })
    res.end(readFileSync(fp))
  })
  await new Promise((r) => server.listen(0, '127.0.0.1', r))
  const port = server.address().port
  const packUrl = `http://127.0.0.1:${port}/packs`

  const { context, page } = await setupContext(browser, {
    initExtra: () => {
      // Filled below via evaluate or localStorage? Notice ocrApi loads dynamically.
    },
  })
  try {
    await page.goto(BASE)
    await page.waitForFunction(() => !!window.__loadPdfOcrTestApi)
    await page.evaluate(async (baseUrl) => {
      window.ocrApi = await window.__loadPdfOcrTestApi()
      window.ocrApi.packs.setPackBaseUrl?.(baseUrl)
      // Wait: check how offline-ocr.mjs overrides pack URL
    }, packUrl)

    // Vector export 5 reps on tour fixtures
    const exports = {}
    for (const [fxPath, method] of [
      ['scripts/tour/Library Update.docx', 'exportWordToPdf'],
      ['scripts/tour/Spring Launch.pptx', 'exportSlidesToPdf'],
      ['crates/office-sheets/tests/poi-simple-monthly-budget.xlsx', 'exportSheetsToPdf'],
    ]) {
      const bytes = readFileSync(path.join(root, fxPath))
      const b64 = bytes.toString('base64')
      const samples = []
      for (let rep = 0; rep < 5; rep++) {
        const dt = await page.evaluate(async ({ b64, method, name }) => {
          const raw = Uint8Array.fromBase64(b64)
          const t0 = performance.now()
          const out = await window.ocrApi.exporter[method](raw, name)
          if (!out || out.length < 100) throw new Error('Empty PDF export')
          return performance.now() - t0
        }, { b64, method, name: path.basename(fxPath) })
        samples.push(dt)
      }
      exports[path.basename(fxPath)] = summarize(samples)
    }

    // OCR benchmark via existing scripts/benchmark-pdf-ocr.mjs (which tests eng, heb, ara)
    return { exports }
  } finally {
    await context.close()
    server.close()
  }
}

async function measureMarkdownAndText(browser) {
  const { context, page } = await setupContext(browser)
  await page.goto(BASE)
  await page.waitForFunction(() => !!window.__workspace)

  const measureEditor = async (filename, openSelector, isMarkdown = false) => {
    const file = path.join(fixturesDir, filename)
    const openMs = []
    for (let rep = 0; rep < 5; rep++) {
      const t0 = performance.now()
      await page.evaluate((p) => window.__workspace.useWorkspace.getState().openPaths([p]), file)
      await page.locator(openSelector).waitFor()
      openMs.push(performance.now() - t0)
      if (rep < 4) await closeAllTabs(page)
    }
    if (isMarkdown) {
      await page.evaluate(() => {
        const id = window.__workspace.useWorkspace.getState().activeId
        window.__workspace.getEditor(id)?.viewMode?.('split')
      })
      await page.locator('.view[data-active=true] .cm-content').waitFor()
    }
    await page.locator('.view[data-active=true] .cm-content').click()
    const keys = []
    for (let k = 0; k < 20; k++) {
      await page.evaluate(() => {
        window.__kDone = new Promise((resolve) => {
          window.addEventListener('keydown', () => {
            const t0 = performance.now()
            requestAnimationFrame(() => requestAnimationFrame(() => resolve(performance.now() - t0)))
          }, { once: true, capture: true })
        })
      })
      await page.keyboard.press('KeyX')
      keys.push(await page.evaluate(() => window.__kDone))
    }
    await closeAllTabs(page)
    return { open_ms: summarize(openMs), typing_ms: summarize(keys) }
  }

  const md = await measureEditor('doc-100k.md', '.view[data-active=true] .md-body', true)
  const txt = await measureEditor('doc-2mb.txt', '.view[data-active=true] .cm-content', false)
  await context.close()
  return { 'doc-100k.md': md, 'doc-2mb.txt': txt }
}

async function measureMultiTabAndMemory(browser) {
  const { context, page, cdp } = await setupContext(browser)
  await page.goto(BASE)
  await page.waitForFunction(() => !!window.__workspace)

  const tabFiles = [
    'word-100p.docx',
    'slides-150.pptx',
    'sheets-50k.xlsx',
    'pdf-100p.pdf',
    'doc-100k.md',
    'doc-2mb.txt',
    'word-10p.docx',
    'slides-20.pptx',
    'sheets-10k.xlsx',
    'sheets-50k.csv',
  ].map((f) => path.join(fixturesDir, f))

  const beforeOpen = await getPerfMetrics(page, cdp)
  await page.evaluate((paths) => window.__workspace.useWorkspace.getState().openPaths(paths), tabFiles)
  const tabIds = await page.evaluate(() => window.__workspace.useWorkspace.getState().tabs.map((t) => ({ id: t.id, kind: t.kind })))

  // Activate every tab once so all 10 editors mount
  for (const t of tabIds) {
    await page.evaluate((id) => window.__workspace.useWorkspace.getState().activate(id), t.id)
    await page.waitForFunction((id) => !!window.__workspace.getEditor(id), t.id)
    await page.waitForTimeout(100)
  }
  const mounted10 = await getPerfMetrics(page, cdp)

  // Measure 20 tab switches across the 10 mounted tabs
  const switches = []
  for (let k = 0; k < 20; k++) {
    const target = tabIds[(k * 3) % tabIds.length].id
    const dt = await page.evaluate(async (id) => {
      const t0 = performance.now()
      window.__workspace.useWorkspace.getState().activate(id)
      await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))
      return performance.now() - t0
    }, target)
    switches.push(dt)
  }

  // Idle long tasks over 3s
  await page.evaluate(() => { window.__perfLongTasks = [] })
  await page.waitForTimeout(3000)
  const idleLongTasks = await page.evaluate(() => window.__perfLongTasks || [])

  await closeAllTabs(page)
  await page.waitForTimeout(200)
  const afterClose10 = await getPerfMetrics(page, cdp)

  // 5-cycle open/close leak check
  const cycleMetrics = []
  const cycleFiles = ['word-100p.docx', 'pdf-100p.pdf', 'sheets-10k.xlsx'].map((f) => path.join(fixturesDir, f))
  for (let c = 0; c < 5; c++) {
    await page.evaluate((paths) => window.__workspace.useWorkspace.getState().openPaths(paths), cycleFiles)
    const ids = await page.evaluate(() => window.__workspace.useWorkspace.getState().tabs.map((t) => t.id))
    for (const id of ids) {
      await page.evaluate((i) => window.__workspace.useWorkspace.getState().activate(i), id)
      await page.waitForFunction((i) => !!window.__workspace.getEditor(i), id)
    }
    await closeAllTabs(page)
    await page.waitForTimeout(150)
    cycleMetrics.push({ cycle: c + 1, ...(await getPerfMetrics(page, cdp)) })
  }

  await context.close()
  return {
    beforeOpen,
    mounted10Tabs: mounted10,
    afterClose10Tabs: afterClose10,
    tabSwitch_ms: summarize(switches),
    idle3sLongTasks: { count: idleLongTasks.length, totalMs: Math.round(idleLongTasks.reduce((s, x) => s + x.duration, 0) * 100) / 100 },
    openClose5Cycles: cycleMetrics,
  }
}

const stopPreview = await ensurePreview()
let browser
try {
  browser = await chromium.launch({
    args: ['--no-sandbox', '--enable-precise-memory-info', '--js-flags=--expose-gc'],
  })

  const report = {
    meta: {
      label,
      commit: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
      timestamp: new Date().toISOString(),
      platform: `${os.platform()} ${os.release()} (${os.arch()})`,
      cpu: os.cpus()[0]?.model ?? 'unknown',
      cores: os.cpus().length,
      ramGB: Math.round((os.totalmem() / (1024 ** 3)) * 10) / 10,
      node: process.version,
      chromium: browser.version(),
      viewport: '1400x900@2x',
    },
    assets: measureAssets(),
  }

  if (!skipRust && want('rust')) {
    console.log('Running Rust native office-word benchmarks...')
    report.rustWord = measureRustExamples()
  }
  if (want('startup')) {
    console.log('Running startup & first-doc benchmarks...')
    report.startup = await measureStartup(browser)
  }
  if (want('word')) {
    console.log('Running Word (DOCX) benchmarks...')
    report.word = await measureWord(browser)
  }
  if (want('sheets')) {
    console.log('Running Excel/CSV benchmarks...')
    report.sheets = await measureSheets(browser)
  }
  if (want('slides')) {
    console.log('Running PowerPoint (PPTX) benchmarks...')
    report.slides = await measureSlides(browser)
  }
  if (want('pdf')) {
    console.log('Running PDF benchmarks...')
    report.pdf = await measurePdf(browser)
  }
  if (want('export')) {
    console.log('Running Vector PDF Export benchmarks...')
    report.pdfExport = await measureExportAndOcr(browser)
  }
  if (want('text')) {
    console.log('Running Markdown & Text benchmarks...')
    report.markdownAndText = await measureMarkdownAndText(browser)
  }
  if (want('multitab')) {
    console.log('Running Multi-tab, Tab-switch & Memory recovery benchmarks...')
    report.multiTab = await measureMultiTabAndMemory(browser)
  }

  mkdirSync(path.dirname(outFile), { recursive: true })
  writeFileSync(outFile, JSON.stringify(report, null, 2))
  console.log(`Saved benchmark report to ${outFile}`)
} finally {
  await browser?.close().catch(() => {})
  await stopPreview()
}
