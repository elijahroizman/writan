// PDF to an editable Word document, as Word opens a PDF: each page's text with its fonts, sizes
// and colours, its pictures and tables, rebuilt into paragraphs in reading order. Pages that are
// scans have their text recognized. The layout comes from PDFium (`layout` in the PDF worker),
// and the document from office-wasm (`crates/office-wasm/src/pdf_docx.rs`).

import { PdfEngine, Priority } from '../editors/pdf/engine/client'
import type { LayoutPicture, PageInfo, PageLayout } from '../editors/pdf/engine/protocol'
import { isScannedPage } from '../editors/pdf/ocr'
import { loadEngine } from '../engine/load'
import type { OcrPageResult } from './ocr/types'

export interface PdfToDocxOptions {
  pdfBytes: Uint8Array
  password?: string
  title?: string
  signal?: AbortSignal
  onProgress?: (message: string) => void
  /** Recognizes the text of the given pages (scans) with the conversion's own PDF engine. */
  recognize?: (engine: PdfEngine, pages: PageInfo[], indices: number[]) => Promise<OcrPageResult[]>
}

interface PictureOut {
  x: number
  y: number
  w: number
  h: number
  offset: number
  length: number
  contentType: string
  figure: boolean
}

/** `ocr`: read off a picture (recognized, or a scan's invisible text layer). */
type FragmentOut = PageLayout['fragments'][number] & { ocr?: boolean }

interface PageOut {
  width: number
  height: number
  fragments: FragmentOut[]
  rules: PageLayout['rules']
  pictures: PictureOut[]
}

export async function pdfToDocx(opts: PdfToDocxOptions): Promise<Uint8Array> {
  const { signal, onProgress } = opts
  // A copy of its own: drawing the pictures alone switches the rest of each page off.
  const engine = await PdfEngine.create()
  try {
    onProgress?.('Opening PDF…')
    const opened = await engine.call('open', [opts.pdfBytes.slice().buffer, opts.password ?? ''], { signal })
    const pages = opened.pages
    if (!pages.length) throw new Error('This PDF has no pages to convert.')
    const stats = await engine.call('documentTextStats', [], { signal })
    const scans = pages.map((_, i) => !!stats[i] && isScannedPage(stats[i])).map((scan, i) => (scan ? i : -1)).filter((i) => i >= 0)

    // Every page a scan: the recognizer's own reconstruction reads them best.
    if (scans.length === pages.length && opts.recognize) {
      const results = await opts.recognize(engine, pages, scans)
      signal?.throwIfAborted()
      onProgress?.('Building Word document…')
      const { ocrResultsToDocx } = await import('./ocr/toDocx')
      return await ocrResultsToDocx(results, opts.title, { width: pages[0].width, height: pages[0].height })
    }

    const out: PageOut[] = []
    const media: Uint8Array[] = []
    let offset = 0
    for (let i = 0; i < pages.length; i++) {
      signal?.throwIfAborted()
      onProgress?.(pages.length > 1 ? `Reading page ${i + 1} of ${pages.length}…` : 'Reading the page…')
      const layout = await engine.call('layout', [pages[i].key, true], { signal, priority: Priority.action })
      const pictures: PictureOut[] = []
      for (const picture of layout.pictures) {
        const encoded = await encodePicture(picture)
        if (!encoded) continue
        pictures.push({ ...picture.box, offset, length: encoded.bytes.length, contentType: encoded.type, figure: picture.figure })
        media.push(encoded.bytes)
        offset += encoded.bytes.length
      }
      // A scan's text layer (an OCR'd PDF) is text read off the scan.
      const fragments = layout.fragments.map(({ invisible, ...f }) => (invisible ? { ...f, ocr: true } : f))
      out.push({ width: layout.width, height: layout.height, fragments, rules: layout.rules, pictures })
    }

    if (scans.length && opts.recognize) {
      const results = await opts.recognize(engine, pages, scans)
      for (const result of results) {
        const page = out[result.pageIndex]
        if (!page) continue
        // The page's own text stays; recognized words fill in where it has none.
        const own = page.fragments
        const free = (f: FragmentOut) => !own.some((o) => f.x + f.w / 2 >= o.x && f.x + f.w / 2 <= o.x + o.w && f.y + f.h / 2 >= o.y && f.y + f.h / 2 <= o.y + o.h)
        page.fragments = [...own, ...ocrFragments(result, page.width, page.height).filter(free)]
      }
    }
    signal?.throwIfAborted()

    onProgress?.('Building Word document…')
    const blob = new Uint8Array(offset)
    let at = 0
    for (const bytes of media) {
      blob.set(bytes, at)
      at += bytes.length
    }
    const wasm = await loadEngine()
    return wasm.pdfToDocx(JSON.stringify({ title: opts.title, pages: out }), blob)
  } finally {
    engine.destroy()
  }
}

/** Recognized words as layout fragments, one per word, in points. */
function ocrFragments(result: OcrPageResult, widthPt: number, heightPt: number): FragmentOut[] {
  const sx = result.widthPx > 0 ? widthPt / result.widthPx : 0.75
  const sy = result.heightPx > 0 ? heightPt / result.heightPx : 0.75
  const fragments: FragmentOut[] = []
  if (result.detectedRotation !== 0) {
    // Words on a turned scan are boxed on the picture as it lies; stack its lines instead.
    let y = 72
    for (const paragraph of result.blocks.flatMap((b) => b.paragraphs)) {
      for (const line of paragraph.lines) {
        const text = line.text.trim()
        if (!text) continue
        y += 14
        fragments.push({ x: 72, y: y - 9, w: widthPt - 144, h: 12, baseline: y, size: 11, rtl: line.rtl, ocr: true, spans: [{ text, x: 72, w: widthPt - 144, baseline: y, size: 11, bold: false, italic: false }] })
      }
      y += 10
    }
    return fragments
  }
  for (const line of result.blocks.flatMap((b) => b.paragraphs).flatMap((p) => p.lines)) {
    const lineHeight = line.bbox.h * sy
    for (const word of line.words) {
      const text = word.text.trim()
      if (!text) continue
      const x = word.bbox.x * sx
      const y = word.bbox.y * sy
      const w = word.bbox.w * sx
      const h = word.bbox.h * sy
      const baseline = (word.baseline ?? line.baseline ?? word.bbox.y + word.bbox.h) * sy
      const size = Math.max(4, Math.min(72, word.fontSizePx > 0 ? word.fontSizePx * sy : lineHeight / 1.2))
      fragments.push({ x, y, w, h, baseline, size, rtl: word.rtl, ocr: true, spans: [{ text, x, w, baseline, size, bold: false, italic: false }] })
    }
  }
  return fragments
}

/** A picture as JPEG (an opaque photo) or PNG (drawings and anything see-through), or nothing when it's blank. */
async function encodePicture(picture: LayoutPicture): Promise<{ bytes: Uint8Array; type: string } | null> {
  const { width, height } = picture
  const data = new Uint8ClampedArray(picture.data)
  let opaque = true
  let blank = true
  for (let i = 3; i < data.length; i += 4) {
    if (data[i] < 255) opaque = false
    if (data[i] > 0) blank = false
    if (!opaque && !blank) break
  }
  if (blank) return null
  const type = opaque && !picture.figure && width * height > 40_000 ? 'image/jpeg' : 'image/png'
  const image = new ImageData(data, width, height)
  let blob: Blob | null
  if (typeof OffscreenCanvas === 'function') {
    const canvas = new OffscreenCanvas(width, height)
    canvas.getContext('2d')!.putImageData(image, 0, 0)
    blob = await canvas.convertToBlob({ type, quality: 0.9 })
  } else {
    const canvas = document.createElement('canvas')
    canvas.width = width
    canvas.height = height
    canvas.getContext('2d')!.putImageData(image, 0, 0)
    blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, type, 0.9))
  }
  if (!blob) return null
  return { bytes: new Uint8Array(await blob.arrayBuffer()), type: blob.type || type }
}
