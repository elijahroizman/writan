export interface OcrBBox { x: number; y: number; w: number; h: number }

export interface OcrWord {
  text: string
  bbox: OcrBBox
  baseline?: number
  baselineLine?: { x0: number; y0: number; x1: number; y1: number }
  orientedHeightPx?: number
  confidence: number
  rtl: boolean
  fontSizePx: number
}

export interface OcrLine {
  text: string
  bbox: OcrBBox
  baseline?: number
  confidence: number
  rtl: boolean
  words: OcrWord[]
}

export interface OcrParagraph {
  text: string
  bbox: OcrBBox
  confidence: number
  rtl: boolean
  lines: OcrLine[]
}

export interface OcrBlock {
  bbox: OcrBBox
  confidence: number
  paragraphs: OcrParagraph[]
}

export type OcrRotation = 0 | 90 | 180 | 270

export interface OcrPageResult {
  pageIndex: number
  widthPx: number
  heightPx: number
  widthPt?: number
  heightPt?: number
  text: string
  meanConfidence: number
  detectedRotation: OcrRotation
  elapsedMs: number
  blocks: OcrBlock[]
}

export interface OcrDocumentInput {
  title?: string
  pages: OcrPageResult[]
  /** The page to set the document on, in points. */
  page?: { width: number; height: number }
}

export interface OcrPageInput {
  data: Uint8ClampedArray | Uint8Array
  width: number
  height: number
  /** Pixel format of `data`: `4` for RGBA (default), `1` for packed 8-bit grayscale. */
  channels?: 1 | 4
  /** Transfer `data.buffer` straight to the OCR worker without cloning when unshared. */
  transfer?: boolean
  pageIndex?: number
  widthPt?: number
  heightPt?: number
}

export interface OcrRecognizeOptions {
  langs?: readonly string[]
  signal?: AbortSignal
}

export interface OcrInstalledPack {
  code: string
  size: number
  sha256Verified: boolean
  /** Ships with the app and can't be removed. */
  builtIn?: boolean
}

export interface OcrPackInventory {
  installed: OcrInstalledPack[]
  totalBytes: number
}

export interface OcrPackProgress {
  code: string
  received: number
  total: number
}
