// Opens a PDF as an editable Word document from the PDF inspector's button, as Word does.
// The PDF is made by Writan itself from Markdown, so the text it should get back is known.
// Run against a VITE_TEST_HOOKS build with WRITAN_URL set to its preview URL.
import assert from 'node:assert/strict'
import JSZip from 'jszip'
import { BASE, launch } from './harness.mjs'

const markdown = `# Field Report

This report describes the work of the **survey team** between April and June. The team visited eleven sites and collected samples from each of them, and water levels were higher than in the previous quarter. More is on [the project site](https://example.com/survey).

## Sites

- Upper ridge, reached on foot
- Lake shore, where the gauge is
- The old quarry

| Site | Samples | Depth |
| --- | ---: | ---: |
| Upper ridge | 14 | 2.5 |
| Lake shore | 22 | 4.0 |
| Quarry | 9 | 12.5 |

The quarry samples were taken from the lower terrace only.
`

const { browser, page, logs } = await launch({ tauri: false })
try {
  await page.goto(BASE)
  await page.waitForFunction(() => !!window.__workspace && !!window.__loadPdfOcrTestApi)
  const pdf = await page.evaluate(async (md) => {
    const { convert } = await window.__loadPdfOcrTestApi()
    const ws = window.__workspace.useWorkspace.getState()
    ws.openUnsaved(new TextEncoder().encode(md), 'Field report.md')
    const tab = window.__workspace.useWorkspace.getState().tabs.find((t) => t.kind === 'md')
    const { bytes } = await convert.convertTab(tab, 'pdf')
    await ws.closeTab(tab.id)
    window.__workspace.useWorkspace.getState().openUnsaved(bytes, 'Field report.pdf')
    return bytes.length
  }, markdown)
  assert.ok(pdf > 1000, 'Expected a PDF made from the Markdown')

  const button = page.getByRole('button', { name: 'Edit as Word document' })
  await button.waitFor()
  await page.waitForFunction(() => !document.querySelector('.pane.right button.primary[disabled]'))
  await button.click()
  await page.waitForFunction(() => window.__workspace.useWorkspace.getState().tabs.some((t) => t.kind === 'docx'), null, { timeout: 60_000 })
  const result = await page.evaluate(() => {
    const { tabs, activeId } = window.__workspace.useWorkspace.getState()
    const tab = tabs.find((t) => t.kind === 'docx')
    return { title: tab.title, active: tab.id === activeId, dirty: tab.dirty, pdfStillOpen: tabs.some((t) => t.kind === 'pdf'), bytes: Array.from(tab.bytes ?? []) }
  })
  assert.equal(result.title, 'Field report', 'The Word copy is named after the PDF')
  assert.ok(result.active, 'The Word copy opens in front')
  assert.ok(result.pdfStillOpen, 'The PDF stays open as it was')

  const zip = await JSZip.loadAsync(Buffer.from(result.bytes))
  const xml = await zip.file('word/document.xml').async('string')
  const paragraphs = [...xml.matchAll(/<w:p[ >][\s\S]*?<\/w:p>/g)].map((m) => m[0])
  const text = (p) => [...p.matchAll(/<w:t(?: [^>]*)?>([^<]*)<\/w:t>/g)].map((m) => m[1]).join('').replaceAll('&amp;', '&')
  const find = (start) => paragraphs.find((p) => text(p).startsWith(start))

  assert.match(find('Field Report') ?? '', /<w:pStyle w:val="Heading1"\/>/, 'The title becomes a heading')
  assert.match(find('Sites') ?? '', /<w:pStyle w:val="Heading\d"\/>/, 'The section title becomes a heading')
  const body = find('This report describes')
  assert.ok(body, 'The paragraph is there')
  assert.match(text(body), /water levels were higher than in the previous quarter\. More is on the project site\.$/, 'Its wrapped lines are one paragraph')
  assert.match(body, /<w:b\/>[\s\S]*?survey team/, 'Bold text stays bold')
  assert.match(body, /<w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"\/>/, 'Carlito, the stand-in Writan drew the PDF with, is Calibri again')
  assert.match(body, /<w:sz w:val="22"\/>/, 'The text is 11 point, as in the Markdown it came from')
  assert.match(body, /<w:hyperlink [^>]*r:id=/, 'The link is still a link')
  for (const item of ['Upper ridge, reached on foot', 'Lake shore, where the gauge is', 'The old quarry']) {
    assert.match(find(item) ?? '', /<w:numPr>/, `"${item}" is a list item`)
  }
  const table = xml.match(/<w:tbl>[\s\S]*?<\/w:tbl>/)?.[0] ?? ''
  const rows = [...table.matchAll(/<w:tr[ >][\s\S]*?<\/w:tr>/g)].map((r) => [...r[0].matchAll(/<w:tc>[\s\S]*?<\/w:tc>/g)].map((c) => text(c[0])))
  assert.deepEqual(rows, [['Site', 'Samples', 'Depth'], ['Upper ridge', '14', '2.5'], ['Lake shore', '22', '4.0'], ['Quarry', '9', '12.5']], 'The table is a table')
  assert.ok(find('The quarry samples were taken'), 'Text after the table is there')
  assert.match(xml, /<w:pgSz w:w="1190[56]" w:h="1683[78]"\/>/, 'The page is A4, like the PDF')

  // The Word editor shows it.
  await page.locator('.docx-page').first().waitFor()
  assert.ok(!logs.some((line) => line.startsWith('[pageerror]')), logs.join('\n'))
  console.log('PASS PDF opens as an editable Word document')
} finally {
  await browser.close()
}
