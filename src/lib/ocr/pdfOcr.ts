import { PdfEngine, Priority } from '../../editors/pdf/engine/client'
import type { GrayBitmap, PageInfo } from '../../editors/pdf/engine/protocol'
import { pdfEngineClient, type PdfInspection } from '../pdf/client'
import { decodeImageToRgba, exportImageToPdf, type OcrLayerInput } from '../pdf/exporter'
import { loadFontsForOcrLayers } from '../pdf/fonts'
import { OcrClient } from './client'
import type { OcrPageResult } from './types'

export const SCANNED_CHAR_THRESHOLD = 10

export interface PdfOcrProgress {
  done: number
  total: number
  pageIndex: number
  phase: 'inspecting' | 'recognizing' | 'injecting'
}

export interface PdfOcrOptions {
  pdfBytes: Uint8Array
  pageIndices?: number[]
  langs: string[]
  skipPagesWithText?: boolean
  /** False to only recognize the pages; `searchablePdfBytes` is then the original PDF. */
  searchable?: boolean
  allowSigned?: boolean
  password?: string
  engine?: PdfEngine
  pagesInfo?: PageInfo[]
  onProgress?: (progress: PdfOcrProgress) => void
  signal?: AbortSignal
}

export interface PdfOcrResult {
  searchablePdfBytes: Uint8Array
  pages: OcrPageResult[]
  skippedPages: number[]
  recognizedPages: number[]
  inspection: PdfInspection
}

export interface ImageOcrResult {
  page: OcrPageResult
  buildSearchablePdf: (signal?: AbortSignal) => Promise<Uint8Array>
}

const PX_TO_PT = 72 / 96

function pageToOcrLayer(page: OcrPageResult, pageWidthPt: number, pageHeightPt: number): OcrLayerInput {
  const scaleX = page.widthPx > 0 ? pageWidthPt / page.widthPx : 1
  const scaleY = page.heightPx > 0 ? pageHeightPt / page.heightPx : 1
  const words: OcrLayerInput['words'] = []
  for (const block of page.blocks) {
    for (const para of block.paragraphs) {
      for (const line of para.lines) {
        for (const word of line.words) {
          if (!word.text.trim()) continue
          const bl = word.baselineLine
          let orientedHPt: number | undefined
          if (word.orientedHeightPx !== undefined) {
            const vx = bl ? bl.x1 - bl.x0 : word.bbox.w
            const vy = bl ? bl.y1 - bl.y0 : 0
            const len = Math.hypot(vx, vy) || 1
            orientedHPt = Math.max(0.5, word.orientedHeightPx * Math.hypot((vy / len) * scaleX, (vx / len) * scaleY))
          }
          words.push({
            text: word.text,
            xPt: word.bbox.x * scaleX,
            yPt: word.bbox.y * scaleY,
            wPt: Math.max(0.5, word.bbox.w * scaleX),
            hPt: Math.max(0.5, word.bbox.h * scaleY),
            baselineYPt: word.baseline !== undefined ? word.baseline * scaleY : undefined,
            baselineX0Pt: bl ? bl.x0 * scaleX : undefined,
            baselineY0Pt: bl ? bl.y0 * scaleY : undefined,
            baselineX1Pt: bl ? bl.x1 * scaleX : undefined,
            baselineY1Pt: bl ? bl.y1 * scaleY : undefined,
            orientedHPt,
            rtl: word.rtl,
          })
        }
      }
    }
  }
  return {
    pageIndex: page.pageIndex,
    pageWidthPt,
    pageHeightPt,
    words,
  }
}

export async function inspectPdfDocument(bytes: Uint8Array, signal?: AbortSignal): Promise<PdfInspection> {
  return await pdfEngineClient.inspectPdf(bytes, signal)
}

export async function ocrPdfDocument(opts: PdfOcrOptions): Promise<PdfOcrResult> {
  const { pdfBytes, langs, signal, onProgress } = opts
  const skipPagesWithText = opts.skipPagesWithText !== false
  signal?.throwIfAborted?.()

  const searchable = opts.searchable !== false
  onProgress?.({ done: 0, total: 1, pageIndex: 0, phase: 'inspecting' })
  // Only writing a searchable copy needs the file's structure; PDFium reads the pages of any PDF.
  const inspection: PdfInspection = searchable
    ? await inspectPdfDocument(pdfBytes, signal)
    : { pageCount: 0, encrypted: false, hasDigitalSignature: false, signatureFields: [], hasForms: false, hasOutlines: false, annotationCount: 0 }
  if (inspection.encrypted) {
    throw new Error('Encrypted PDFs cannot be processed by OCR.')
  }
  if (inspection.hasDigitalSignature && !opts.allowSigned) {
    throw new Error('This PDF contains a digital signature. Confirm before creating an unsigned searchable copy.')
  }

  let ownEngine: PdfEngine | undefined
  let engine = opts.engine
  let pagesInfo = opts.pagesInfo
  const ocrClient = new OcrClient()

  try {
    if (!engine) {
      ownEngine = await PdfEngine.create()
      const opened = await ownEngine.call('open', [pdfBytes.slice().buffer, opts.password ?? ''], { signal })
      engine = ownEngine
      pagesInfo = opened.pages
    }
    if (!pagesInfo?.length) {
      throw new Error('This PDF has no pages to recognise.')
    }

    const priority = opts.engine ? Priority.ocr : Priority.action
    const stats = await engine.call('documentTextStats', [], { priority, signal })
    const requested = opts.pageIndices?.length
      ? [...new Set(opts.pageIndices)].filter((i) => i >= 0 && i < pagesInfo!.length).sort((a, b) => a - b)
      : pagesInfo.map((_, i) => i)

    const skippedPages: number[] = []
    const toProcess: number[] = []
    for (const idx of requested) {
      const stat = stats[idx]
      if (skipPagesWithText && stat && stat.nonWhitespaceCount >= SCANNED_CHAR_THRESHOLD) {
        skippedPages.push(idx)
      } else {
        toProcess.push(idx)
      }
    }

    if (!toProcess.length) {
      return {
        searchablePdfBytes: pdfBytes.slice(),
        pages: [],
        skippedPages,
        recognizedPages: [],
        inspection,
      }
    }

    const renderPageGray = (pageIdx: number): Promise<GrayBitmap> => {
      const info = pagesInfo![pageIdx]
      const scale = Math.min(220 / 72, 2800 / Math.max(1, info.width, info.height))
      return engine!.call('renderGrayscale', [info.key, scale, null], { priority, signal })
    }

    const pages: OcrPageResult[] = []
    const layers: OcrLayerInput[] = []
    let nextRender: Promise<GrayBitmap> | undefined = renderPageGray(toProcess[0])
    try {
      for (let step = 0; step < toProcess.length; step++) {
        signal?.throwIfAborted?.()
        const pageIdx = toProcess[step]
        const info = pagesInfo[pageIdx]
        onProgress?.({ done: step, total: toProcess.length, pageIndex: pageIdx, phase: 'recognizing' })

        const gray = await nextRender!
        nextRender = step + 1 < toProcess.length ? renderPageGray(toProcess[step + 1]) : undefined
        nextRender?.catch(() => {})

        const recognized = await ocrClient.recognize(
          {
            data: new Uint8Array(gray.data),
            width: gray.width,
            height: gray.height,
            channels: 1,
            transfer: true,
            pageIndex: pageIdx,
            widthPt: info.width,
            heightPt: info.height,
          },
          { langs, signal },
        )
        pages.push(recognized)
        layers.push(pageToOcrLayer(recognized, info.width, info.height))
      }
    } catch (err) {
      nextRender?.catch(() => {})
      throw err
    }

    if (!searchable) return { searchablePdfBytes: pdfBytes, pages, skippedPages, recognizedPages: toProcess, inspection }
    onProgress?.({ done: toProcess.length, total: toProcess.length, pageIndex: toProcess[toProcess.length - 1], phase: 'injecting' })
    const fontPack = await loadFontsForOcrLayers(layers)
    const searchablePdfBytes = await pdfEngineClient.injectOcrTextLayer(pdfBytes, JSON.stringify(layers), fontPack, {
      signal,
      allowSigned: !!opts.allowSigned,
    })
    return {
      searchablePdfBytes,
      pages,
      skippedPages,
      recognizedPages: toProcess,
      inspection,
    }
  } finally {
    ocrClient.dispose()
    ownEngine?.destroy()
  }
}

export async function ocrImageBytes(
  bytes: Uint8Array,
  nameOrMime: string,
  langs: string[],
  signal?: AbortSignal,
): Promise<ImageOcrResult> {
  signal?.throwIfAborted?.()
  const rgba = await decodeImageToRgba(bytes, nameOrMime)
  const widthPt = Math.max(72, rgba.width * PX_TO_PT)
  const heightPt = Math.max(72, rgba.height * PX_TO_PT)
  const client = new OcrClient()
  try {
    const page = await client.recognize(
      {
        data: rgba.data,
        width: rgba.width,
        height: rgba.height,
        pageIndex: 0,
        widthPt,
        heightPt,
      },
      { langs, signal },
    )
    return {
      page,
      buildSearchablePdf: async (buildSignal?: AbortSignal) => {
        const layer = pageToOcrLayer(page, widthPt, heightPt)
        return await exportImageToPdf(bytes, nameOrMime, layer, buildSignal)
      },
    }
  } finally {
    client.dispose()
  }
}
