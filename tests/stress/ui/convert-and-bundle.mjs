// End-to-end Chromium verification of one-click file conversion (DOCX <-> clean LLM Markdown,
// Office formats, PDF) and bundling multiple open tabs into one merged PDF.

import { mkdirSync, writeFileSync } from 'node:fs'
import { BASE, launch } from './harness.mjs'

function assert(cond, msg) {
  if (!cond) throw new Error(`FAIL: ${msg}`)
}

mkdirSync('/tmp/writan-stress', { recursive: true })
const mdPath = '/tmp/writan-stress/spec.md'
const csvPath = '/tmp/writan-stress/metrics.csv'

writeFileSync(
  mdPath,
  [
    '# Launch Spec',
    '',
    'Intro with **bold**, *italic*, `code`, ~~old~~, and [Writan](https://writan.app).',
    '',
    '## Checklist',
    '',
    '- Fast export',
    '- Clean markdown',
    '',
    '| Metric | Value |',
    '| --- | --- |',
    '| Latency | 12ms |',
    '| Score | 99 |',
    '',
  ].join('\n'),
)

writeFileSync(csvPath, 'Item,Qty,Price\nApples,4,1.50\nOranges,6,2.00\n')

const { browser, context, page, logs } = await launch({
  init: () => {
    window.__copied = ''
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: {
        writeText: async (text) => {
          window.__copied = text
        },
        readText: async () => window.__copied,
      },
    })
  },
})

try {
  const url = `${BASE}?open=${encodeURIComponent(mdPath)}&open=${encodeURIComponent(csvPath)}`
  await page.goto(url, { waitUntil: 'domcontentloaded' })
  await page.waitForSelector('.tab:has-text("spec")', { timeout: 15000 })
  await page.waitForSelector('.tab:has-text("metrics")', { timeout: 15000 })

  // Switch to spec.md and open Convert file… via Command Palette.
  await page.click('.tab:has-text("spec")')
  await page.keyboard.press('Control+k')
  await page.waitForSelector('.palette input')
  await page.fill('.palette input', 'Convert file')
  await page.keyboard.press('Enter')
  await page.waitForSelector('[aria-label="Convert file"]')

  // Convert spec.md -> Word (.docx) (opens as new tab).
  await page.click('[aria-label="Convert file"] button:has-text("Word (.docx)")')

  // Wait for the new Word editor tab to open and render.
  await page.waitForSelector('.docx-page', { timeout: 20000 })
  console.log('PASS: Markdown -> Word (.docx) opened and rendered')

  // Open Convert dialog on the .docx tab and click "Copy as clean Markdown (for LLMs)".
  await page.keyboard.press('Control+k')
  await page.waitForSelector('.palette input')
  await page.fill('.palette input', 'Convert file')
  await page.keyboard.press('Enter')
  await page.waitForSelector('[aria-label="Convert file"]')
  await page.click('button:has-text("Copy as clean Markdown")')

  await page.waitForFunction(() => (window.__copied || '').includes('# Launch Spec'))
  const copiedMd = await page.evaluate(() => window.__copied)
  assert(copiedMd.includes('# Launch Spec'), `missing H1 in markdown:\n${copiedMd}`)
  assert(copiedMd.includes('**bold**'), `missing bold in markdown:\n${copiedMd}`)
  assert(copiedMd.includes('*italic*'), `missing italic in markdown:\n${copiedMd}`)
  assert(copiedMd.includes('`code`'), `missing inline code in markdown:\n${copiedMd}`)
  assert(copiedMd.includes('~~old~~'), `missing strikethrough in markdown:\n${copiedMd}`)
  assert(copiedMd.includes('[Writan](https://writan.app)'), `missing link in markdown:\n${copiedMd}`)
  assert(copiedMd.includes('- Fast export'), `missing bullet list in markdown:\n${copiedMd}`)
  assert(copiedMd.includes('| Metric | Value |'), `missing table in markdown:\n${copiedMd}`)
  console.log('PASS: DOCX -> clean LLM Markdown verified')

  // Wait for Convert dialog to auto-close after copy.
  await page.waitForSelector('[aria-label="Convert file"]', { state: 'detached' })

  // Now open "Bundle open files into one PDF…" via Command Palette.
  await page.keyboard.press('Control+k')
  await page.waitForSelector('.palette input')
  await page.fill('.palette input', 'Bundle open files')
  await page.keyboard.press('Enter')
  await page.waitForSelector('[aria-label="Bundle open files into one PDF"]')

  const rowCount = await page.locator('.bundle-row').count()
  assert(rowCount >= 3, `expected >= 3 bundle rows, got ${rowCount}`)

  // Reorder: move first row down.
  await page.locator('.bundle-row').first().locator('button[aria-label*="down"]').click()

  // Click "Merge N into PDF".
  await page.click('.bundle-actions button.primary')

  // Wait for either the bundle dialog to close (merged PDF tab opened) or an error to appear.
  await page.waitForFunction(
    () => !document.querySelector('[aria-label="Bundle open files into one PDF"]') || !!document.querySelector('.convert-error'),
    undefined,
    { timeout: 30000 },
  )
  const bundleErr = await page.locator('.convert-error').textContent().catch(() => null)
  assert(!bundleErr, `Bundle dialog showed error: ${bundleErr}\nLogs:\n${logs.join('\n')}`)

  // Wait for the merged PDF tab to open and render pages in PDFium editor.
  await page.waitForSelector('.tab:has-text("bundle")', { timeout: 15000 })
  await page.waitForSelector('[data-page]', { timeout: 20000 })
  const pdfPages = await page.locator('[data-page]').count()
  assert(pdfPages >= 3, `expected >= 3 merged PDF pages, got ${pdfPages}`)
  console.log(`PASS: Bundled ${rowCount} open tabs into one ${pdfPages}-page PDF`)

  const errors = logs.filter((l) => l.startsWith('[pageerror]') || l.startsWith('[error]'))
  assert(errors.length === 0, `unexpected browser errors:\n${errors.join('\n')}`)
  console.log('ALL CHECKS PASSED')
} finally {
  await context.close()
  await browser.close()
}
