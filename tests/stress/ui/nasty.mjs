// Damaged, empty, renamed and encrypted files: each shows a clear message in its tab, and the app
// (and the tabs already open) keep working.  node tests/stress/ui/nasty.mjs [only-this-substring]
import { readdirSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { launch, start, open, tabs, check, done, TMP } from './harness.mjs'

const S = resolve(import.meta.dirname, '../samples')
const only = process.argv[2]
const files = readdirSync(S).filter((f) => /^S\d-(random|empty|truncated|encrypted|pdf-renamed|docx-renamed|pdf-no-pages|password|markdown-percent|text-binary)/.test(f) && (!only || f.includes(only)))
const { browser, page, logs } = await launch()
await start(page)

// A healthy document first, to check it survives everything else.
await open(page, join(S, 'S3-text-mixed-eol.txt'))
await page.waitForSelector('.view[data-active=true] .cm-content')
const keeper = await page.evaluate(() => window.__workspace.useWorkspace.getState().activeId)

for (const f of files) {
  const before = logs.length
  await open(page, join(S, f))
  // Up to 15 s for a message, an editor, or the app's error dialog.
  let state = 'timeout'
  for (let i = 0; i < 75; i++) {
    await page.waitForTimeout(200)
    state = await page.evaluate(() => {
      if (document.querySelector('.dialog')) return 'dialog: ' + document.querySelector('.dialog').textContent
      const v = document.querySelector('.view[data-active=true]')
      if (!v) return 'no view'
      const msg = v.querySelector('.center-state')
      if (msg && !msg.querySelector('.spin')) return 'message: ' + msg.textContent
      if (v.querySelector('.workbench .cm-content, .workbench .wp-page, .workbench .slide-surface, .workbench .pdf-page, .workbench .sheet-grid, .workbench canvas')) return 'editor'
      if (v.querySelector('.workbench')) return 'loading'
      const text = v.textContent.trim()
      if (text && !v.querySelector('.spin')) return 'message: ' + text
      return 'blank'
    })
    if (state.startsWith('message:') || state.startsWith('dialog:') || state === 'editor') break
  }
  await page.screenshot({ path: join(TMP, `nasty-${f}.png`) })
  const errs = logs.slice(before).filter((l) => l.startsWith('[pageerror]'))
  console.log(`      ${f}: ${state.slice(0, 160).replace(/\s+/g, ' ')}`)
  check(`${f}: a message or an editor, not blank`, state.startsWith('message') || state.startsWith('dialog') || state.endsWith('editor'), state.slice(0, 80))
  if (/\.(docx|pptx|xlsx|png)$/.test(f)) {
    check(`${f}: damaged Office/image is refused, never an empty writable editor`, state.startsWith('message:') || state.startsWith('dialog:'), state.slice(0, 180))
    if (f.includes('password.')) check(`${f}: password is explained`, /password/i.test(state), state)
    if (/docx-renamed\.(pptx|xlsx)$/.test(f)) check(`${f}: correct file type is explained`, /Word document.*not.*(PowerPoint|Excel)/i.test(state), state)
  }
  if (f === 'S3-empty.tsv') check('empty TSV opens as an editable sheet', state === 'editor', state)
  if (f.includes('pdf-no-pages')) check(`${f}: zero pages are explained`, /no pages/i.test(state), state)
  check(`${f}: no unhandled errors`, errs.length === 0, errs.join(' | ').slice(0, 300))
  if (state.startsWith('dialog')) await page.locator('.dialog button').last().click()
  // Close the tab again; the app must respond.
  const t = await tabs(page)
  const last = t.find((x) => x.path?.endsWith(f))
  if (last) {
    check(`${f}: opening never writes over the sample`, await page.evaluate((p) => !window.__fake.writes.some((w) => w.path === p), last.path))
  }
  if (last) await page.evaluate((id) => window.__workspace.useWorkspace.getState().closeTab(id), last.id)
}

await page.evaluate((id) => window.__workspace.useWorkspace.getState().activate(id), keeper)
await page.waitForTimeout(300)
await page.click('.view[data-active=true] .cm-content')
await page.keyboard.type('still alive')
const text = await page.locator('.view[data-active=true] .cm-content').textContent()
check('the first tab still edits', text.includes('still alive'))
await page.screenshot({ path: join(TMP, `nasty-final-${process.env.ENGINE ?? 'chromium'}.png`) })
await done(browser, logs)
