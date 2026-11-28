// Run against a VITE_TEST_HOOKS build. Use the signed GPO PDF from corpus/manifest.tsv
// as /dev-samples/us-budget-2025-appendix.pdf to verify signature-preserving saves.
// node tests/stress/native/pdf-unchanged.mjs URL [chromium|webkit] [sample-path]
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import * as pw from 'playwright'

const base = process.argv[2] ?? 'http://localhost:4173/'
const kind = process.argv[3] ?? 'chromium'
const sample = process.argv[4] ?? '/dev-samples/us-budget-2025-appendix.pdf'
const browser = await pw[kind].launch()
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, acceptDownloads: true })
  page.setDefaultTimeout(30000)
  const errors = []
  page.on('pageerror', e => errors.push(e.message))
  await page.addInitScript(() => {
    localStorage.setItem('workspace.setupDone', '2')
    localStorage.setItem('workspace.settings', JSON.stringify({ version: 2, autosave: { enabled: false, delay: 1, kinds: {} } }))
  })
  await page.goto(new URL(`?open=${encodeURIComponent(sample)}`, base).href)
  await page.waitForFunction(() => {
    const s = window.__workspace?.useWorkspace.getState()
    if (!s || s.tabs[0]?.kind !== 'pdf') return false
    if (!s.rightOpen.pdf) s.toggleRight('pdf')
    return !!window.__workspace.getEditor(s.activeId)
  })
  await page.getByRole('button', { name: 'Rotate right', exact: true }).waitFor()
  await page.waitForFunction(() => [...document.querySelectorAll('button')].some(b => b.textContent.includes('Rotate right') && !b.disabled))
  const original = await (await page.request.get(new URL(sample, base).href)).body()
  const hash = bytes => createHash('sha256').update(bytes).digest('hex')
  const save = async suffix => {
    const download = page.waitForEvent('download')
    assert.equal(await page.evaluate(async () => {
      const s = window.__workspace.useWorkspace.getState()
      return s.save(s.activeId, { to: '/saved/regression.pdf', quiet: true })
    }), true)
    const file = await download
    const path = `/tmp/writan-pdf-${kind}-${suffix}.pdf`
    await file.saveAs(path)
    return hash(await readFile(path))
  }
  assert.equal(await save('unchanged'), hash(original), 'unchanged save rewrote PDF bytes')
  await page.getByRole('button', { name: 'Rotate right', exact: true }).click()
  await page.waitForFunction(() => window.__workspace.useWorkspace.getState().tabs[0].dirty)
  await page.keyboard.press('Control+z')
  await page.waitForFunction(() => !window.__workspace.useWorkspace.getState().tabs[0].dirty)
  assert.equal(await save('undone'), hash(original), 'undo back to original rewrote PDF bytes')
  await page.getByRole('button', { name: 'Rotate right', exact: true }).click()
  await page.waitForFunction(() => window.__workspace.useWorkspace.getState().tabs[0].dirty)
  const edited = await save('edited')
  assert.notEqual(edited, hash(original), 'page rotation was not saved')
  assert.equal(await save('repeated'), edited, 'repeated clean save rewrote the edited PDF')
  await page.screenshot({ path: `/tmp/writan-pdf-${kind}-edited.png` })
  assert.deepEqual(errors, [])
  console.log(`pdf-unchanged: ${kind} passed unchanged, undo, edited, and repeated saves`)
} finally {
  await browser.close()
}
