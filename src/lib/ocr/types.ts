export interface OcrBBox { x: number; y: number; w: number; h: number }

export interface OcrWord {
  text: string
  bbox: OcrBBox
  baseline?: number
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
}

export interface OcrPageInput {
  data: Uint8ClampedArray | Uint8Array
  width: number
  height: number
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
