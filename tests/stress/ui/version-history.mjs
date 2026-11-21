// Version history: each version shown as the document itself (text, a workbook's sheets, Word
// pages, slides, a PDF), scrubbing the timeline, naming, restoring, deleting and saving a
// version. The fake bridge keeps versions in memory the way src-tauri/src/history.rs does:
// write_file with x-history adds one, newest first.
import assert from 'node:assert/strict'
import { mkdirSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import JSZip from 'jszip'
import { launch, start, open, done } from './harness.mjs'

const dir = '/tmp/writan-stress/history'
mkdirSync(dir, { recursive: true })
const NOTES = `${dir}/Notes.txt`
const FRESH = `${dir}/Fresh.md`
const tour = (name) => fileURLToPath(new URL(`../../../scripts/tour/${name}`, import.meta.url))
const PDF = tour('Lease.pdf')
const DOCX = tour('Library Update.docx')
const PPTX = tour('Spring Launch.pptx')
// Oldest first. The file on disk is the newest.
const drafts = ['Plan\n\nWe open in May.\n', 'Plan\n\nWe open in June.\n', 'Plan\n\nWe open in June.\n\nBring snacks.\n']
writeFileSync(NOTES, drafts.at(-1))
writeFileSync(FRESH, 'Nothing saved yet.\n')
const README = `${dir}/Readme.md`
writeFileSync(README, '# Opening day\n\nDoors open at **nine**. See [the map](https://example.com/map) and [the café](#café).\n\n' + 'More.\n\n'.repeat(200) + '## Café\n\nUpstairs.\n')
// Windows-1252, as older Windows files are.
const OLD = `${dir}/Old.txt`
writeFileSync(OLD, Buffer.from('Caf\xe9 menu\n', 'latin1'))
const LETTER = `${dir}/Letter.pdf`

/** A one-page PDF that says `text`. */
function pdf(text) {
  const stream = `BT /F1 24 Tf 40 100 Td (${text}) Tj ET`
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>',
    `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ]
  let out = '%PDF-1.4\n'
  const offsets = objects.map((o, i) => {
    const at = out.length
    out += `${i + 1} 0 obj\n${o}\nendobj\n`
    return at
  })
  const xref = out.length
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((o) => `${String(o).padStart(10, '0')} 00000 n \n`).join('')}`
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`
  return Buffer.from(out, 'latin1')
}
const letters = [pdf('Version one'), pdf('Version two')]
writeFileSync(LETTER, letters.at(-1))

const BUDGET = `${dir}/Budget.xlsx`

/** A workbook of `sheets` ({ name: rows }). Numbers stay numbers; { date } is a date serial shown as a date. */
async function workbook(sheets) {
  const zip = new JSZip()
  const names = Object.keys(sheets)
  const col = (c) => String.fromCharCode(65 + c)
  const cell = (v, r, c) => {
    const ref = `${col(c)}${r + 1}`
    if (typeof v === 'number') return `<c r="${ref}"><v>${v}</v></c>`
    if (v && typeof v === 'object') return `<c r="${ref}" s="1"><v>${v.date}</v></c>`
    return `<c r="${ref}" t="inlineStr"><is><t>${v}</t></is></c>`
  }
  zip.file('[Content_Types].xml', `<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>${names.map((_, i) => `<Override PartName="/xl/worksheets/sheet${i + 1}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>`).join('')}</Types>`)
  zip.file('_rels/.rels', `<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>`)
  zip.file('xl/workbook.xml', `<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>${names.map((n, i) => `<sheet name="${n}" sheetId="${i + 1}" r:id="rId${i + 1}"/>`).join('')}</sheets></workbook>`)
  zip.file('xl/_rels/workbook.xml.rels', `<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">${names.map((_, i) => `<Relationship Id="rId${i + 1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet${i + 1}.xml"/>`).join('')}<Relationship Id="rId${names.length + 1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>`)
  zip.file('xl/styles.xml', `<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs></styleSheet>`)
  names.forEach((n, i) => {
    const rows = sheets[n].map((row, r) => `<row r="${r + 1}">${row.map((v, c) => (v === null ? '' : cell(v, r, c))).join('')}</row>`).join('')
    zip.file(`xl/worksheets/sheet${i + 1}.xml`, `<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="1" max="3" width="16" customWidth="1"/></cols><sheetData>${rows}</sheetData></worksheet>`)
  })
  return zip.generateAsync({ type: 'nodebuffer' })
}

// Oldest first: then Food went up, and Travel was added.
const budgets = [
  await workbook({ Budget: [['Item', 'Cost', 'Paid'], ['Rent', 1200, { date: 45962 }], ['Food', 300]], Notes: [['Check in May']] }),
  await workbook({ Budget: [['Item', 'Cost', 'Paid'], ['Rent', 1200, { date: 45962 }], ['Food', 350], ['Travel', 500]], Notes: [['Check in May']] }),
]
writeFileSync(BUDGET, budgets.at(-1))

const seed = {
  [NOTES]: drafts.map((text, i) => ({ text, label: i === 1 ? 'Moved to June' : undefined })),
  [PDF]: [{}],
  [DOCX]: [{}],
  [README]: [{}],
  [OLD]: [{}],
  [LETTER]: letters.map((b) => ({ b64: b.toString('base64') })),
  [PPTX]: [{}],
  [BUDGET]: budgets.map((b) => ({ b64: b.toString('base64') })),
}

/** Runs in the page after the fake bridge: version history on top of it, seeded with `seed`. */
function fakeHistory(seed) {
  const enc = new TextEncoder()
  const store = new Map()
  const now = Date.now()
  for (const [path, list] of Object.entries(seed)) {
    store.set(
      path,
      list.map((v, i) => ({ version: { id: `${path}#${i}`, hash: String(i), time: now - (list.length - i) * 3_600_000, size: 1, source: i ? 'save' : 'original', label: v.label }, bytes: v.text ? enc.encode(v.text) : v.b64 ? Uint8Array.from(atob(v.b64), (ch) => ch.charCodeAt(0)) : null })).reverse(),
    )
  }
  window.__history = { store, labels: [] }
  const inner = window.__TAURI_INTERNALS__.invoke
  window.__TAURI_INTERNALS__.invoke = async (cmd, args, opts) => {
    if (cmd === 'write_file' && opts?.headers?.['x-history']) {
      const path = decodeURIComponent(opts.headers['x-path'])
      const list = store.get(path) ?? []
      const label = opts.headers['x-history-label'] && decodeURIComponent(opts.headers['x-history-label'])
      list.unshift({ version: { id: `${path}#${Date.now()}-${list.length}`, hash: '', time: Date.now(), size: args.length, source: 'save', label }, bytes: new Uint8Array(args) })
      store.set(path, list)
    }
    const list = store.get(args?.path) ?? []
    const hit = () => list.find((v) => v.version.id === args.id)
    switch (cmd) {
      case 'history_list':
        return list.map((v) => ({ ...v.version }))
      case 'history_read':
        if (hit().bytes) return hit().bytes.slice().buffer
        return (await fetch('/__fs' + encodeURI(args.path))).arrayBuffer()
      case 'history_label':
        window.__history.labels.push(args.label)
        hit().version.label = args.label || undefined
        return null
      case 'history_delete':
        store.set(args.path, list.filter((v) => v.version.id !== args.id))
        return null
    }
    return inner(cmd, args, opts)
  }
}

const { browser, page, logs } = await launch({ init: `(${fakeHistory})(${JSON.stringify(seed)})` })

const dialog = page.getByRole('dialog', { name: /^Version history of/ })
const slider = dialog.getByRole('slider')
const title = dialog.locator('.vh-head h2')
const pageText = () => dialog.locator('.vh-page').textContent()
/** The version on screen once it's drawn: the canvas stops saying it's loading. */
const settled = () => page.waitForFunction(() => { const c = document.querySelector('.vh-canvas'); return c && !c.hasAttribute('data-loading') })
const versionCount = (path) => page.evaluate((path) => window.__history.store.get(path)?.length ?? 0, path)
const cellsText = () => dialog.locator('.sg-cells').innerText()

async function openHistory(name) {
  await page.locator('.tab', { hasText: name }).first().click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Version history…' }).click()
  await dialog.waitFor()
}

async function shot(name) {
  if (process.env.SHOTS) await page.screenshot({ path: `${process.env.SHOTS}/${name}.png` })
}

try {
  await start(page)
  await open(page, NOTES)
  await page.locator('.tab', { hasText: 'Notes' }).waitFor()
  await openHistory('Notes')

  // It opens on the latest version, shown as the file is: no comparison, nothing marked.
  await settled()
  assert.equal(await pageText(), drafts[2])
  assert.equal(await dialog.locator('ins, del').count(), 0)
  assert.equal(await dialog.getByRole('radiogroup').count(), 0)
  assert.equal(await slider.getAttribute('aria-valuenow'), '3')
  assert.equal(await slider.getAttribute('aria-valuemax'), '3')
  assert.match(await dialog.locator('.vh-sub').innerText(), /Latest/)
  assert.equal(await dialog.getByRole('button', { name: 'Restore' }).isDisabled(), true)

  // ← steps back one version: the named one, from before the snacks.
  await page.keyboard.press('ArrowLeft')
  await page.waitForFunction(() => !document.querySelector('.vh-page')?.textContent.includes('snacks'))
  assert.equal(await slider.getAttribute('aria-valuenow'), '2')
  assert.equal(await title.innerText(), 'Moved to June')
  assert.equal(await pageText(), drafts[1])

  // Clicking the first dot jumps to the oldest version.
  await dialog.locator('.vh-tick').first().click()
  await page.waitForFunction(() => /May/.test(document.querySelector('.vh-page')?.textContent ?? ''))
  assert.equal(await slider.getAttribute('aria-valuenow'), '1')
  assert.match(await dialog.locator('.vh-sub').innerText(), /Before Writan first saved it · Oldest kept/)

  // Dragging along the track scrubs to the newest.
  const box = await slider.boundingBox()
  await page.mouse.move(box.x + 4, box.y + 10)
  await page.mouse.down()
  await page.mouse.move(box.x + box.width / 2, box.y + 10, { steps: 5 })
  assert.equal(await slider.getAttribute('aria-valuenow'), '2')
  await page.mouse.move(box.x + box.width - 2, box.y + 10, { steps: 5 })
  await page.mouse.up()
  assert.equal(await slider.getAttribute('aria-valuenow'), '3')
  await page.waitForFunction(() => /snacks/.test(document.querySelector('.vh-page')?.textContent ?? ''))

  // Naming: click the title, type, Enter.
  await page.keyboard.press('Home')
  await settled()
  await title.click()
  const field = dialog.getByPlaceholder(/Name this version/)
  await field.fill('First draft')
  await field.press('Enter')
  await page.waitForFunction(() => document.querySelector('.vh-head h2')?.textContent === 'First draft')
  assert.deepEqual(await page.evaluate(() => window.__history.labels), ['First draft'])

  // Restore asks first. While it asks, the arrow keys don't move the timeline.
  await dialog.getByRole('button', { name: 'Restore' }).click()
  const ask = page.getByRole('dialog', { name: /Restore this version/ })
  await ask.waitFor()
  await page.keyboard.press('ArrowRight')
  assert.equal(await slider.getAttribute('aria-valuenow'), '1')
  await ask.getByRole('button', { name: 'Restore' }).click()
  // The restored copy is the newest version, and the timeline moves to it.
  await page.waitForFunction(() => document.querySelector('[role=slider]')?.getAttribute('aria-valuemax') === '4')
  await page.waitForFunction(() => document.querySelector('[role=slider]')?.getAttribute('aria-valuenow') === '4')
  assert.match(await dialog.locator('.vh-head h2').innerText(), /^Restored from .+ at .+$/u)
  const saved = await page.evaluate((p) => new TextDecoder().decode(window.__fake.files.get(p)), NOTES)
  assert.equal(saved, drafts[0])

  // Delete from the ⋯ menu.
  await dialog.getByRole('button', { name: 'More' }).click()
  await page.getByRole('menuitem', { name: 'Delete version' }).click()
  await page.getByRole('alertdialog', { name: 'Delete this version?' }).getByRole('button', { name: 'Delete' }).click()
  await page.waitForFunction(() => document.querySelector('[role=slider]')?.getAttribute('aria-valuemax') === '3')
  assert.equal(await versionCount(NOTES), 3)

  // Escape closes it.
  await page.keyboard.press('Escape')
  await dialog.waitFor({ state: 'detached' })

  // Unsaved changes: Save version saves them as a new version, which then shows.
  await page.locator('.cm-content').first().click()
  await page.keyboard.press('Control+End')
  await page.keyboard.type('\nOne more line.')
  await page.waitForFunction(() => window.__workspace.useWorkspace.getState().tabs.find((t) => t.title.startsWith('Notes'))?.dirty)
  await openHistory('Notes')
  await dialog.getByRole('button', { name: 'Save version' }).click()
  await page.waitForFunction(() => document.querySelector('[role=slider]')?.getAttribute('aria-valuemax') === '4')
  await dialog.getByRole('button', { name: 'Save version' }).waitFor({ state: 'detached' })
  await page.waitForFunction(() => document.querySelector('.vh-page')?.textContent.includes('One more line.'))
  await page.keyboard.press('Escape')

  // A file with no versions says so, and has no timeline.
  await open(page, FRESH)
  await page.locator('.tab', { hasText: 'Fresh' }).waitFor()
  await openHistory('Fresh')
  await dialog.getByText(/No versions yet/).waitFor()
  assert.equal(await slider.count(), 0)
  await page.keyboard.press('Escape')

  // Markdown shows as it reads, not as its source.
  await open(page, README)
  await page.locator('.tab', { hasText: 'Readme' }).waitFor()
  await openHistory('Readme')
  await settled()
  assert.equal(await dialog.locator('.md-body h1').innerText(), 'Opening day')
  assert.equal(await dialog.locator('.md-body strong').innerText(), 'nine')
  // A link opens in the browser; the app stays where it is.
  const url = page.url()
  await dialog.getByRole('link', { name: 'the map' }).click()
  await page.waitForFunction(() => window.__fake.opened.includes('https://example.com/map'))
  assert.equal(page.url(), url)
  // A link to a heading scrolls to it.
  await dialog.getByRole('link', { name: 'the café' }).click()
  await page.waitForFunction(() => {
    const heading = [...document.querySelectorAll('.vh-canvas h2')].find((h) => h.textContent === 'Café')
    return heading && heading.getBoundingClientRect().top < innerHeight
  })
  await page.keyboard.press('Escape')

  // Text reads in the encoding the editor reads it in.
  await open(page, OLD)
  await page.locator('.tab', { hasText: 'Old' }).waitFor()
  await openHistory('Old')
  await settled()
  assert.equal(await pageText(), 'Café menu\n')
  await page.keyboard.press('Escape')

  // A workbook shows as its grid, cells as Writan shows them, a sheet at a time.
  await open(page, BUDGET)
  await page.locator('.tab', { hasText: 'Budget' }).waitFor()
  await openHistory('Budget')
  await settled()
  await dialog.locator('.sg-cells').getByText('Travel').waitFor()
  assert.match(await cellsText(), /350/)
  assert.doesNotMatch(await cellsText(), /45962/)
  assert.match(await cellsText(), /2025/)
  assert.deepEqual(await dialog.getByRole('tab').allInnerTexts(), ['Budget', 'Notes'])
  await shot('budget-latest')
  // The older version: Food was 300, and there was no Travel.
  await page.keyboard.press('ArrowLeft')
  await page.waitForFunction(() => !document.querySelector('.vh-canvas .sg-cells')?.textContent.includes('Travel'))
  assert.match(await cellsText(), /300/)
  // It can't be changed: typing, Delete and paste do nothing, and the open workbook stays as it was.
  const food = dialog.locator('.sg-cells span', { hasText: /^300$/ })
  const at = await food.boundingBox()
  await page.mouse.click(at.x + at.width / 2, at.y + at.height / 2)
  await page.keyboard.press('Delete')
  await page.keyboard.type('999')
  await page.keyboard.press('Enter')
  assert.match(await cellsText(), /300/)
  assert.doesNotMatch(await cellsText(), /999/)
  // In the grid, the arrow keys move around the cells, not along the timeline.
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('ArrowRight')
  assert.equal(await slider.getAttribute('aria-valuenow'), '1')
  // Another sheet.
  await dialog.getByRole('tab', { name: 'Notes' }).click()
  await dialog.locator('.sg-cells').getByText('Check in May').waitFor()
  await page.keyboard.press('Escape')
  const budgetDirty = () => page.evaluate(() => window.__workspace.useWorkspace.getState().tabs.find((t) => t.title.startsWith('Budget'))?.dirty)
  assert.equal(await budgetDirty(), false)
  // Opened from the menu while a cell of the workbook has the keyboard: what's typed doesn't reach it.
  const rent = page.locator('.sheet-grid .sg-cells span', { hasText: /^Rent$/ })
  const cell = await rent.boundingBox()
  await page.mouse.click(cell.x + cell.width / 2, cell.y + cell.height / 2)
  await page.evaluate(() => window.__fake.emit('menu', 'version-history'))
  await dialog.waitFor()
  await page.keyboard.type('777')
  await page.keyboard.press('Delete')
  await page.keyboard.press('Escape')
  await dialog.waitFor({ state: 'detached' })
  assert.equal(await budgetDirty(), false)

  // A Word document shows its pages.
  await open(page, DOCX)
  await page.locator('.tab', { hasText: 'Library Update' }).waitFor()
  await openHistory('Library Update')
  await settled()
  await page.waitForFunction(() => document.querySelectorAll('.vp-page .docx-page-content span').length > 20)
  assert.ok((await dialog.locator('.vp-page').count()) >= 1)
  await shot('docx')
  await page.keyboard.press('Escape')

  // A presentation shows its slides.
  await open(page, PPTX)
  await page.locator('.tab', { hasText: 'Spring Launch' }).waitFor()
  await openHistory('Spring Launch')
  await settled()
  await page.waitForFunction(() => document.querySelector('.vp-slide .slide-surface span'))
  assert.ok((await dialog.locator('.vp-slide').count()) > 1)
  await shot('pptx')
  await page.keyboard.press('Escape')

  // A PDF shows in the PDF viewer.
  await open(page, PDF)
  await page.locator('.tab', { hasText: 'Lease' }).waitFor()
  await openHistory('Lease')
  await settled()
  // Its pages are laid out at a real size, and the first is drawn.
  await page.waitForFunction(() => {
    const page = document.querySelector('.vp-pdf [data-page-number]')
    return page && page.getBoundingClientRect().height > 200 && document.querySelector('.vp-pdf canvas, .vp-pdf img')
  })
  assert.equal(await dialog.getByText(/can’t be shown here/).count(), 0)
  await shot('pdf')
  await page.keyboard.press('Escape')

  // Scrubbing between two PDFs: the text that can be selected and copied is the version's own.
  await open(page, LETTER)
  await page.locator('.tab', { hasText: 'Letter' }).waitFor()
  await openHistory('Letter')
  const pdfText = (text) => page.waitForFunction((text) => document.querySelector('.vp-pdf .text-layer')?.textContent.includes(text), text)
  await pdfText('Version two')
  await page.keyboard.press('ArrowLeft')
  await pdfText('Version one')
  await page.keyboard.press('Escape')
} finally {
  await done(browser, logs)
}
