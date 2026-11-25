import { discardBitmap, PdfEngine, Priority } from '../../editors/pdf/engine/client'
import type { Bitmap, PageInfo, PageTextStats } from '../../editors/pdf/engine/protocol'
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
          words.push({
            text: word.text,
            xPt: word.bbox.x * scaleX,
            yPt: word.bbox.y * scaleY,
            wPt: Math.max(0.5, word.bbox.w * scaleX),
            hPt: Math.max(0.5, word.bbox.h * scaleY),
            baselineYPt: word.baseline !== undefined ? word.baseline * scaleY : undefined,
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

function bitmapToImageData(bitmap: Bitmap): { data: Uint8ClampedArray; width: number; height: number } {
  const { width, height, bands } = bitmap
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d')!
  for (const band of bands) {
    if (band.image) {
      ctx.drawImage(band.image, 0, band.y)
    } else if (band.data) {
      const clamped = new Uint8ClampedArray(band.data)
      ctx.putImageData(new ImageData(clamped, width, band.height), 0, band.y)
    }
  }
  const imgData = ctx.getImageData(0, 0, width, height)
  return { data: imgData.data, width, height }
}

export async function inspectPdfDocument(bytes: Uint8Array, signal?: AbortSignal): Promise<PdfInspection> {
  return await pdfEngineClient.inspectPdf(bytes, signal)
}

export function countScannedPages(stats: PageTextStats[]): number[] {
  return stats.filter((s) => s.nonWhitespaceCount < SCANNED_CHAR_THRESHOLD).map((s) => s.index)
}

export async function ocrPdfDocument(opts: PdfOcrOptions): Promise<PdfOcrResult> {
  const { pdfBytes, langs, signal, onProgress } = opts
  const skipPagesWithText = opts.skipPagesWithText !== false
  signal?.throwIfAborted?.()

  onProgress?.({ done: 0, total: 1, pageIndex: 0, phase: 'inspecting' })
  const inspection = await inspectPdfDocument(pdfBytes, signal)
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

    const stats = await engine.call('documentTextStats', [], { priority: Priority.action, signal })
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

    const pages: OcrPageResult[] = []
    const layers: OcrLayerInput[] = []
    for (let step = 0; step < toProcess.length; step++) {
      signal?.throwIfAborted?.()
      const pageIdx = toProcess[step]
      const info = pagesInfo[pageIdx]
      onProgress?.({ done: step, total: toProcess.length, pageIndex: pageIdx, phase: 'recognizing' })

      const scale = Math.min(220 / 72, 2800 / Math.max(1, info.width, info.height))
      const bitmap = await engine.call('render', [info.key, scale, null], { priority: Priority.action, signal })
      let imageData: { data: Uint8ClampedArray; width: number; height: number }
      try {
        imageData = bitmapToImageData(bitmap)
      } finally {
        discardBitmap(bitmap)
      }

      const recognized = await ocrClient.recognize(
        {
          data: imageData.data,
          width: imageData.width,
          height: imageData.height,
          pageIndex: pageIdx,
          widthPt: info.width,
          heightPt: info.height,
        },
        { langs, signal },
      )
      pages.push(recognized)
      layers.push(pageToOcrLayer(recognized, info.width, info.height))
    }

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
