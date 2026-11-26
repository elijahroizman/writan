// Messages between the PDF editor and its PDFium worker. Coordinates are in "display points":
// PDF points on the page as shown (after its rotation), measured from the top-left corner.

import type { PdfAnnotationObject } from '@embedpdf/models'

export interface Box {
  x: number
  y: number
  w: number
  h: number
}

export interface PageInfo {
  /** Size as displayed, in points. */
  width: number
  height: number
  /** Quarter turns clockwise (0–3). */
  rotation: number
  /** The page dictionary's object number: follows the page when pages move. */
  key: number
}

export interface DocMeta {
  title?: string
  author?: string
  producer?: string
  version?: string
}

export interface OpenResult {
  pages: PageInfo[]
  meta: DocMeta
  encrypted: boolean
  /** The PDF has fillable form fields (AcroForm or XFA). */
  hasForm: boolean
}

/** A strip of a drawing: an ImageBitmap where the worker can make one, otherwise raw RGBA. */
export interface Band {
  y: number
  height: number
  image?: ImageBitmap
  data?: ArrayBuffer
}

export interface Bitmap {
  width: number
  height: number
  bands: Band[]
}

/** A line-ish run of text for the selectable text layer. */
export interface TextRun {
  text: string
  /** PDFium character index for each UTF-16 unit of `text` (spaces between words may be synthetic). */
  map: number[]
  box: Box
  /** Text runs downwards (vertical writing or a rotated line). */
  vertical?: boolean
  /** One word recognized from the page's picture; `map` is empty because PDFium has no such characters. */
  ocr?: boolean
}

export interface OutlineItem {
  title: string
  page?: number
  y?: number
  url?: string
  children: OutlineItem[]
}

export type FieldKind = 'text' | 'checkbox' | 'radio' | 'combo' | 'list' | 'button' | 'signature' | 'other'

export interface Field {
  kind: FieldKind
  name: string
  value: string
  checked: boolean
  readOnly: boolean
  multiline: boolean
  password: boolean
  fontSize: number
  options: { label: string; selected: boolean }[]
}

export interface Annot {
  id: string
  type: number
  box: Box
  /** Links: an internal page (0-based) or a web address. */
  link?: { page?: number; y?: number; url?: string }
  field?: Field
  /** The engine's object, passed back when moving, resizing or deleting. */
  raw: PdfAnnotationObject
}

export interface SearchHit {
  page: number
  boxes: Box[]
}

export interface PageTextStats {
  index: number
  charCount: number
  nonWhitespaceCount: number
  /** Share of the page (0–1) under its largest picture. */
  imageCoverage: number
  /** Share of the page (0–1) covered by its characters; measured only on pages that are mostly a picture. */
  textCoverage: number
}

/** Methods take a page's `key` (see PageInfo), except where they take a position in the document. */
export interface Api {
  open(bytes: ArrayBuffer, password: string): OpenResult
  /** Render `rect` (display points; whole page when omitted) at `scale` device pixels per point. */
  render(key: number, scale: number, rect: Box | null): Bitmap
  text(key: number): TextRun[]
  pageTextStats(key: number): PageTextStats
  documentTextStats(): PageTextStats[]
  /** Tight boxes around characters [start, start + count). */
  textBoxes(key: number, start: number, count: number): Box[]
  annotations(key: number): Annot[]
  createAnnotation(key: number, annot: PdfAnnotationObject, image: ArrayBuffer | null): string
  updateAnnotation(key: number, annot: PdfAnnotationObject): void
  /** Remove an annotation; the returned token restores it (see restoreAnnotation). */
  removeAnnotation(key: number, annot: PdfAnnotationObject): number
  restoreAnnotation(token: number): string
  setField(key: number, annot: PdfAnnotationObject, value: { text: string } | { checked: boolean } | { option: number }): void
  search(key: number, query: string, matchCase: boolean): Box[][]
  outline(): OutlineItem[]
  rotate(keys: number[], quarterTurns: number): PageInfo[]
  move(key: number, to: number): PageInfo[]
  duplicate(key: number): PageInfo[]
  insertBlank(at: number, width: number, height: number): PageInfo[]
  insertPdf(at: number, bytes: ArrayBuffer, password?: string): { pages: PageInfo[]; count: number }
  /** Delete pages; the returned token puts them back (see restorePages). */
  deletePages(keys: number[]): { pages: PageInfo[]; token: number }
  restorePages(token: number): PageInfo[]
  extract(keys: number[]): ArrayBuffer
  save(): ArrayBuffer
  /** A copy with form fields and annotations made part of the page, so they can't be changed. */
  flatten(): ArrayBuffer
  /** Bytes of WebAssembly memory in use, for diagnostics. */
  memory(): number
}

export type Method = keyof Api

export type Request = { id: number; method: Method; args: unknown[]; priority: number } | { cancel: number }

export type Response = { id: number; result?: unknown; error?: { message: string; code?: number } }

export interface Boot {
  boot: true
  module?: WebAssembly.Module
  wasmUrl: string
}
