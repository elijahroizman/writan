// Linked Excel ranges and charts in Word and Slides: live auto-update when the source
// workbook changes, manifest persistence in saved .docx/.pptx packages, and unlink.
import { launch } from './harness.mjs'

const { browser, page, logs } = await launch()
const errors = []
const check = (name, ok, detail = '') => {
  console.log(`${ok ? 'ok' : 'FAIL'} ${name}${ok ? '' : ` — ${detail}`}`)
  if (!ok) errors.push(name)
}

try {
  await page.goto(process.env.WRITAN_URL ?? 'http://localhost:1420/')
  await page.waitForSelector('.home', { timeout: 30000 })

  // 1. Create a spreadsheet with data, a formula, and a chart.
  await page.getByRole('button', { name: 'New workbook', exact: true }).first().click()
  await page.waitForFunction(() => !!window.__sheets, null, { timeout: 30000 })

  await page.evaluate(() => {
    const s = window.__sheets
    s.run({ cmd: 'input', sheet: 0, row: 0, col: 0, text: 'Metric' })
    s.run({ cmd: 'input', sheet: 0, row: 0, col: 1, text: 'Value' })
    s.run({ cmd: 'input', sheet: 0, row: 1, col: 0, text: 'Revenue' })
    s.run({ cmd: 'input', sheet: 0, row: 1, col: 1, text: '100' })
    s.run({ cmd: 'input', sheet: 0, row: 2, col: 0, text: 'Double' })
    s.run({ cmd: 'input', sheet: 0, row: 2, col: 1, text: '=B2*2' })
    s.run({ cmd: 'addChart', sheet: 0, range: { r0: 0, c0: 0, r1: 2, c1: 1 }, kind: 'bar' })
  })

  const snap = await page.evaluate(() => window.__sheets.rangeSnapshot(0, 'A1:B3'))
  check('sheets rangeSnapshot evaluates formula', snap?.rows?.[2]?.[1]?.text === '200', JSON.stringify(snap))
  const chartCount = await page.evaluate(() => window.__sheets.chartsOf(0).length)
  check('sheets workbook has chart', chartCount === 1, String(chartCount))

  // 2. Open a new Word document and link both the Excel range and chart via the UI modal.
  await page.getByRole('tab', { name: 'Home' }).click()
  await page.getByRole('button', { name: 'New document', exact: true }).first().click()
  await page.waitForFunction(() => !!window.__docxEditor && !!window.__docxLinks, null, { timeout: 30000 })

  const activeExcelBtn = () => page.locator('.view[data-active=true] button[title="Link a live range or chart from an open Excel workbook"]')

  // Insert linked range via UI modal
  await activeExcelBtn().click()
  await page.locator('[data-testid="excel-link-range"]').fill('A1:B3')
  await page.locator('[data-testid="excel-link-insert-btn"]').click()
  await page.waitForFunction(() => window.__docxLinks.links.length === 1, null, { timeout: 15000 })

  // Insert linked chart via UI modal
  await activeExcelBtn().click()
  await page.locator('[data-testid="excel-link-kind-chart"]').click()
  await page.locator('[data-testid="excel-link-insert-btn"]').click()
  await page.waitForFunction(() => window.__docxLinks.links.length === 2, null, { timeout: 15000 })

  const wordHas = (q) => page.evaluate((query) => window.__docxEditor.doc.paint(0, true).includes(query), q)
  check('Word linked range shows initial evaluated value 200', (await wordHas('Revenue')) && (await wordHas('200')))
  const wordTags = await page.evaluate(() => window.__docxEditor.linkedTags())
  check('Word document stores both range and chart link tags', wordTags.length === 2, JSON.stringify(wordTags))

  // 3. Open a new Slides presentation and link both the Excel range and chart via the UI modal.
  await page.getByRole('tab', { name: 'Home' }).click()
  await page.getByRole('button', { name: 'New presentation', exact: true }).first().click()
  await page.waitForFunction(() => !!window.__slides && !!window.__slidesLinks, null, { timeout: 30000 })

  await activeExcelBtn().click()
  await page.locator('[data-testid="excel-link-range"]').fill('A1:B3')
  await page.locator('[data-testid="excel-link-insert-btn"]').click()
  await page.waitForFunction(() => window.__slidesLinks.links.length === 1, null, { timeout: 15000 })

  await activeExcelBtn().click()
  await page.locator('[data-testid="excel-link-kind-chart"]').click()
  await page.locator('[data-testid="excel-link-insert-btn"]').click()
  await page.waitForFunction(() => window.__slidesLinks.links.length === 2, null, { timeout: 15000 })

  const slidesInitialText = await page.evaluate(() => JSON.parse(window.__slides.doc.elements(0)).map((e) => e.text || '').join('\n'))
  check('Slides linked range shows initial evaluated value 200', slidesInitialText.includes('200'), slidesInitialText)
  const slidesTags = await page.evaluate(() => window.__slides.linkedTags())
  check('Slides deck stores both range and chart link tags', slidesTags.length === 2, JSON.stringify(slidesTags))

  // 4. Edit the source Excel workbook and verify live auto-update in BOTH Word and Slides!
  await page.evaluate(() => {
    window.__sheets.run({ cmd: 'input', sheet: 0, row: 1, col: 1, text: '350' })
  })

  await page.waitForFunction(
    () => {
      const wOk = window.__docxEditor.doc.paint(0, true).includes('700')
      const sOk = JSON.parse(window.__slides.doc.elements(0)).some((e) => (e.text || '').includes('700'))
      return wOk && sOk
    },
    null,
    { timeout: 15000 },
  )
  check('Word linked range auto-updated to 700', await wordHas('700'))
  check(
    'Slides linked range auto-updated to 700',
    await page.evaluate(() => JSON.parse(window.__slides.doc.elements(0)).some((e) => (e.text || '').includes('700'))),
  )

  // 5. Verify saved .docx and .pptx packages contain the persisted writan/excel-links.json manifest.
  const manifestCheck = await page.evaluate(async () => {
    const docxBytes = await window.__docxLinks.writeToBytes(window.__docxEditor.save())
    const pptxBytes = await window.__slidesLinks.writeToBytes(window.__slides.save())
    const textDec = new TextDecoder()
    return {
      docxHasManifest: textDec.decode(docxBytes).includes('writan/excel-links.json'),
      pptxHasManifest: textDec.decode(pptxBytes).includes('writan/excel-links.json'),
    }
  })
  check('saved .docx includes writan/excel-links.json manifest', manifestCheck.docxHasManifest)
  check('saved .pptx includes writan/excel-links.json manifest', manifestCheck.pptxHasManifest)

  // 6. Unlink the range in Slides and verify further Excel edits no longer change it.
  await page.evaluate(() => {
    const rangeLink = window.__slidesLinks.links.find((l) => l.kind === 'range')
    if (rangeLink) window.__slidesLinks.unlink(rangeLink.id)
  })
  await page.evaluate(() => {
    window.__sheets.run({ cmd: 'input', sheet: 0, row: 1, col: 1, text: '500' })
  })
  await page.waitForFunction(() => window.__docxEditor.doc.paint(0, true).includes('1000'), null, { timeout: 15000 })
  const slidesStill700 = await page.evaluate(() => JSON.parse(window.__slides.doc.elements(0)).some((e) => (e.text || '').includes('700')))
  check('unlinked Slides range stays at 700 while linked Word range updates to 1000', slidesStill700)
} catch (e) {
  check('linked excel flow', false, e.stack || e.message)
}

for (const l of logs) console.log(' ', l)
await browser.close()
process.exit(errors.length ? 1 : 0)
