import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { readFile, readdir } from 'node:fs/promises'
import path from 'node:path'
import { gzipSync } from 'node:zlib'
import { arabic, english, hebrew, makeScans, readPdf, requirePacks, root, withOcrBrowser } from '../tests/stress/ui/offline-ocr.mjs'

const baselineDir = process.env.WRITAN_BASELINE_DIR || path.resolve(root, '.cache/baseline')
const distAssets = path.join(root, 'dist/assets')
const documents = [
  { stem: 'baseline-word', extension: 'docx', method: 'exportWordToPdf', fallback: 'scripts/tour/Library Update.docx' },
  { stem: 'baseline-slides', extension: 'pptx', method: 'exportSlidesToPdf', fallback: 'scripts/tour/Spring Launch.pptx' },
  { stem: 'baseline-sheets', extension: 'xlsx', method: 'exportSheetsToPdf', fallback: 'crates/office-sheets/tests/poi-simple-monthly-budget.xlsx' },
]
const scans = {
  eng: { lines: english },
  heb: { lines: hebrew, font: 'Hebrew', rtl: true },
  ara: { lines: arabic, font: 'Arabic', rtl: true, fontSize: 45, leading: 75 },
}
const ms = time => Math.round(time * 10) / 10

async function measureAssets(startupRequests) {
  const startupFiles = new Set(startupRequests.map(url => path.posix.basename(new URL(url).pathname)))
  const names = (await readdir(distAssets)).sort()
  const rows = []
  for (const file of names) {
    let group
    if (/^index-.*\.js$/.test(file)) group = 'App entry'
    else if (/^office_wasm.*\.wasm$/.test(file)) group = 'Word and Office WASM'
    else if (/^office_pdf_wasm.*\.wasm$/.test(file)) group = 'PDF export WASM'
    else if (/^pdfium.*\.wasm$/.test(file)) group = 'PDFium WASM'
    else if (/^tesseract-core/.test(file)) group = file.endsWith('.wasm') ? 'Tesseract WASM' : 'Tesseract glue'
    else if (/^DocxEditor-.*\.js$/.test(file)) group = 'Word editor'
    else if (/^PdfEditor-.*\.js$/.test(file)) group = 'PDF editor'
    else if (/^worker-.*\.js$/.test(file)) group = 'Worker'
    if (!group) continue
    const contents = await readFile(path.join(distAssets, file))
    if (group === 'Worker') {
      const js = contents.toString()
      if (js.includes('office_pdf_wasm')) group = 'PDF export worker'
      else if (js.includes('tesseract-core')) group = 'OCR worker'
      else if (js.includes('pdfium')) group = 'PDFium worker'
    }
    rows.push({ group, file, bytes: contents.length, gzipBytes: gzipSync(contents).length, startup: startupFiles.has(file) })
  }
  for (const group of ['App entry', 'PDF export WASM', 'PDF export worker', 'Tesseract WASM', 'OCR worker', 'Word editor', 'PDFium worker']) {
    assert(rows.some(row => row.group === group), `Missing production asset group: ${group}`)
  }
  assert(rows.find(row => row.group === 'App entry').startup)
  assert.deepEqual(rows.filter(row => /^(?:PDF export|Tesseract|OCR)/.test(row.group) && row.startup), [],
    'PDF export and OCR assets must stay lazy')
  return rows
}

function countJpegXObjects(bytes) {
  // The writers emit image XObjects as streams. Check the stream dictionaries,
  // where both the image subtype and the DCT filter must appear.
  const pdf = bytes.toString('latin1')
  return Array.from(pdf.matchAll(/(?:^|[\r\n])\d+\s+\d+\s+obj\b((?:(?!\bendobj\b)[\s\S])*?)\bstream(?:\r\n|\r|\n)/g))
    .filter(([, dict]) => /\/Subtype\s*\/Image\b/.test(dict) && /\/DCTDecode\b/.test(dict)).length
}

async function summarizePdf(page, bytes) {
  const pages = await page.evaluate(readPdf, { base64: bytes.toString('base64') })
  return {
    bytes: bytes.length,
    pages: pages.length,
    chars: pages.reduce((sum, page) => sum + page.chars, 0),
    links: pages.reduce((sum, page) => sum + page.links.length, 0),
    jpegXObjects: countJpegXObjects(bytes),
  }
}

async function measureExports(page) {
  const rows = []
  for (const { stem, extension, method, fallback } of documents) {
    const name = stem + '.' + extension
    const file = [
      path.join(baselineDir, name),
      path.join(root, 'tests/fixtures', name),
      path.join(root, fallback),
    ].find(file => existsSync(file))
    assert(file, `Missing input fixture: ${name}`)
    const baselineFile = path.join(baselineDir, stem + '-baseline.pdf')
    const input = await readFile(file)
    const result = await page.evaluate(async ({ method, encoded, name }) => {
      const bytes = Uint8Array.fromBase64(encoded)
      const start = performance.now()
      const pdf = await window.ocrApi.exporter[method](bytes, name)
      return { encoded: pdf.toBase64(), elapsed: performance.now() - start }
    }, { method, encoded: input.toString('base64'), name: path.basename(file) })
    const vector = await summarizePdf(page, Buffer.from(result.encoded, 'base64'))
    assert(vector.pages > 0 && vector.chars > 0, `${name} should export searchable vector text`)
    const comparable = file === path.join(baselineDir, name) && existsSync(baselineFile)
    const baseline = comparable ? await summarizePdf(page, await readFile(baselineFile)) : null
    rows.push({ input: path.basename(file), exportMs: ms(result.elapsed), vector, baseline })
  }
  return rows
}

async function measureOcr({ page, assets, requests }) {
  await makeScans(page, scans)
  const downloads = await page.evaluate(async langs => {
    const { packs } = window.ocrApi
    const times = {}
    for (const lang of langs) {
      const start = performance.now()
      await packs.downloadPack(lang)
      times[lang] = performance.now() - start
    }
    return times
  }, Object.keys(scans))
  await assets.close()
  const start = requests.length
  const results = await page.evaluate(async langs => {
    const { exporter, packs, pdfClient, pdfOcr, toDocx } = window.ocrApi
    const client = pdfClient.pdfEngineClient
    const originalInject = client.injectOcrTextLayer
    let injectionMs
    client.injectOcrTextLayer = async (...args) => {
      const start = performance.now()
      const result = await originalInject.apply(client, args)
      injectionMs = performance.now() - start
      return result
    }
    try {
      const rows = []
      for (const lang of langs) {
        const scan = window.ocrScans[lang]
        const pdfBytes = await exporter.exportImageToPdf(scan.jpeg, 'fixture.jpg')
        const start = performance.now()
        const [{ bytes }] = await packs.ensurePacks([lang])
        const cacheReadMs = performance.now() - start
        injectionMs = undefined
        window.ocrWorkerMessages.length = 0
        const ocr = await pdfOcr.ocrPdfDocument({ pdfBytes, langs: [lang] })
        const output = ocr.searchablePdfBytes
        const events = new Map(window.ocrWorkerMessages.map(event => [event.kind, event.at]))
        const docxStart = performance.now()
        const docx = await toDocx.ocrResultsToDocx(ocr.pages, lang + ' OCR benchmark')
        const docxMs = performance.now() - docxStart
        rows.push({
          lang, packBytes: bytes.length, cacheReadMs, workerInitMs: events.get('ready') - events.get('initialize'),
          recognitionMs: ocr.pages[0].elapsedMs, injectionMs, docxMs, docxBytes: docx.length,
          chars: [...ocr.pages[0].text].length, inputPdfBytes: pdfBytes.length, searchablePdfBytes: output.length,
          lossless: pdfBytes.every((byte, index) => byte === output[index]),
        })
      }
      return rows
    } finally {
      client.injectOcrTextLayer = originalInject
    }
  }, Object.keys(scans))
  assert.deepEqual(requests.slice(start).filter(url => url.startsWith(assets.url)), [], 'OCR timings must come from cached packs')
  return results.map(({ lossless, ...result }) => {
    assert(lossless && result.searchablePdfBytes > result.inputPdfBytes && result.chars > 0)
    assert(result.docxBytes > 0 && Number.isFinite(result.workerInitMs) && Number.isFinite(result.injectionMs))
    return {
      ...result, packDownloadMs: ms(downloads[result.lang]), cacheReadMs: ms(result.cacheReadMs),
      workerInitMs: ms(result.workerInitMs), recognitionMs: ms(result.recognitionMs),
      injectionMs: ms(result.injectionMs), docxMs: ms(result.docxMs),
    }
  })
}

function printTable(headers, rows) {
  console.log(`| ${headers.join(' | ')} |`)
  console.log(`| ${headers.map(() => '---').join(' | ')} |`)
  for (const row of rows) console.log(`| ${row.join(' | ')} |`)
}

requirePacks(Object.keys(scans))
await withOcrBrowser(async session => {
  const assets = await measureAssets(session.startupRequests)
  const exports = await measureExports(session.page)
  const ocr = await measureOcr(session)
  console.log('PDF and OCR benchmark (one sample per fixture, milliseconds)')
  console.log(JSON.stringify({ assets, exports, ocr }, null, 2))
  console.log('\nProduction assets')
  printTable(['Asset', 'File', 'Bytes', 'Gzip bytes', 'Startup'], assets.map(row => [
    row.group, row.file, row.bytes, row.gzipBytes, row.startup ? 'yes' : 'no',
  ]))
  console.log('\nVector exports and saved baselines')
  printTable(['Fixture', 'PDF', 'Bytes', 'Export ms', 'Pages', 'PDFium chars', 'Links', 'JPEG XObjects'], exports.flatMap(row => {
    const metrics = pdf => [pdf.pages, pdf.chars, pdf.links, pdf.jpegXObjects]
    return [
      [row.input, 'Vector', row.vector.bytes, row.exportMs, ...metrics(row.vector)],
      ...(row.baseline ? [[row.input, 'Saved baseline', row.baseline.bytes, '—', ...metrics(row.baseline)]] : []),
    ]
  }))
  console.log('\nOffline OCR and reconstruction')
  printTable(['Pack', 'Download ms', 'IndexedDB read ms', 'Worker init ms', 'Recognition ms', 'Lossless injection ms', 'DOCX ms', 'Chars'],
    ocr.map(row => [row.lang, row.packDownloadMs, row.cacheReadMs, row.workerInitMs, row.recognitionMs, row.injectionMs, row.docxMs, row.chars]))
})
