import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { existsSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { createServer } from 'node:http'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { fileURLToPath, pathToFileURL } from 'node:url'
import JSZip from 'jszip'
import { launch } from './harness.mjs'

export const root = fileURLToPath(new URL('../../../', import.meta.url))
export const tessdataDir = process.env.WRITAN_TESSDATA_DIR || path.resolve(root, '.cache/tessdata')
const host = '127.0.0.1'
const appUrl = 'http://127.0.0.1:4173/'
const lazyAssets = /(?:tesseract-core|office_pdf_wasm)/i
const fontPaths = {
  Latin: 'LiberationSans-Regular.ttf',
  Hebrew: 'LiberationSans-Regular.ttf',
  Arabic: 'pdf/NotoSansArabic-Regular.ttf',
}

export const english = [
  'The quick brown fox jumps over the lazy dog.',
  'A bright morning filled the quiet valley.',
  'The river flows between the green hills.',
  'We walked across the bridge before lunch.',
]
export const hebrew = [
  'שלום עולם זהו יום יפה ונעים',
  'השמש זורחת מעל ההרים הגבוהים',
  'אנחנו הולכים לאורך הנהר השקט',
  'בערב נחזור הביתה ונקרא ספר',
]
export const arabic = [
  'هذا يوم جميل في فصل الربيع',
  'تشرق الشمس فوق الجبال العالية',
  'نسير على الطريق بجانب النهر',
  'وفي المساء نعود إلى البيت',
]

async function previewReady() {
  try {
    const response = await fetch(appUrl, { signal: AbortSignal.timeout(1000) })
    await response.body?.cancel()
    return response.ok
  } catch {
    return false
  }
}

async function startPreview() {
  if (await previewReady()) return async () => {}
  assert(existsSync(path.join(root, 'dist/index.html')), 'Run VITE_TEST_HOOKS=1 npm run build first.')
  // Invoke the same Vite executable as npx, so closing the child also closes preview on Windows.
  const child = spawn(process.execPath, ['node_modules/vite/bin/vite.js', 'preview', '--host', host, '--port', '4173', '--strictPort'], {
    cwd: root,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  const stopped = once(child, 'close')
  let output = ''
  child.stdout.on('data', chunk => { output += chunk })
  child.stderr.on('data', chunk => { output += chunk })
  const stop = async () => {
    child.kill()
    await stopped
  }
  try {
    const deadline = Date.now() + 30_000
    while (Date.now() < deadline) {
      if (await previewReady()) return stop
      if (child.exitCode !== null) break
      await delay(100)
    }
    throw new Error('Vite preview failed to start.\n' + output)
  } catch (error) {
    await stop()
    throw error
  }
}

async function startAssets() {
  const packRequests = []
  const server = createServer(async (request, response) => {
    const pathname = new URL(request.url, appUrl).pathname
    const pack = /^\/(packs|corrupt)\/([a-z_]+)\.traineddata$/.exec(pathname)
    const font = /^\/fonts\/(Latin|Hebrew|Arabic)\.ttf$/.exec(pathname)
    response.setHeader('Access-Control-Allow-Origin', '*')
    response.setHeader('Cache-Control', 'no-store')
    try {
      if (pack) {
        packRequests.push(pathname)
        let bytes
        if (pack[1] === 'corrupt' && pack[2] === 'heb') {
          // Match the catalog's length so verification must reach the SHA-256 comparison.
          bytes = Buffer.alloc(961404, 0xa5)
        } else if (pack[1] === 'packs') {
          bytes = await readFile(path.join(tessdataDir, pack[2] + '.traineddata'))
        }
        if (bytes) {
          response.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Length': bytes.length })
          response.end(bytes)
          return
        }
      }
      if (font) {
        response.writeHead(200, { 'Content-Type': 'font/ttf' })
        response.end(await readFile(path.join(root, 'src/engine/fonts', fontPaths[font[1]])))
        return
      }
      response.writeHead(404).end()
    } catch (error) {
      if (!response.headersSent) response.writeHead(error.code === 'ENOENT' ? 404 : 500)
      response.end()
    }
  })
  const listening = once(server, 'listening')
  server.listen(0, host)
  await listening
  const url = `http://${host}:${server.address().port}`
  const close = async () => {
    if (server.listening) await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()))
  }
  return { url, packRequests, close }
}

export async function withOcrBrowser(run) {
  const stop = await startPreview()
  let assets
  let browser
  try {
    assets = await startAssets()
    // Web mode exercises the production IndexedDB path.
    const session = await launch({ scale: 2, tauri: false })
    ;({ browser } = session)
    const { context, page, logs } = session
    assert.equal(browser.browserType().name(), 'chromium', 'Run these suites with ENGINE=chromium.')
    const requests = []
    const external = new Set()
    const local = url => ['data:', 'blob:'].includes(url.protocol) || [host, 'localhost'].includes(url.hostname)
    const guard = async route => {
      const url = new URL(route.request().url())
      if (local(url)) {
        // Preserve other harness routes after checking the destination.
        return route.fallback()
      }
      external.add(url.href)
      await route.abort('blockedbyclient')
    }
    context.on('request', request => {
      const url = new URL(request.url())
      requests.push(url.href)
      // Track redirect destinations as well as requests sent to the route handlers.
      if (!local(url)) external.add(url.href)
    })
    await context.route('**/*', guard)
    await page.route('**/*', guard)
    await page.addInitScript(url => {
      window.__ocrPackBaseUrl = url + '/packs'
      const NativeWorker = window.Worker
      window.ocrWorkerMessages = []
      window.Worker = class extends NativeWorker {
        constructor(url, options) {
          super(url, options)
          if (options?.name !== 'writan-ocr') return
          const calls = window.ocrWorkerMessages
          const post = this.postMessage.bind(this)
          this.postMessage = (message, transfer) => {
            calls.push({ kind: message.kind, at: performance.now() })
            post(message, transfer)
            window.onOcrWorkerPost?.(message)
          }
          this.addEventListener('message', ({ data }) => calls.push({ kind: data.kind, at: performance.now() }))
        }
      }
    }, assets.url)
    try {
      await page.goto(appUrl)
      await page.waitForFunction(() => window.__workspace && window.__loadPdfOcrTestApi && document.querySelector('.app'))
      await page.waitForLoadState('networkidle')
      assert.match(await page.title(), /Writan|Workspace/i)
      const startupRequests = requests.slice()
      assert.deepEqual(startupRequests.filter(url => lazyAssets.test(url)), [], 'OCR and PDF export assets must be lazy')
      await page.evaluate(async url => {
        window.ocrApi = await window.__loadPdfOcrTestApi()
        const style = document.createElement('style')
        style.textContent = ['Latin', 'Hebrew', 'Arabic'].map(font =>
          `@font-face { font-family: Fixture${font}; src: url("${url}/fonts/${font}.ttf") format("truetype"); }`).join('\n')
        document.head.append(style)
      }, assets.url)
      await run({ ...session, assets, requests, startupRequests })
      assert.deepEqual(logs.filter(log => log.startsWith('[pageerror]')), [], 'unhandled page errors')
    } finally {
      assert.deepEqual([...external], [], 'external requests were attempted and blocked')
    }
  } finally {
    try {
      await browser?.close()
    } finally {
      await Promise.all([assets?.close(), stop()])
    }
  }
}

export function requirePacks(langs) {
  const missing = langs.filter(lang => !existsSync(path.join(tessdataDir, lang + '.traineddata')))
  assert.deepEqual(missing, [], `Missing OCR packs in ${tessdataDir}`)
}

export async function downloadPacks(page, langs) {
  await page.evaluate(async langs => {
    const { packs } = window.ocrApi
    for (const lang of langs) await packs.downloadPack(lang)
  }, langs)
}

// Serialized into Chromium. The fixture font files come from the same repository as the app.
export async function makeScans(page, fixtures) {
  await page.evaluate(async fixtures => {
    window.ocrScans = {}
    for (const [name, fixture] of Object.entries(fixtures)) window.ocrScans[name] = await drawScan(fixture)

    async function drawScan({ lines, font = 'Latin', rtl = false, degrees = 0, fontSize = 42, leading = 69 }) {
      const width = 1080
      const height = 570
      const fontSpec = `${fontSize}px Fixture${font}`
      const faces = await document.fonts.load(fontSpec, lines.join(' '))
      if (faces.length !== 1 || faces[0].status !== 'loaded') throw new Error('Fixture font is unavailable: ' + font)
      const canvas = document.createElement('canvas')
      canvas.width = Math.abs(degrees) === 90 ? height : width
      canvas.height = Math.abs(degrees) === 90 ? width : height
      const ctx = canvas.getContext('2d')
      ctx.fillStyle = 'white'
      ctx.fillRect(0, 0, canvas.width, canvas.height)
      ctx.translate(canvas.width / 2, canvas.height / 2)
      ctx.rotate(degrees * Math.PI / 180)
      ctx.translate(-width / 2, -height / 2)
      ctx.font = fontSpec
      ctx.fillStyle = 'black'
      ctx.direction = rtl ? 'rtl' : 'ltr'
      ctx.textAlign = rtl ? 'right' : 'left'
      const firstBaseline = (height - (lines.length - 1) * leading) / 2
      for (const [index, line] of lines.entries()) {
        ctx.fillText(line, rtl ? width - 85 : 85, firstBaseline + index * leading)
      }
      const blob = await new Promise(resolve => canvas.toBlob(resolve, 'image/jpeg', 0.96))
      return {
        jpeg: new Uint8Array(await blob.arrayBuffer()), width: canvas.width, height: canvas.height,
        data: ctx.getImageData(0, 0, canvas.width, canvas.height).data,
      }
    }
  }, fixtures)
}

// A PDF with real JPEG XObjects and an optional digital first page, blank pages, a line of real
// text over each scan (as library copies have), scans drawn in a unit-square form XObject on pages
// whose box starts at 100, 100 (`wrapped`), a small copy beside a page-sized drawing in a form
// scaled 2× (`art`), drawn in four strips through a form (`strips`), or a synthetic signature field.
export function scanPdf(scans, { digital = '', blank = 0, footer = '', wrapped = false, art = false, strips = false, signed = false } = {}) {
  const objects = [null, null]
  const add = object => objects.push(object)
  const stream = (dict, content) => {
    const bytes = Buffer.from(content)
    return Buffer.concat([Buffer.from(`<< ${dict} /Length ${bytes.length} >>\nstream\n`), bytes, Buffer.from('\nendstream')])
  }
  const pages = []
  const font = digital || footer ? add('<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>') : 0
  if (digital) {
    const body = add(stream('', `BT /F1 24 Tf 50 500 Td (${digital}) Tj ET`))
    pages.push(add(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 612] /Resources << /Font << /F1 ${font} 0 R >> >> /Contents ${body} 0 R >>`))
  }
  for (let i = 0; i < blank; i++) pages.push(add('<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>'))
  for (const { jpeg, width, height } of scans) {
    const image = add(stream(`/Type /XObject /Subtype /Image /Width ${width} /Height ${height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode`, jpeg))
    const w = width * 0.75
    const h = height * 0.75
    const text = footer ? ` BT /F1 9 Tf 20 12 Td (${footer}) Tj ET` : ''
    const fonts = footer ? ` /Font << /F1 ${font} 0 R >>` : ''
    if (strips) {
      const bands = [0, 0.25, 0.5, 0.75].map(y => `q 1 0 0 0.25 0 ${y} cm /Scan Do Q`).join(' ')
      const form = add(stream(`/Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Scan ${image} 0 R >> >>`, bands))
      const body = add(stream('', `q ${w} 0 0 ${h} 0 0 cm /Fm Do Q`))
      pages.push(add(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 ${w} ${h}] /Resources << /XObject << /Fm ${form} 0 R >> >> /Contents ${body} 0 R >>`))
      continue
    }
    if (art) {
      const form = add(stream(`/Type /XObject /Subtype /Form /BBox [0 0 300 300] /Resources << /XObject << /Scan ${image} 0 R >> >>`, '0 0 300 300 re f q 10 0 0 10 5 5 cm /Scan Do Q'))
      const body = add(stream('', 'q 2 0 0 2 0 0 cm /Fm Do Q'))
      pages.push(add(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 600] /Resources << /XObject << /Fm ${form} 0 R >> >> /Contents ${body} 0 R >>`))
      continue
    }
    if (wrapped) {
      const form = add(stream(`/Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Scan ${image} 0 R >> >>`, '/Scan Do'))
      const body = add(stream('', `q ${w} 0 0 ${h} 100 100 cm /Fm Do Q`))
      pages.push(add(`<< /Type /Page /Parent 2 0 R /MediaBox [100 100 ${100 + w} ${100 + h}] /Resources << /XObject << /Fm ${form} 0 R >> >> /Contents ${body} 0 R >>`))
      continue
    }
    const body = add(stream('', `q ${w} 0 0 ${h} 0 0 cm /Scan Do Q${text}`))
    pages.push(add(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 ${w} ${h}] /Resources << /XObject << /Scan ${image} 0 R >>${fonts} >> /Contents ${body} 0 R >>`))
  }
  let form = ''
  if (signed) {
    const sig = add('<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 100 150 50] /Contents <0000> >>')
    const field = add(`<< /FT /Sig /T (Approval) /V ${sig} 0 R >>`)
    form = ` /AcroForm << /SigFlags 3 /Fields [${field} 0 R] >>`
  }
  objects[0] = `<< /Type /Catalog /Pages 2 0 R${form} >>`
  objects[1] = `<< /Type /Pages /Count ${pages.length} /Kids [${pages.map(id => `${id} 0 R`).join(' ')}] >>`
  const parts = [Buffer.from('%PDF-1.7\n%\xe2\xe3\xcf\xd3\n', 'latin1')]
  let offset = parts[0].length
  const xref = ['0000000000 65535 f \n']
  for (const [index, object] of objects.entries()) {
    xref.push(`${String(offset).padStart(10, '0')} 00000 n \n`)
    const part = Buffer.concat([Buffer.from(`${index + 1} 0 obj\n`), Buffer.from(object), Buffer.from('\nendobj\n')])
    parts.push(part)
    offset += part.length
  }
  parts.push(Buffer.from(`xref\n0 ${xref.length}\n${xref.join('')}trailer\n<< /Size ${xref.length} /Root 1 0 R >>\nstartxref\n${offset}\n%%EOF\n`))
  return Buffer.concat(parts)
}

// This function is sent directly to Playwright by both the regression suite and the benchmark.
export async function readPdf({ base64, query, render = false }) {
  const { PdfEngine } = window.ocrApi
  const engine = await PdfEngine.create()
  try {
    const bytes = Uint8Array.fromBase64(base64)
    const { pages } = await engine.call('open', [bytes.buffer, ''])
    const results = []
    for (const { key } of pages) {
      const runs = await engine.call('text', [key])
      const stats = await engine.call('pageTextStats', [key])
      const annotations = await engine.call('annotations', [key])
      const hits = query ? await engine.call('search', [key, query, false]) : []
      let pixelHash
      if (render) {
        const bitmap = await engine.call('render', [key, 0.75, null])
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height)
        const ctx = canvas.getContext('2d')
        for (const band of bitmap.bands) {
          if (band.image) {
            ctx.drawImage(band.image, 0, band.y)
            band.image.close()
          } else {
            ctx.putImageData(new ImageData(new Uint8ClampedArray(band.data), bitmap.width, band.height), 0, band.y)
          }
        }
        const hash = await crypto.subtle.digest('SHA-256', ctx.getImageData(0, 0, bitmap.width, bitmap.height).data)
        pixelHash = new Uint8Array(hash).toBase64()
      }
      results.push({
        text: runs.map(run => run.text).join(' '),
        chars: stats.charCount,
        hits,
        links: annotations.flatMap(annot => annot.link ? [annot.link] : []),
        pixelHash,
      })
    }
    return results
  } finally {
    engine.destroy()
  }
}

const normalize = text => text.replace(/[\p{Bidi_Control}]/gu, '').replace(/\s+/gu, ' ').trim()
const base64 = bytes => Buffer.from(bytes).toString('base64')

async function ocrPdf(page, bytes, langs, options = {}) {
  return page.evaluate(async ({ encoded, langs, options }) => {
    const bytes = Uint8Array.fromBase64(encoded)
    const result = await window.ocrApi.pdfOcr.ocrPdfDocument({ pdfBytes: bytes, langs, ...options })
    return { ...result, searchablePdfBytes: result.searchablePdfBytes.toBase64(), input: bytes.toBase64() }
  }, { encoded: base64(bytes), langs, options })
}

async function verifyTampering(page, assets) {
  const result = await page.evaluate(async url => {
    const { packs, catalog } = window.ocrApi
    const failure = async fn => {
      try { await fn() } catch (error) { return error.message }
    }
    window.__ocrPackBaseUrl = url + '/corrupt'
    const downloadError = await failure(() => packs.downloadPack('heb'))
    const missingError = await failure(() => packs.ensurePacks(['heb']))
    const { installed } = await packs.refreshPacks()
    const { size } = catalog.getPackMeta('heb')
    const db = await new Promise((resolve, reject) => {
      const request = indexedDB.open('writan-ocr-packs', 1)
      request.onsuccess = () => resolve(request.result)
      request.onerror = () => reject(request.error)
    })
    const write = async operation => new Promise((resolve, reject) => {
      const transaction = db.transaction('packs', 'readwrite')
      operation(transaction.objectStore('packs'))
      transaction.oncomplete = resolve
      transaction.onerror = () => reject(transaction.error)
    })
    try {
      // Also verify the read-time digest check independently of download validation.
      await write(store => store.put(new ArrayBuffer(size), 'heb'))
      const cachedError = await failure(() => packs.ensurePacks(['heb']))
      return { downloadError, missingError, installed, cachedError }
    } finally {
      await write(store => store.delete('heb'))
      db.close()
      window.__ocrPackBaseUrl = url + '/packs'
    }
  }, assets.url)
  assert.match(result.downloadError, /failed verification/i)
  assert.match(result.missingError, /download.*first/i)
  // English ships with the app.
  assert.deepEqual(result.installed.map(pack => [pack.code, !!pack.builtIn]), [['eng', true]])
  assert.match(result.cachedError, /damaged/i)
  assert.deepEqual(assets.packRequests, ['/corrupt/heb.traineddata'])
  console.log('PASS  Same-length SHA-256 tampering rejected on download and IndexedDB read; English is built in')
}

export async function setFakeEngine(page, enabled) {
  await page.evaluate(({ enabled, phrase }) => {
    if (!enabled) {
      delete window.__ocrFakeEngine
      return
    }
    window.__ocrFakeEngine = input => {
      const bbox = { x: input.width * 0.08, y: input.height * 0.25, w: input.width * 0.78, h: input.height * 0.08 }
      const word = { text: phrase, bbox, confidence: 99, rtl: false, fontSizePx: bbox.h }
      const line = { ...word, words: [word] }
      const paragraph = { ...word, lines: [line] }
      return {
        pageIndex: input.pageIndex, widthPx: input.width, heightPx: input.height,
        widthPt: input.widthPt, heightPt: input.heightPt, text: phrase, meanConfidence: 99,
        detectedRotation: 0, elapsedMs: 0,
        blocks: [{ bbox, confidence: 99, paragraphs: [paragraph] }],
      }
    }
  }, { enabled, phrase: english[0] })
}

async function verifyPdfProtection(page, scan) {
  const signed = scanPdf([scan], { signed: true })
  const encrypted = await readFile(path.join(root, 'tests/stress/corpus/files/irs-fw9-password.pdf'))
  const rejected = await page.evaluate(async ({ signed, encrypted }) => {
    const { pdfOcr } = window.ocrApi
    const run = async (encoded, options) => {
      const bytes = Uint8Array.fromBase64(encoded)
      try {
        await pdfOcr.ocrPdfDocument({ pdfBytes: bytes, langs: ['eng'], ...options })
        throw new Error('Unexpected OCR success')
      } catch (error) {
        return { error: error.message, unchanged: encoded === bytes.toBase64() }
      }
    }
    return {
      signed: await run(signed),
      encrypted: await run(encrypted, { allowSigned: true }),
    }
  }, { signed: base64(signed), encrypted: base64(encrypted) })
  assert.match(rejected.signed.error, /digital signature/i)
  assert.match(rejected.encrypted.error, /encrypt/i)
  assert(rejected.signed.unchanged && rejected.encrypted.unchanged)
  const output = await ocrPdf(page, signed, ['eng'], { allowSigned: true })
  assert.deepEqual(output.inspection.signatureFields, ['Approval'])
  assert.equal(output.input, base64(signed))
  assert.notEqual(output.searchablePdfBytes, output.input)
  const [result] = await page.evaluate(readPdf, { base64: output.searchablePdfBytes, query: 'quick brown fox' })
  assert(result.hits.length > 0)
  console.log('PASS  Signed input requires opt-in, opt-in yields a separate searchable copy, and encrypted input is rejected')
  const remainingSignature = await page.evaluate(async encoded =>
    (await window.ocrApi.pdfOcr.inspectPdfDocument(Uint8Array.fromBase64(encoded))).hasDigitalSignature, output.searchablePdfBytes)
  assert.equal(remainingSignature, false, 'allowed searchable copy should not retain digital signature fields')
  console.log('PASS  Allowed searchable copy strips digital signature fields')
}

function structuredPages() {
  const box = (x, y, w, h) => ({ x, y, w, h })
  const line = (text, y, h = 20, words = []) => ({
    text, bbox: box(20, y, 260, h), confidence: 99, rtl: /\p{Script=Hebrew}/u.test(text), words,
  })
  const paragraph = (...lines) => ({
    text: lines.map(line => line.text).join('\n'), bbox: lines[0].bbox, confidence: 99,
    rtl: lines.some(line => line.rtl), lines,
  })
  const block = (bbox, ...paragraphs) => ({ bbox, confidence: 99, paragraphs })
  const column = (text, x, y) => block(box(x, y, 240, 45), paragraph(line(text, y)))
  const row = (y, ...cells) => line(cells.join(' '), y, 20, cells.map((text, index) => ({
    text, bbox: box(20 + index * 130, y, 35, 20), confidence: 99, rtl: false, fontSizePx: 20,
  })))
  const ocrPage = (pageIndex, blocks) => ({
    pageIndex, widthPx: 600, heightPx: 800, text: '', meanConfidence: 99, detectedRotation: 0, elapsedMs: 0, blocks,
  })
  // Deliberately shuffle pages and columns to exercise reconstruction order.
  return [
    ocrPage(1, [block(box(10, 0, 550, 260),
      paragraph(line('• Alpha', 0), line('• Beta', 24)),
      paragraph(line('(3) Gamma', 50), line('(4) Delta', 74)),
      paragraph(row(100, 'Name', 'City', 'Score'), row(124, 'Noah', 'Roma', '123'), row(148, 'Liam', 'Paris', '456')),
      paragraph(line('שלום עולם', 185)),
    )]),
    ocrPage(0, [
      block(box(10, 0, 550, 40), paragraph(line('Field notes', 0, 38))),
      column('Right two', 320, 105), column('Left two', 20, 115),
      column('Right one', 320, 50), column('Left one', 20, 60),
    ]),
  ]
}

async function verifyReconstruction(page) {
  const result = await page.evaluate(async pages => {
    const { toDocx } = window.ocrApi
    const bytes = await toDocx.ocrResultsToDocx(pages, 'OCR field notes')
    const tab = window.__workspace.useWorkspace.getState().openUnsaved(bytes, 'OCR field notes.docx')
    return { plain: toDocx.ocrResultsToPlainText(pages), markdown: toDocx.ocrResultsToMarkdown(pages), tab }
  }, structuredPages())
  assert(result.plain.startsWith('Field notes'))
  assert.match(result.markdown, /^# Field notes/m)
  assert.match(result.markdown, /^- Alpha$/m)
  assert.match(result.markdown, /^3\. Gamma$/m)
  assert.match(result.markdown, /^---$/m)
  assert.match(result.markdown, /\u200fשלום עולם/u)
  for (const exported of [result.plain, result.markdown]) {
    for (const text of ['Left one', 'Right two', 'Name City Score', 'Liam Paris 456', 'שלום עולם']) assert(exported.includes(text))
  }
  assert(result.tab)
  await page.waitForFunction(tab => {
    const editor = window.__workspace.getEditor(tab)
    return editor && !String(editor.getRevision()).startsWith('undefined:')
  }, result.tab)
  const saved = await page.evaluate(async tab => (await window.__workspace.getEditor(tab).getBytes()).toBase64(), result.tab)
  const zip = await JSZip.loadAsync(Buffer.from(saved, 'base64'))
  const xml = await zip.file('word/document.xml').async('string')
  const numbering = await zip.file('word/numbering.xml').async('string')
  assert.match(xml, /<w:pStyle\b[^>]*w:val="Heading1"/)
  assert.match(xml, /Left one[\s\S]*Left two[\s\S]*Right one[\s\S]*Right two/)
  assert.match(xml, /<w:br\b[^>]*w:type="page"/)
  assert.equal((xml.match(/<w:numPr>/g) ?? []).length, 4)
  assert.match(numbering, /w:val="bullet"/)
  assert.match(numbering, /w:val="decimal"/)
  assert.match(xml, /<w:tbl>[\s\S]*<w:tblBorders>[\s\S]*<w:insideV\b[\s\S]*<\/w:tblBorders>[\s\S]*Name[\s\S]*City[\s\S]*Score[\s\S]*Liam[\s\S]*Paris[\s\S]*456[\s\S]*<\/w:tbl>/)
  const rtlParagraph = (xml.match(/<w:p[ >][\s\S]*?<\/w:p>/g) ?? []).find(p => p.includes('שלום עולם'))
  assert.match(rtlParagraph, /<w:bidi\b/)
  console.log('PASS  OCR plain text and Markdown, and Writan DOCX reopen/save with page breaks, column order, heading, lists, bordered table and RTL')
}

async function verifySearchablePdf(page, scan) {
  const pdf = scanPdf([scan])
  const query = 'quick brown fox'
  const [before] = await page.evaluate(readPdf, { base64: base64(pdf), query, render: true })
  assert.equal(before.chars, 0)
  assert.deepEqual(before.hits, [])
  const output = await ocrPdf(page, pdf, ['eng'])
  assert.deepEqual(output.recognizedPages, [0])
  assert.deepEqual(output.skippedPages, [])
  assert.equal(output.input, base64(pdf), 'OCR must leave the original input buffer intact')
  const outputBytes = Buffer.from(output.searchablePdfBytes, 'base64')
  assert(outputBytes.length > pdf.length)
  assert.deepEqual(outputBytes.subarray(0, pdf.length), pdf, 'Incremental injection must preserve every original byte')
  assert(outputBytes.includes(Buffer.from(scan.jpeg)), 'JPEG XObject data must remain byte-identical')
  const [after] = await page.evaluate(readPdf, { base64: output.searchablePdfBytes, query, render: true })
  assert(normalize(after.text).includes(english[0]), after.text)
  assert(after.hits.length > 0 && after.hits[0].length > 0)
  assert.equal(after.pixelHash, before.pixelHash, 'The searchable text layer must leave rendered pixels identical')
  console.log('PASS  PDFium extracts and searches injected English text with identical source bytes, JPEG stream and rendered pixels')

  const digital = 'Existing digital text on page one'
  const hybrid = await ocrPdf(page, scanPdf([scan], { digital }), ['eng'], { skipPagesWithText: true })
  assert.deepEqual(hybrid.skippedPages, [0])
  assert.deepEqual(hybrid.recognizedPages, [1])
  const [first, second] = await page.evaluate(readPdf, { base64: hybrid.searchablePdfBytes, query })
  assert.equal(normalize(first.text), digital)
  assert.deepEqual(first.hits, [])
  assert(second.hits.length > 0)
  console.log('PASS  Hybrid OCR skips digital page zero and recognizes scanned page one')
}

async function verifyAutomaticRecognition(page, scan, real) {
  // A digital page, a blank page, a plain scan, and a scan with a line of real text at the bottom.
  const footer = 'Downloaded from the archive on 2 March 2024 for private study only'
  const pdf = scanPdf([scan], { digital: 'Existing digital text on page one', blank: 1 })
  const withFooter = scanPdf([scan, scan], { footer })
  const calls = await page.evaluate(() => {
    window.ocrCalls = 0
    const fake = window.__ocrFakeEngine
    if (fake) window.__ocrFakeEngine = (...args) => (window.ocrCalls++, fake(...args))
    return window.ocrCalls
  })
  assert.equal(calls, 0)
  // How much of each page is picture, as the scan check measures it.
  const coverage = await page.evaluate(async encoded => {
    const engine = await window.ocrApi.PdfEngine.create()
    try {
      const out = []
      for (const bytes of encoded) {
        const { pages } = await engine.call('open', [Uint8Array.fromBase64(bytes).buffer, ''])
        out.push((await engine.call('pageTextStats', [pages[0].key])).imageCoverage)
      }
      return out
    } finally {
      engine.destroy()
    }
  }, [scanPdf([scan]), scanPdf([scan], { wrapped: true }), scanPdf([scan], { art: true }), scanPdf([scan], { strips: true })].map(base64))
  assert(coverage[0] >= 0.5 && coverage[1] >= 0.5 && coverage[3] >= 0.5, 'A scan, also through a scaled form on a moved page box or in strips, is mostly picture: ' + coverage)
  assert(coverage[2] < 0.01, 'A small picture beside a drawing is not: ' + coverage[2])
  const recognizedLayers = () => page.evaluate(() =>
    [...document.querySelectorAll('.text-layer')].filter(layer => /quick/i.test(layer.textContent ?? '')).length)
  const closeAll = () => page.evaluate(async () => {
    const { tabs, closeTab } = window.__workspace.useWorkspace.getState()
    for (const tab of [...tabs].reverse()) await closeTab(tab.id)
  })
  const open = async (bytes, name) => {
    const tabId = await page.evaluate(({ encoded, name }) =>
      window.__workspace.useWorkspace.getState().openUnsaved(Uint8Array.fromBase64(encoded), name), { encoded: base64(bytes), name })
    assert.ok(tabId)
    return tabId
  }
  try {
    // Nothing to ask: the scan's words land in the page's text layer on their own.
    await open(pdf, 'mixed.pdf')
    const pageNumber = page.getByRole('textbox', { name: 'Page number' }).locator('visible=true')
    await pageNumber.fill('3')
    await pageNumber.press('Enter')
    await page.waitForFunction(() => [...document.querySelectorAll('.text-layer')].some(layer => /quick/i.test(layer.textContent ?? '')), null, { timeout: 60_000 })
    assert.equal(await recognizedLayers(), 1, 'Only the scanned page gets recognized text')
    if (!real) assert.equal(await page.evaluate(() => window.ocrCalls), 1, 'The digital and blank pages are not recognized')
    assert.equal(await page.locator('[data-testid="pdf-ocr-banner"], [data-testid="pdf-ocr-panel"]').count(), 0, 'No banner or dialog')

    // A scan drawn through a scaled form XObject, on a page whose box doesn't start at 0, 0.
    await closeAll()
    await open(scanPdf([scan], { wrapped: true }), 'wrapped.pdf')
    await page.waitForFunction(() => [...document.querySelectorAll('.text-layer')].some(layer => /quick/i.test(layer.textContent ?? '')), null, { timeout: 30_000 })

    await open(withFooter, 'library-copy.pdf')
    await page.waitForFunction(() => [...document.querySelectorAll('.text-layer')].filter(layer => /Downloaded from/.test(layer.textContent ?? '') && /quick/i.test(layer.textContent ?? '')).length === 2, null, { timeout: 60_000 })
    // The footer follows the page's words, so a selection dragged into it takes only what it crosses.
      const footers = await page.evaluate(() => [...document.querySelectorAll('.text-layer')].map(layer => layer.textContent ?? '').filter(text => /Downloaded from/.test(text)))
    assert(footers.length >= 2 && footers.every(text => /(?:dog|lunch)\.?\s*Downloaded from/.test(text)), 'Footer text comes after the recognized words: ' + JSON.stringify(footers))
    await page.waitForFunction(() => !/Reading scans/.test(document.body.innerText), null, { timeout: 60_000 })

    // ⌘F / Ctrl+F finds the recognized words like any other text.
    await page.locator('.pdf-container:visible').focus()
    await page.keyboard.press(process.platform === 'darwin' ? 'Meta+F' : 'Control+F')
    const find = page.getByRole('textbox', { name: 'Find' })
    await find.waitFor()
    await find.fill('Quick Brown')
    await page.waitForFunction(() => /^1 of 2$/.test(document.querySelector('.find-bar .count')?.textContent ?? ''), null, { timeout: 30_000 })
    assert.equal(await page.locator('.search-hit').count() >= 2, true, 'Matches are highlighted on both scans')
    // Enter goes to the next match and Shift+Enter back.
    await find.press('Enter')
    await page.waitForFunction(() => /^2 of 2$/.test(document.querySelector('.find-bar .count')?.textContent ?? ''))
    await find.press('Shift+Enter')
    await page.waitForFunction(() => /^1 of 2$/.test(document.querySelector('.find-bar .count')?.textContent ?? ''))
    // The scan's line of real text is found once, not again from its picture.
    await find.fill('private study')
    await page.waitForFunction(() => /^1 of 2$/.test(document.querySelector('.find-bar .count')?.textContent ?? ''), null, { timeout: 30_000 })
    await find.fill('Quick Brown')
    await page.waitForFunction(() => /^1 of 2$/.test(document.querySelector('.find-bar .count')?.textContent ?? ''))
    if (process.env.OCR_SHOT) await page.locator('.workbench:visible').screenshot({ path: process.env.OCR_SHOT })

    const state = await page.evaluate(() => {
      const s = window.__workspace.useWorkspace.getState()
      const tab = s.tabs.find((t) => t.id === s.activeId)
      return { dirty: tab.dirty, bytes: tab.bytes.toBase64() }
    })
    assert.equal(state.dirty, false, 'Recognizing text leaves the document unchanged')
    assert.equal(state.bytes, base64(withFooter))
    console.log('PASS  Scanned pages are recognized automatically, found with ⌘F, and the file is left unchanged')
  } finally {
    await closeAll()
  }
}

async function verifyImageAndRtl(page, real) {
  const image = await page.evaluate(async () => {
    const { pdfOcr } = window.ocrApi
    const result = await pdfOcr.ocrImageBytes(window.ocrScans.english.jpeg, 'scan.jpg', ['eng'])
    return (await result.buildSearchablePdf()).toBase64()
  })
  const [englishPage] = await page.evaluate(readPdf, { base64: image, query: 'quick brown fox' })
  assert(normalize(englishPage.text).includes(english[0]), englishPage.text)
  assert(englishPage.hits.length > 0)
  console.log('PASS  Image-to-searchable-PDF with PDFium extraction and search')
  if (!real) return

  const scan = await getScan(page, 'hebrew')
  const output = await ocrPdf(page, scanPdf([scan]), ['heb', 'eng'])
  const [hebrewPage] = await page.evaluate(readPdf, { base64: output.searchablePdfBytes, query: 'שלום' })
  const words = output.pages[0].blocks.flatMap(block => block.paragraphs.flatMap(para => para.lines.flatMap(line => line.words)))
  assert(words.some(word => word.text === 'שלום' && word.rtl), JSON.stringify(words))
  assert(normalize(hebrewPage.text).includes('שלום עולם'), hebrewPage.text)
  assert(hebrewPage.hits.length > 0)
  console.log('PASS  Scanned PDF with heb+eng yields searchable Hebrew RTL text')
}

async function verifyTransformsAndCancellation(page, scan) {
  const transformed = await page.evaluate(async () => {
    const { ocrClient, preprocess } = window.ocrApi
    const client = new ocrClient.OcrClient()
    const report = {}
    try {
      for (const key of ['rotated', 'skewed']) {
        const { width, height, data } = window.ocrScans[key]
        const result = await client.recognize({ width, height, data }, { langs: ['eng'] })
        const prepared = preprocess.preprocessImage(data, width, height, result.detectedRotation)
        const left = prepared.mapPoint(50, 100)
        const right = prepared.mapPoint(450, 100)
        const originalAngle = Math.atan2(right.y - left.y, right.x - left.x) * 180 / Math.PI
        report[key] = { ...result, originalAngle }
      }
      return report
    } finally {
      client.dispose()
    }
  })
  for (const [name, degrees] of [['rotated', 90], ['skewed', -6]]) {
    const result = transformed[name]
    assert(normalize(result.text).includes(english[0]), `${name}: ${result.text}`)
    assert.equal(result.detectedRotation, name === 'rotated' ? 270 : 0)
    const line = result.blocks[0].paragraphs[0].lines[0]
    const first = line.words[0].bbox
    const last = line.words.at(-1).bbox
    const radians = degrees * Math.PI / 180
    const dx = last.x + last.w / 2 - first.x - first.w / 2
    const dy = last.y + last.h / 2 - first.y - first.h / 2
    assert(dx * Math.cos(radians) + dy * Math.sin(radians) > 650, 'Words must be ordered along the original scan line')
    assert(Math.abs(-dx * Math.sin(radians) + dy * Math.cos(radians)) < 12, 'Mapped words must share an upright baseline')
    for (const { bbox } of line.words) {
      assert(bbox.x >= 0 && bbox.y >= 0 && bbox.x + bbox.w <= result.widthPx && bbox.y + bbox.h <= result.heightPx)
    }

    const scanInfo = await getScan(page, name)
    const targetWord = line.words.find(w => /quick/i.test(w.text))
    assert(targetWord, `Expected word "quick" in ${name} OCR result`)
    const injected = await ocrPdf(page, scanPdf([scanInfo]), ['eng'])
    const [pageInfo] = await page.evaluate(readPdf, { base64: injected.searchablePdfBytes, query: 'quick' })
    const matchBoxes = pageInfo.hits.flat()
    assert(matchBoxes.length > 0, `PDFium search missed "quick" on ${name} PDF`)
    const hitBox = matchBoxes[0]
    const scaleX = (scanInfo.width * 72 / 96) / result.widthPx
    const scaleY = (scanInfo.height * 72 / 96) / result.heightPx
    if (name === 'rotated') {
      assert(hitBox.h > hitBox.w, `rotated highlight must be vertical: ${JSON.stringify(hitBox)}`)
      assert(hitBox.h >= targetWord.bbox.h * scaleY * 0.45, `rotated highlight extent too short: ${JSON.stringify(hitBox)} vs bbox h=${targetWord.bbox.h * scaleY}`)
    } else {
      assert(hitBox.w > hitBox.h, `skewed highlight must remain wider than tall: ${JSON.stringify(hitBox)}`)
      assert(hitBox.w >= targetWord.bbox.w * scaleX * 0.45, `skewed highlight extent too short: ${JSON.stringify(hitBox)} vs bbox w=${targetWord.bbox.w * scaleX}`)
    }
    const wordCenterX = (targetWord.bbox.x + targetWord.bbox.w / 2) * scaleX
    const wordCenterY = (targetWord.bbox.y + targetWord.bbox.h / 2) * scaleY
    const hitCenterX = hitBox.x + hitBox.w / 2
    const hitCenterY = hitBox.y + hitBox.h / 2
    assert(Math.abs(hitCenterX - wordCenterX) < 18 && Math.abs(hitCenterY - wordCenterY) < 18,
      `${name} highlight center (${hitCenterX.toFixed(1)},${hitCenterY.toFixed(1)}) drifted from OCR word center (${wordCenterX.toFixed(1)},${wordCenterY.toFixed(1)})`)
  }
  assert(Math.abs(transformed.skewed.originalAngle + 6) < 1, 'Deskewed horizontal coordinates should map back to -6°')
  console.log('PASS  90° rotation and -6° skew recover readable text, aligned PDF search highlights, and mapped coordinates')

  const concurrent = await page.evaluate(async encoded => {
    const bytes = Uint8Array.fromBase64(encoded)
    const { pdfEngineClient } = window.ocrApi.pdfClient
    const acA = new AbortController()
    const pA = pdfEngineClient.inspectPdf(bytes, acA.signal).catch(err => ({ aborted: err.name }))
    const pB = pdfEngineClient.inspectPdf(bytes)
    acA.abort()
    const [resA, resB] = await Promise.all([pA, pB])
    return { resA, resB }
  }, base64(scanPdf([scan])))
  assert.equal(concurrent.resA.aborted, 'AbortError')
  assert.equal(concurrent.resB.pageCount, 1)
  console.log('PASS  Aborting one PDF client call leaves concurrent calls alive')

  const report = await page.evaluate(async encoded => {
    const { pdfOcr } = window.ocrApi
    const pdfBytes = Uint8Array.fromBase64(encoded)
    const controller = new AbortController()
    const options = { pdfBytes, langs: ['eng'] }
    window.ocrWorkerMessages.length = 0
    window.onOcrWorkerPost = ({ kind }) => {
      if (kind === 'recognize') controller.abort()
    }
    let errorName
    try {
      await pdfOcr.ocrPdfDocument({ ...options, signal: controller.signal })
    } catch (error) {
      errorName = error.name
    } finally {
      delete window.onOcrWorkerPost
    }
    const aborted = window.ocrWorkerMessages.map(message => message.kind)
    window.ocrWorkerMessages.length = 0
    const retried = await pdfOcr.ocrPdfDocument(options)
    return { aborted, errorName, recognizedPages: retried.recognizedPages, reused: window.ocrWorkerMessages.map(message => message.kind) }
  }, base64(scanPdf([scan, scan])))
  assert.equal(report.errorName, 'AbortError')
  assert.deepEqual(report.aborted, ['initialize', 'ready', 'recognize'])
  assert.deepEqual(report.recognizedPages, [0, 1])
  assert.deepEqual(report.reused, ['initialize', 'ready', 'recognize', 'result', 'recognize', 'result'])
  console.log('PASS  Cancellation after recognition dispatch and a two-page retry with one worker initialization')
}

async function verifyOfflineReuse({ page, context, assets, requests }) {
  const installed = await page.evaluate(async () => {
    const { packs } = window.ocrApi
    return (await packs.ensurePacks(['eng'])).map(({ code, bytes }) => ({ code, length: bytes.length }))
  })
  assert.deepEqual(installed.map(pack => pack.code), ['eng'])
  assert(installed[0].length > 0)
  await assets.close()
  const beforeRestart = requests.length
  const warmed = await page.evaluate(async () => {
    const { ocrClient } = window.ocrApi
    window.offlineOcr = new ocrClient.OcrClient()
    return (await window.offlineOcr.recognize(window.ocrScans.english, { langs: ['eng'] })).text
  })
  assert(normalize(warmed).includes(english[0]), warmed)
  assert.deepEqual(requests.slice(beforeRestart).filter(url => url.startsWith(assets.url)), [])
  const beforeOffline = requests.length
  await context.setOffline(true)
  try {
    const text = await page.evaluate(async () => {
      const { packs } = window.ocrApi
      await packs.ensurePacks(['eng'])
      try {
        return (await window.offlineOcr.recognize(window.ocrScans.english, { langs: ['eng'] })).text
      } finally {
        window.offlineOcr.dispose()
      }
    })
    assert(normalize(text).includes(english[0]), text)
    assert.deepEqual(requests.slice(beforeOffline), [], 'Fully offline recognition must make zero network requests')
  } finally {
    await context.setOffline(false)
  }
  console.log('PASS  New OCR worker reuses IndexedDB after pack server shutdown, then recognizes fully disconnected with zero requests')
}

export async function getScan(page, name) {
  const scan = await page.evaluate(name => {
    const { jpeg, width, height } = window.ocrScans[name]
    return { jpeg: jpeg.toBase64(), width, height }
  }, name)
  return { ...scan, jpeg: Buffer.from(scan.jpeg, 'base64') }
}

async function main() {
  await withOcrBrowser(async session => {
    const { page, assets, requests } = session
    console.log('PASS  Home screen has zero Tesseract or office PDF WASM requests')
    await verifyTampering(page, assets)
    const real = existsSync(tessdataDir)
    if (real) {
      requirePacks(['eng', 'heb'])
      await downloadPacks(page, ['eng', 'heb'])
    } else {
      console.log('[SKIP real-pack] No tessdata directory. Running PDF structure, search and protection with the fake engine.')
      console.log('[SKIP real-pack] Hebrew Tesseract, orientation, cancellation/worker reuse and offline cached recognition require real packs.')
      await setFakeEngine(page, true)
    }
    await makeScans(page, {
      english: { lines: english },
      ...(real && {
        hebrew: { lines: hebrew, font: 'Hebrew', rtl: true },
        rotated: { lines: english, degrees: 90 },
        skewed: { lines: english, degrees: -6 },
      }),
    })
    const scan = await getScan(page, 'english')
    await verifySearchablePdf(page, scan)
    await verifyAutomaticRecognition(page, scan, real)
    await verifyPdfProtection(page, scan)
    await verifyReconstruction(page)
    await verifyImageAndRtl(page, real)
    if (real) {
      await verifyTransformsAndCancellation(page, scan)
      await verifyOfflineReuse(session)
      assert(requests.some(url => /tesseract-core.*\.wasm$/.test(url)), 'Real OCR must load the bundled Tesseract WASM')
    } else {
      assert(!requests.some(url => /tesseract-core/.test(url)), 'The fake engine must not load Tesseract')
    }
  })
  console.log('PASS  All OCR requests stayed on loopback or used local browser URLs')
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main()
