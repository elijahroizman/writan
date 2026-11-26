// Scripted performance measurements for the fixed corpus. The runner (scripts/perf.mjs) owns
// the preview server; this file is written so its JSON can also feed a comparison harness.
import { fileURLToPath } from 'node:url'
import { launch, open } from '../ui/harness.mjs'

const root = fileURLToPath(new URL('.', import.meta.url))
const type = process.argv.find((a) => a.startsWith('--type='))?.slice(7) ?? 'docx'
const warm = process.argv.includes('--warm')
const target = process.argv.slice(2).find((a) => !a.startsWith('--'))
const failures = []
const budget = (name, value, env) => {
  const limit = Number(process.env[env])
  if (limit && value > limit) failures.push(`${name} ${Math.round(value)}ms over ${limit}ms`)
}

const { browser, page, logs } = await launch({ tauri: true, init: () => localStorage.setItem('workspace.engineTiming', '1') })
const infos = []
page.on('console', (m) => {
  if (m.type() === 'info' && m.text().startsWith('[engine]')) infos.push(m.text())
})

try {
  const t0 = performance.now()
  await page.goto(process.env.WRITAN_URL ?? 'http://localhost:4173/', { waitUntil: 'domcontentloaded' })
  await page.waitForSelector('.app', { timeout: 15000 })
  const appMs = performance.now() - t0
  await page.waitForFunction(() => window.__workspace, null, { timeout: 15000 })

  if (type === 'startup') {
    await page.waitForTimeout(2600)
    const resources = await page.evaluate(() => performance.getEntriesByType('resource').map((r) => ({ encoded: r.encodedBodySize, transfer: r.transferSize })))
    const out = {
      type,
      appMs,
      resources: resources.length,
      encodedBytes: resources.reduce((a, r) => a + r.encoded, 0),
      transferBytes: resources.reduce((a, r) => a + r.transfer, 0),
      errors: logs,
    }
    const byteBudget = Number(process.env.PERF_BUDGET_STARTUP_BYTES)
    if (byteBudget && out.encodedBytes > byteBudget) failures.push(`startup encodedBytes ${out.encodedBytes} over ${byteBudget}`)
    console.log(JSON.stringify({ ...out, failures }, null, 2))
    if (failures.length) process.exitCode = 1
  } else if (type === 'xlsx') {
    const file = target ?? `${root}generated/large-sheet.xlsx`
    if (warm) await page.waitForTimeout(3000)
    const tOpen = performance.now()
    await open(page, file)
    await page.waitForFunction(() => window.__sheets, null, { timeout: 30000 })
    const readyMs = performance.now() - tOpen
    await page.waitForSelector('.sg-cells > *', { timeout: 30000 })
    const firstPaintMs = performance.now() - tOpen
    const stats = await page.evaluate(() => {
      const e = window.__sheets
      const st = e.state
      const g = e.geometryOf(st.sheet)
      const runs = []
      for (let i = 0; i < 30; i++) {
        const t = performance.now()
        e.run({ cmd: 'input', sheet: st.sheet, row: st.focus[0], col: st.focus[1], text: '1' })
        runs.push(performance.now() - t)
      }
      runs.sort((a, b) => a - b)
      return {
        rows: g.info.rows,
        cols: g.info.cols,
        medianKeyMs: runs[Math.floor(runs.length / 2)],
        p90KeyMs: runs[Math.floor(runs.length * 0.9)],
        heapMb: performance.memory ? Math.round(performance.memory.usedJSHeapSize / 1048576) : null,
      }
    })
    budget('ready', readyMs, 'PERF_BUDGET_OPEN_MS')
    budget('first paint', firstPaintMs, 'PERF_BUDGET_PAINT_MS')
    budget('median key', stats.medianKeyMs, 'PERF_BUDGET_KEY_MS')
    console.log(JSON.stringify({ type, file, warm, appMs, readyMs, firstPaintMs, ...stats, errors: logs, failures }, null, 2))
    if (failures.length) process.exitCode = 1
  } else {
    const file = target ?? `${root}generated/large-document.docx`
    if (warm) await page.waitForTimeout(3000)
    const tOpen = performance.now()
    await open(page, file)
    await page.waitForFunction(() => window.__docxEditor, null, { timeout: 30000 })
    const readyMs = performance.now() - tOpen
    await page.waitForSelector('.docx-page-content span', { timeout: 30000 })
    const firstPaintMs = performance.now() - tOpen
    const stats = await page.evaluate(async () => {
      const e = window.__docxEditor
      const runs = []
      for (let i = 0; i < 30; i++) {
        const t = performance.now()
        e.run({ cmd: 'insertText', text: 'x' })
        runs.push(performance.now() - t)
      }
      runs.sort((a, b) => a - b)
      const scroller = document.querySelector('.view[data-active="true"] .canvas')?.parentElement
      let scrollMs = null
      if (scroller) {
        const t = performance.now()
        scroller.scrollTop = scroller.scrollHeight * 0.5
        await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))
        scrollMs = performance.now() - t
      }
      return {
        pages: e.state.page.total,
        medianKeyMs: runs[Math.floor(runs.length / 2)],
        p90KeyMs: runs[Math.floor(runs.length * 0.9)],
        scrollMs,
        heapMb: performance.memory ? Math.round(performance.memory.usedJSHeapSize / 1048576) : null,
        dom: {
          pageFrames: document.querySelectorAll('.docx-page').length,
          focusVeils: document.querySelectorAll('.wp-focus').length,
        },
      }
    })
    budget('ready', readyMs, 'PERF_BUDGET_OPEN_MS')
    budget('first paint', firstPaintMs, 'PERF_BUDGET_PAINT_MS')
    budget('median key', stats.medianKeyMs, 'PERF_BUDGET_KEY_MS')
    console.log(JSON.stringify({ type, file, warm, appMs, readyMs, firstPaintMs, ...stats, engineLog: infos.slice(-8), errors: logs, failures }, null, 2))
    if (failures.length) process.exitCode = 1
  }
} finally {
  await browser.close()
}
