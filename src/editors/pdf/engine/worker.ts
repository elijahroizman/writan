/// <reference lib="webworker" />
// One PDFium (WebAssembly) instance per open PDF. Requests are queued by priority, so drawing
// the visible pages always comes before thumbnails or search, and the editor can cancel work
// for pages that scrolled away before it starts.

import { PdfiumNative } from '@embedpdf/engines/pdfium'
import type { PdfAnnotationObject, PdfDocumentObject, PdfPageObject, PdfBookmarkObject, PdfLinkTarget, PdfWidgetAnnoObject, Rect } from '@embedpdf/models'
import { AppearanceMode, PDF_FORM_FIELD_FLAG, PDF_FORM_FIELD_TYPE, PdfActionType, PdfAnnotationFlags, PdfAnnotationSubtype, PdfPageFlattenFlag, PdfPageFlattenResult, PdfZoomMode } from '@embedpdf/models'
import { init } from '@embedpdf/pdfium'
import type { Annot, Api, Band, Bitmap, Boot, Box, DocMeta, Field, GrayBitmap, LayoutPicture, LayoutSpan, Method, OpenResult, OutlineItem, PageInfo, PageLayout, PageTextStats, Request, Response, TextRun } from './protocol'

type Module = Awaited<ReturnType<typeof init>>

interface PageCtx {
  pagePtr: number
  getTextPage(): number
  release(): void
}
interface DocCtx {
  docPtr: number
  acquirePage(index: number): PageCtx
  pageCache: { forceReleaseAll(): void }
}
/** The parts of PdfiumNative we reach into: its document cache and helpers that aren't public API. */
interface NativeInternals {
  cache: { getContext(id: string): DocCtx | undefined; updateConfig(config: { maxPagesPerDocument: number; pageTtl: number }): void }
  readPageBoxes(docPtr: number, index: number, boxPtr: number): PdfPageObject['boxes']
  convertPagePointToDevicePoint(doc: PdfDocumentObject, page: PdfPageObject, p: { x: number; y: number }): { x: number; y: number }
  exportAnnotationAppearanceAsPdf(doc: PdfDocumentObject, page: PdfPageObject, annot: PdfAnnotationObject): { toPromise(): Promise<ArrayBuffer> }
}

const DOC_ID = 'doc'
const FLAT_ID = 'flat'
/** What the open document was unlocked with, to open copies of it. */
let openPassword = ''
let mod: Module
let engine: PdfiumNative
let internals: NativeInternals
let doc: PdfDocumentObject | null = null
/** Holds deleted pages so undo can put them back. */
let trashDoc = 0
let nextToken = 1
const deletedPages = new Map<number, { at: number[]; trashIndices: number[] }>()
const removedAnnots = new Map<number, { page: number; annot: PdfAnnotationObject; appearance: ArrayBuffer | null }>()

const ctx = () => {
  const c = internals.cache.getContext(DOC_ID)
  if (!c) throw new Error('No document is open')
  return c
}
/** Page dictionaries' object numbers → current index. Requests name pages this way, so a render
 *  queued before a page moved still draws the right page. */
let byKey = new Map<number, number>()
const page = (key: number) => {
  const index = byKey.get(key)
  const p = index === undefined ? undefined : doc?.pages[index]
  if (!p) throw Object.assign(new Error('That page is gone'), { code: -1 })
  return p
}
const indexOf = (key: number) => page(key).index
const task = async <T>(t: { toPromise(): Promise<T> }): Promise<T> => {
  try {
    return await t.toPromise()
  } catch (err) {
    const reason = (err as { reason?: { message?: string; code?: number } }).reason
    throw Object.assign(new Error(reason?.message ?? String(err)), { code: reason?.code })
  }
}

// ── Scratch memory
const scratch = (bytes: number) => mod.pdfium.wasmExports.malloc(bytes)
const free = (ptr: number) => mod.pdfium.wasmExports.free(ptr)
const getF = (ptr: number) => mod.pdfium.getValue(ptr, 'float')
const getD = (ptr: number) => mod.pdfium.getValue(ptr, 'double')

/** Emscripten's heap views (the package's types leave them out). */
const heap = () => mod.pdfium as unknown as { HEAPU8: Uint8Array<ArrayBuffer>; HEAPF32: Float32Array<ArrayBuffer> }

const toBox = (r: Rect): Box => ({ x: r.origin.x, y: r.origin.y, w: r.size.width, h: r.size.height })

/** Re-read the page list after the page tree changed (and drop PDFium's cached page handles). */
function readPages(): PageInfo[] {
  const c = ctx()
  c.pageCache.forceReleaseAll()
  const count = mod.FPDF_GetPageCount(c.docPtr)
  const size = scratch(8)
  const boxPtr = scratch(16)
  const pages: PdfPageObject[] = []
  for (let index = 0; index < count; index++) {
    mod.FPDF_GetPageSizeByIndexF(c.docPtr, index, size)
    pages.push({
      index,
      size: { width: getF(size), height: getF(size + 4) },
      rotation: mod.EPDF_GetPageRotationByIndex(c.docPtr, index),
      objectNumber: mod.EPDFDoc_GetPageObjectNumberByIndex(c.docPtr, index),
      boxes: internals.readPageBoxes(c.docPtr, index, boxPtr),
    })
  }
  free(size)
  free(boxPtr)
  setPages(pages)
  return pages.map(info)
}

function setPages(pages: PdfPageObject[]) {
  doc!.pages = pages
  doc!.pageCount = pages.length
  byKey = new Map(pages.map((p) => [p.objectNumber || -1 - p.index, p.index]))
}

const info = (p: PdfPageObject): PageInfo => ({ width: p.size.width, height: p.size.height, rotation: p.rotation, key: p.objectNumber || -1 - p.index })

// ── Rendering

let hasForms = false

function openForm(pagePtr: number) {
  const info = mod.PDFiumExt_OpenFormFillInfo()
  const handle = mod.PDFiumExt_InitFormFillEnvironment(ctx().docPtr, info)
  mod.FORM_OnAfterLoadPage(pagePtr, handle)
  return {
    handle,
    close() {
      mod.FORM_OnBeforeClosePage(pagePtr, handle)
      mod.PDFiumExt_ExitFormFillEnvironment(handle)
      mod.PDFiumExt_CloseFormFillInfo(info)
    },
  }
}

const canMakeBitmaps = typeof createImageBitmap === 'function' && typeof ImageData === 'function'
/**
 * Big drawings are made in horizontal bands of about this many pixels. WebAssembly memory never
 * shrinks, so one full-page buffer would keep the worker at its biggest size for good.
 */
const BAND_PIXELS = 1 << 20

async function render(key: number, scale: number, rect: Box | null): Promise<Bitmap> {
  const p = page(key)
  const r = rect ?? { x: 0, y: 0, w: p.size.width, h: p.size.height }
  const width = Math.max(1, Math.round(r.w * scale))
  const height = Math.max(1, Math.round(r.h * scale))
  const sx = width / r.w
  const sy = height / r.h
  const bandRows = Math.max(1, Math.min(height, Math.floor(BAND_PIXELS / width)))
  const stride = width * 4
  const pc = ctx().acquirePage(p.index)
  const buffer = scratch(stride * bandRows)
  const matrix = scratch(24)
  const clip = scratch(16)
  const bands: Band[] = []
  // Filled-in form values are drawn by the form layer, not the page's annotations.
  const form = hasForms ? openForm(pc.pagePtr) : null
  try {
    for (let y = 0; y < height; y += bandRows) {
      const rows = Math.min(bandRows, height - y)
      const bitmap = mod.FPDFBitmap_CreateEx(width, rows, 4 /* BGRA */, buffer, stride)
      try {
        mod.FPDFBitmap_FillRect(bitmap, 0, 0, width, rows, 0xffffffff)
        // Display points → device pixels. PDFium applies the page's own rotation first.
        new Float32Array(heap().HEAPF32.buffer, matrix, 6).set([sx, 0, 0, sy, -sx * r.x, -sy * r.y - y])
        new Float32Array(heap().HEAPF32.buffer, clip, 4).set([0, 0, width, rows])
        // FPDF_ANNOT | FPDF_REVERSE_BYTE_ORDER (RGBA, as canvases want it) | FPDF_RENDER_LIMITEDIMAGECACHE
        mod.FPDF_RenderPageBitmapWithMatrix(bitmap, pc.pagePtr, matrix, clip, 0x01 | 0x10 | 0x200)
        if (form) mod.FPDF_FFLDraw(form.handle, bitmap, pc.pagePtr, Math.round(-sx * r.x), Math.round(-sy * r.y - y), Math.round(p.size.width * sx), Math.round(p.size.height * sy), 0, 0x01 | 0x10)
      } finally {
        mod.FPDFBitmap_Destroy(bitmap)
      }
      const view = new Uint8ClampedArray(heap().HEAPU8.buffer, buffer, stride * rows)
      let image: ImageBitmap | undefined
      if (canMakeBitmaps) image = await createImageBitmap(new ImageData(view, width, rows)).catch(() => undefined)
      bands.push(image ? { y, height: rows, image } : { y, height: rows, data: view.slice().buffer })
    }
    return { width, height, bands }
  } finally {
    form?.close()
    free(buffer)
    free(matrix)
    free(clip)
    pc.release()
  }
}

/** Render directly to 8-bit grayscale in the worker so OCR avoids main-thread canvas readback. */
function renderGrayscale(key: number, scale: number, rect: Box | null): GrayBitmap {
  const p = page(key)
  const r = rect ?? { x: 0, y: 0, w: p.size.width, h: p.size.height }
  const width = Math.max(1, Math.round(r.w * scale))
  const height = Math.max(1, Math.round(r.h * scale))
  const sx = width / r.w
  const sy = height / r.h
  const bandRows = Math.max(1, Math.min(height, Math.floor(BAND_PIXELS / width)))
  const stride = width * 4
  const pc = ctx().acquirePage(p.index)
  const buffer = scratch(stride * bandRows)
  const matrix = scratch(24)
  const clip = scratch(16)
  const gray = new Uint8Array(width * height)
  const form = hasForms ? openForm(pc.pagePtr) : null
  try {
    for (let y = 0; y < height; y += bandRows) {
      const rows = Math.min(bandRows, height - y)
      const bitmap = mod.FPDFBitmap_CreateEx(width, rows, 4 /* BGRA */, buffer, stride)
      try {
        mod.FPDFBitmap_FillRect(bitmap, 0, 0, width, rows, 0xffffffff)
        new Float32Array(heap().HEAPF32.buffer, matrix, 6).set([sx, 0, 0, sy, -sx * r.x, -sy * r.y - y])
        new Float32Array(heap().HEAPF32.buffer, clip, 4).set([0, 0, width, rows])
        mod.FPDF_RenderPageBitmapWithMatrix(bitmap, pc.pagePtr, matrix, clip, 0x01 | 0x10 | 0x200)
        if (form) mod.FPDF_FFLDraw(form.handle, bitmap, pc.pagePtr, Math.round(-sx * r.x), Math.round(-sy * r.y - y), Math.round(p.size.width * sx), Math.round(p.size.height * sy), 0, 0x01 | 0x10)
      } finally {
        mod.FPDFBitmap_Destroy(bitmap)
      }
      const rgba = heap().HEAPU8
      const pixels = width * rows
      let dst = y * width
      let src = buffer
      for (let i = 0; i < pixels; i++, src += 4) {
        gray[dst++] = (77 * rgba[src] + 150 * rgba[src + 1] + 29 * rgba[src + 2] + 128) >> 8
      }
    }
    return { width, height, data: gray.buffer }
  } finally {
    form?.close()
    free(buffer)
    free(matrix)
    free(clip)
    pc.release()
  }
}

// ── Text

/** Lines of text with their positions, for the invisible layer that makes text selectable. */
function text(key: number): TextRun[] {
  const p = page(key)
  const pc = ctx().acquirePage(p.index)
  try {
    const tp = pc.getTextPage()
    const count = mod.FPDFText_CountChars(tp)
    const rect = scratch(16)
    const runs: TextRun[] = []
    let run: { text: string; map: number[]; x0: number; y0: number; x1: number; y1: number; lastX: number; lastLeft: number; vertical: boolean; rtl: boolean } | null = null
    const flush = () => {
      if (run && run.text.trim()) {
        runs.push({
          text: run.text.trimEnd(),
          map: run.map.slice(0, run.text.trimEnd().length),
          box: { x: run.x0, y: run.y0, w: run.x1 - run.x0, h: run.y1 - run.y0 },
          vertical: run.vertical || undefined,
        })
      }
      run = null
    }
    for (let i = 0; i < count; i++) {
      const code = mod.FPDFText_GetUnicode(tp, i)
      // Control characters (common in OCR text) have no glyph, and WebKit hunts through every
      // font for one each time it paints them.
      if (code === 0xfffe || code === 0xfffd || (code < 0x20 && code !== 13 && code !== 10) || (code >= 0x7f && code < 0xa0)) continue
      if (code === 13 || code === 10) {
        flush()
        continue
      }
      const ch = String.fromCodePoint(code)
      const hasBox = mod.FPDFText_GetLooseCharBox(tp, i, rect)
      let box: Box | null = null
      if (hasBox) {
        const l = getF(rect)
        const t = getF(rect + 4)
        const r = getF(rect + 8)
        const b = getF(rect + 12)
        if (r > l && t > b) {
          const a = internals.convertPagePointToDevicePoint(doc!, p, { x: l, y: t })
          const c = internals.convertPagePointToDevicePoint(doc!, p, { x: r, y: b })
          box = { x: Math.min(a.x, c.x), y: Math.min(a.y, c.y), w: Math.abs(c.x - a.x), h: Math.abs(c.y - a.y) }
        }
      }
      if (!box) {
        // Spaces (often generated by PDFium between words) have no box; keep them inside the run.
        if (run && /\s/.test(ch)) {
          run.text += ' '
          run.map.push(i)
        }
        continue
      }
      if (run) {
        const h = run.vertical ? run.x1 - run.x0 : run.y1 - run.y0
        const midY = box.y + box.h / 2
        const sameLine = run.vertical
          ? Math.abs(box.x + box.w / 2 - (run.x0 + run.x1) / 2) < h * 0.5 && box.y >= run.y1 - h
          : run.rtl
            ? !/[A-Za-z0-9]/.test(ch) && midY > run.y0 && midY < run.y1 && box.x + box.w <= run.lastX + h * 0.5 && run.x0 - (box.x + box.w) < h * 3
            : // The letters of a ligature (fi, ffi) each get the whole ligature's box.
              midY > run.y0 && midY < run.y1 && (box.x >= run.lastX - h * 0.5 || (Math.abs(box.x - run.lastLeft) < h * 0.05 && Math.abs(box.x + box.w - run.lastX) < h * 0.05)) && box.x - run.x1 < h * 3
        if (!sameLine) {
          // A single character can't tell us the direction yet; a second one straight below makes it vertical,
          // and a short non-LTR prefix moving leftward makes it right-to-left.
          const below = run.text.length === 1 && Math.abs(box.x - run.x0) < box.w * 0.5 && box.y >= run.y1 - 1
          const toLeft =
            !run.vertical &&
            !/[A-Za-z0-9]/.test(ch) &&
            run.text.length <= 3 &&
            midY > run.y0 &&
            midY < run.y1 &&
            box.x + box.w <= run.x0 + h * 0.5 &&
            run.x0 - (box.x + box.w) < h * 3
          if (below) run.vertical = true
          else if (toLeft) run.rtl = true
          else flush()
        } else if (run.rtl && run.x0 - (box.x + box.w) > h * 0.3 && !/\s$/.test(run.text) && !/\s/.test(ch)) {
          run.text += ' '
          run.map.push(i)
        } else if (!run.vertical && !run.rtl && box.x - run.x1 > h * 0.3 && !/\s$/.test(run.text) && !/\s/.test(ch)) {
          run.text += ' '
          run.map.push(i)
        }
      }
      if (!run) run = { text: '', map: [], x0: box.x, y0: box.y, x1: box.x + box.w, y1: box.y + box.h, lastX: box.x, lastLeft: box.x, vertical: false, rtl: false }
      run.text += ch
      // One entry per UTF-16 unit, so characters outside the BMP take two.
      for (let u = 0; u < ch.length; u++) run.map.push(i)
      run.x0 = Math.min(run.x0, box.x)
      run.y0 = Math.min(run.y0, box.y)
      run.x1 = Math.max(run.x1, box.x + box.w)
      run.y1 = Math.max(run.y1, box.y + box.h)
      run.lastX = run.rtl ? box.x : box.x + box.w
      run.lastLeft = box.x
    }
    flush()
    free(rect)
    // Printer's marks and the like outside the visible page aren't part of what you can read.
    const { width, height } = p.size
    return runs.filter((r) => r.box.x < width && r.box.y < height && r.box.x + r.box.w > 0 && r.box.y + r.box.h > 0)
  } finally {
    pc.release()
  }
}

function statsForIndex(index: number): PageTextStats {
  const pc = ctx().acquirePage(index)
  try {
    const tp = pc.getTextPage()
    const count = mod.FPDFText_CountChars(tp)
    let nonWhitespaceCount = 0
    for (let i = 0; i < count; i++) {
      const code = mod.FPDFText_GetUnicode(tp, i)
      if (code > 0x20 && code !== 0xfeff && code !== 0xfffe && code !== 0xfffd && !(code >= 0x7f && code < 0xa0)) {
        nonWhitespaceCount++
        if (nonWhitespaceCount >= 1500) break
      }
    }
    // A page full of text isn't a scan to read (if it is one, it already has its text).
    const imageCoverage = nonWhitespaceCount < 1500 ? pictureCoverage(pc.pagePtr) : 0
    // Scans often carry a little real text (a page number, a "downloaded from" line): measure how
    // much of the page it takes up, which only matters on a page that is mostly a picture.
    const textCoverage = imageCoverage >= 0.3 && nonWhitespaceCount >= 10 ? textArea(tp, count, pc.pagePtr) : 0
    return { index, charCount: count, nonWhitespaceCount, imageCoverage, textCoverage }
  } finally {
    pc.release()
  }
}

/** Share of the page (0–1) covered by the boxes of its characters. */
function textArea(tp: number, count: number, pagePtr: number): number {
  const width = mod.FPDF_GetPageWidthF(pagePtr)
  const height = mod.FPDF_GetPageHeightF(pagePtr)
  if (!(width > 0 && height > 0)) return 0
  const box = scratch(32)
  let area = 0
  try {
    for (let i = 0; i < count; i++) {
      if (!mod.FPDFText_GetCharBox(tp, i, box, box + 8, box + 16, box + 24)) continue
      const w = getD(box + 8) - getD(box)
      const h = getD(box + 24) - getD(box + 16)
      if (w > 0 && h > 0) area += w * h
    }
  } finally {
    free(box)
  }
  return Math.min(1, area / (width * height))
}

type Matrix = [number, number, number, number, number, number]

/** `outer` after `inner`: maps what `inner` maps from to where `outer` maps to. */
const compose = ([a, b, c, d, e, f]: Matrix, [p, q, r, s, t, u]: Matrix): Matrix => [a * p + c * q, b * p + d * q, a * r + c * s, b * r + d * s, a * t + c * u + e, b * t + d * u + f]

/**
 * Share of the page (0–1) under pictures, adding them up, as scanners may draw a page in strips.
 * Objects inside a form XObject report bounds in the form's own space, so they're carried to the
 * page through each form's matrix.
 */
function pictureCoverage(pagePtr: number): number {
  const page = scratch(24)
  const rect = scratch(16)
  try {
    // The visible page, in the same space as the page's objects (its origin needn't be 0, 0).
    if (!mod.FPDF_GetPageBoundingBox(pagePtr, page)) return 0
    const [a, b, c, d] = [getF(page), getF(page + 4), getF(page + 8), getF(page + 12)]
    const [x0, x1, y0, y1] = [Math.min(a, c), Math.max(a, c), Math.min(b, d), Math.max(b, d)]
    const area = (x1 - x0) * (y1 - y0)
    if (!(area > 0)) return 0
    let covered = 0
    const visit = (obj: number, toPage: Matrix, depth: number) => {
      const type = mod.FPDFPageObj_GetType(obj)
      if (type === 5 /* FPDF_PAGEOBJ_FORM */ && depth < 4) {
        if (!mod.FPDFPageObj_GetMatrix(obj, page)) return
        const form: Matrix = [0, 4, 8, 12, 16, 20].map((o) => getF(page + o)) as Matrix
        const inner = compose(toPage, form)
        const n = mod.FPDFFormObj_CountObjects(obj)
        for (let i = 0; i < n && covered < 0.5; i++) visit(mod.FPDFFormObj_GetObject(obj, i), inner, depth + 1)
        return
      }
      // left, bottom, right, top
      if (type !== 3 /* FPDF_PAGEOBJ_IMAGE */ || !mod.FPDFPageObj_GetBounds(obj, rect, rect + 4, rect + 8, rect + 12)) return
      const [l, bt, r, t] = [getF(rect), getF(rect + 4), getF(rect + 8), getF(rect + 12)]
      const [ma, mb, mc, md, me, mf] = toPage
      const xs = [l * ma + bt * mc + me, r * ma + bt * mc + me, l * ma + t * mc + me, r * ma + t * mc + me]
      const ys = [l * mb + bt * md + mf, r * mb + bt * md + mf, l * mb + t * md + mf, r * mb + t * md + mf]
      const w = Math.min(Math.max(...xs), x1) - Math.max(Math.min(...xs), x0)
      const h = Math.min(Math.max(...ys), y1) - Math.max(Math.min(...ys), y0)
      if (w > 0 && h > 0) covered += (w * h) / area
    }
    const n = mod.FPDFPage_CountObjects(pagePtr)
    for (let i = 0; i < n && covered < 0.5; i++) visit(mod.FPDFPage_GetObject(pagePtr, i), [1, 0, 0, 1, 0, 0], 0)
    return Math.min(1, covered)
  } finally {
    free(page)
    free(rect)
  }
}

function pageTextStats(key: number): PageTextStats {
  return statsForIndex(page(key).index)
}

function pageTextStatsBatch(keys: number[]): PageTextStats[] {
  const out: PageTextStats[] = []
  for (const key of keys) out.push(statsForIndex(page(key).index))
  return out
}

function documentTextStats(): PageTextStats[] {
  const count = doc?.pages.length ?? 0
  const stats: PageTextStats[] = []
  for (let i = 0; i < count; i++) stats.push(statsForIndex(i))
  return stats
}

function textBoxes(key: number, start: number, count: number): Box[] {
  const p = page(key)
  const pc = ctx().acquirePage(p.index)
  const l = scratch(8)
  const t = scratch(8)
  const r = scratch(8)
  const b = scratch(8)
  try {
    const tp = pc.getTextPage()
    const n = mod.FPDFText_CountRects(tp, start, count)
    const boxes: Box[] = []
    for (let i = 0; i < n; i++) {
      if (!mod.FPDFText_GetRect(tp, i, l, t, r, b)) continue
      const a = internals.convertPagePointToDevicePoint(doc!, p, { x: getD(l), y: getD(t) })
      const c = internals.convertPagePointToDevicePoint(doc!, p, { x: getD(r), y: getD(b) })
      boxes.push({ x: Math.min(a.x, c.x), y: Math.min(a.y, c.y), w: Math.abs(c.x - a.x), h: Math.abs(c.y - a.y) })
    }
    return boxes
  } finally {
    ;[l, t, r, b].forEach(free)
    pc.release()
  }
}

// ── Layout, for converting a page to Word

const RTL_CHAR = /[֐-ࣿיִ-﷿ﹰ-﻿]/
const LTR_CHAR = /[A-Za-z0-9À-ɏͰ-ϿЀ-ӿ]/
const BOLD_NAME = /bold|black|heavy|semibold|demibold|demi\b|extrabold|ultrabold/i
const ITALIC_NAME = /italic|oblique|kursiv/i
/** PDF's 14 standard fonts and common PostScript names, as the fonts Word documents use. */
const KNOWN_FONTS: Record<string, string> = {
  helvetica: 'Arial',
  arial: 'Arial',
  arialnarrow: 'Arial Narrow',
  times: 'Times New Roman',
  timesroman: 'Times New Roman',
  timesnewroman: 'Times New Roman',
  courier: 'Courier New',
  couriernew: 'Courier New',
  symbol: 'Symbol',
  zapfdingbats: 'Wingdings',
  // The metric-compatible stand-ins Linux (and Writan's own PDFs) use for Office's fonts.
  carlito: 'Calibri',
  caladea: 'Cambria',
  liberationsans: 'Arial',
  liberationserif: 'Times New Roman',
  liberationmono: 'Courier New',
  arimo: 'Arial',
  tinos: 'Times New Roman',
  cousine: 'Courier New',
  // Copies of Helvetica, Times and Courier that TeX and Ghostscript embed.
  nimbussans: 'Arial',
  nimbussansl: 'Arial',
  nimbussanl: 'Arial',
  nimbusroman: 'Times New Roman',
  nimbusromno9l: 'Times New Roman',
  nimbusmono: 'Courier New',
  nimbusmonol: 'Courier New',
  nimbusmonl: 'Courier New',
  texgyreheros: 'Arial',
  texgyretermes: 'Times New Roman',
  texgyrecursor: 'Courier New',
}

interface FontStyle {
  font?: string
  bold: boolean
  italic: boolean
  invisible: boolean
}

/** A family name from a PDF font's name: `ABCDEF+TimesNewRomanPS-BoldMT` is Times New Roman. */
function cleanFontName(raw: string): string | undefined {
  let name = raw.replace(/^[A-Z]{6}\+/, '').trim()
  if (!name || /^(foxit|chrome)/i.test(name)) return undefined
  name = name.split(/[,-]/)[0].trim()
  const compact = name.replace(/\s+/g, '').toLowerCase().replace(/(psmt|ps|mt)$/, '')
  if (KNOWN_FONTS[compact]) return KNOWN_FONTS[compact]
  name = name.replace(/(PSMT|PS|MT)$/, '')
  if (!/\s/.test(name)) {
    const styleless = name.replace(/(Bold|Italic|Oblique|Regular|Medium|Light|Semibold|SemiBold|Black|Heavy|Book|Condensed)+$/, '')
    if (styleless.length >= 2) name = styleless
    if (KNOWN_FONTS[name.toLowerCase()]) return KNOWN_FONTS[name.toLowerCase()]
    name = name.replace(/([a-z])([A-Z])/g, '$1 $2')
  } else {
    name = name.replace(/\s+(Bold|Italic|Oblique|Regular|Medium|Light|Semibold|Black|Heavy)(\s+\w+)*$/i, '')
  }
  return name.trim() || undefined
}

function fontName(read: (buf: number, len: number) => number): string {
  let len = 256
  for (let attempt = 0; attempt < 2; attempt++) {
    const buf = scratch(len)
    try {
      const n = read(buf, len)
      if (n <= 1) return ''
      if (n > len) {
        len = n
        continue
      }
      return new TextDecoder().decode(heap().HEAPU8.slice(buf, buf + n - 1))
    } finally {
      free(buf)
    }
  }
  return ''
}

function textStyle(obj: number): FontStyle {
  const mode = obj ? mod.FPDFTextObj_GetTextRenderMode(obj) : 0
  const invisible = mode === 3 || mode === 7
  const font = obj ? mod.FPDFTextObj_GetFont(obj) : 0
  if (!font) return { bold: false, italic: false, invisible }
  const base = fontName((buf, len) => mod.FPDFFont_GetBaseFontName(font, buf, len))
  // An embedded font names its family; a missing one is drawn with PDFium's own stand-ins.
  const family = mod.FPDFFont_GetIsEmbedded(font) ? fontName((buf, len) => mod.FPDFFont_GetFamilyName(font, buf, len)) : ''
  const weight = mod.FPDFFont_GetWeight(font)
  const flags = mod.FPDFFont_GetFlags(font)
  const anglePtr = scratch(4)
  const angle = mod.FPDFFont_GetItalicAngle(font, anglePtr) ? mod.pdfium.getValue(anglePtr, 'i32') : 0
  free(anglePtr)
  const known = cleanFontName(base)
  return {
    font: (known && Object.values(KNOWN_FONTS).includes(known) ? known : cleanFontName(family)) ?? known,
    // Fill-and-stroke thickens the letters: how PDFs fake bold.
    bold: weight >= 600 || BOLD_NAME.test(base) || BOLD_NAME.test(family) || (flags & (1 << 18)) !== 0 || mode === 2,
    italic: ITALIC_NAME.test(base) || ITALIC_NAME.test(family) || (flags & 64) !== 0 || angle !== 0,
    invisible,
  }
}

/** Page-space bounds (left, bottom, right, top) as a box in display points. */
function displayBox(p: PdfPageObject, l: number, b: number, r: number, t: number): Box {
  const corners = [
    internals.convertPagePointToDevicePoint(doc!, p, { x: l, y: t }),
    internals.convertPagePointToDevicePoint(doc!, p, { x: r, y: b }),
    internals.convertPagePointToDevicePoint(doc!, p, { x: l, y: b }),
    internals.convertPagePointToDevicePoint(doc!, p, { x: r, y: t }),
  ]
  const xs = corners.map((c) => c.x)
  const ys = corners.map((c) => c.y)
  return { x: Math.min(...xs), y: Math.min(...ys), w: Math.max(...xs) - Math.min(...xs), h: Math.max(...ys) - Math.min(...ys) }
}

/** A region of the page drawn on a transparent background, as RGBA pixels. */
function renderRegion(pagePtr: number, r: Box, scale: number): { width: number; height: number; data: ArrayBuffer } {
  const width = Math.max(1, Math.round(r.w * scale))
  const height = Math.max(1, Math.round(r.h * scale))
  const bitmap = mod.FPDFBitmap_Create(width, height, 1)
  const matrix = scratch(24)
  const clip = scratch(16)
  try {
    mod.FPDFBitmap_FillRect(bitmap, 0, 0, width, height, 0x00000000)
    new Float32Array(heap().HEAPF32.buffer, matrix, 6).set([scale, 0, 0, scale, -scale * r.x, -scale * r.y])
    new Float32Array(heap().HEAPF32.buffer, clip, 4).set([0, 0, width, height])
    // FPDF_REVERSE_BYTE_ORDER (RGBA) | FPDF_RENDER_LIMITEDIMAGECACHE
    mod.FPDF_RenderPageBitmapWithMatrix(bitmap, pagePtr, matrix, clip, 0x10 | 0x200)
    const stride = mod.FPDFBitmap_GetStride(bitmap)
    const src = mod.FPDFBitmap_GetBuffer(bitmap)
    const out = new Uint8Array(width * height * 4)
    const bytes = heap().HEAPU8
    for (let y = 0; y < height; y++) out.set(bytes.subarray(src + y * stride, src + y * stride + width * 4), y * width * 4)
    return { width, height, data: out.buffer }
  } finally {
    mod.FPDFBitmap_Destroy(bitmap)
    free(matrix)
    free(clip)
  }
}

const overlaps = (a: Box, b: Box, pad = 0) => a.x - pad < b.x + b.w && b.x - pad < a.x + a.w && a.y - pad < b.y + b.h && b.y - pad < a.y + a.h
const unionBox = (a: Box, b: Box): Box => {
  const x = Math.min(a.x, b.x)
  const y = Math.min(a.y, b.y)
  return { x, y, w: Math.max(a.x + a.w, b.x + b.w) - x, h: Math.max(a.y + a.h, b.y + b.h) - y }
}
/** Joins boxes that touch (within `pad`) until none do. */
function mergeBoxes<T extends { box: Box }>(items: T[], pad: number, join: (a: T, b: T) => T): T[] {
  const out = [...items]
  for (let merged = true; merged; ) {
    merged = false
    for (let i = 0; i < out.length && !merged; i++) {
      for (let j = i + 1; j < out.length; j++) {
        if (overlaps(out[i].box, out[j].box, pad)) {
          out[i] = join(out[i], out[j])
          out.splice(j, 1)
          merged = true
          break
        }
      }
    }
  }
  return out
}

/** Most pixels a picture is drawn with: about 24 MB of RGBA. */
const MAX_PICTURE_PIXELS = 6_000_000

async function layout(key: number, withPictures: boolean): Promise<PageLayout> {
  const p = page(key)
  const links = (await annotations(key).catch(() => [] as Annot[])).flatMap((a) => (a.link?.url ? [{ box: a.box, url: a.link.url }] : []))
  const pc = ctx().acquirePage(p.index)
  const rect = scratch(16)
  const point = scratch(16)
  const rgba = scratch(16)
  const matrix = scratch(24)
  try {
    const tp = pc.getTextPage()
    const count = mod.FPDFText_CountChars(tp)
    const styles = new Map<number, FontStyle>()
    const fragments: PageLayout['fragments'] = []
    let invisibleChars = 0
    let visibleChars = 0
    // Text reads upright on screen when its angle on the page matches the page's own turn.
    const upright = (p.rotation * Math.PI) / 2
    type Building = { x0: number; y0: number; x1: number; y1: number; baseline: number; size: number; rtl: boolean; spans: LayoutSpan[]; lastLeft: number; lastRight: number; key: string }
    let frag: Building | null = null
    let pendingSpace = false
    const flush = () => {
      if (frag) {
        const last = frag.spans[frag.spans.length - 1]
        if (last) last.text = last.text.trimEnd()
        const spans = frag.spans.filter((s) => s.text)
        if (spans.some((s) => s.text.trim())) {
          fragments.push({ x: frag.x0, y: frag.y0, w: frag.x1 - frag.x0, h: frag.y1 - frag.y0, baseline: frag.baseline, size: frag.size, rtl: frag.rtl, spans })
        }
      }
      frag = null
    }
    for (let i = 0; i < count; i++) {
      const code = mod.FPDFText_GetUnicode(tp, i)
      if (code === 13 || code === 10 || code === 32 || code === 0xa0 || code === 9 || code === 0x3000) {
        pendingSpace = true
        continue
      }
      // PDFium reports a hyphen that ends a line as 2; it marks a word broken across lines.
      if (code === 0xfffe || code === 0xfffd || code === 0xfeff || (code < 0x20 && code !== 2) || (code >= 0x7f && code < 0xa0)) continue
      const ch = code === 2 ? '\u00AD' : String.fromCodePoint(code)
      if (/\s/.test(ch)) {
        pendingSpace = true
        continue
      }
      if (!mod.FPDFText_GetLooseCharBox(tp, i, rect)) continue
      const [l, t, r, b] = [getF(rect), getF(rect + 4), getF(rect + 8), getF(rect + 12)]
      if (!(r > l && t > b)) continue
      const box = displayBox(p, l, b, r, t)
      if (box.x > p.size.width || box.y > p.size.height || box.x + box.w < 0 || box.y + box.h < 0) continue
      const angle = mod.FPDFText_GetCharAngle(tp, i)
      if (angle >= 0) {
        const turn = (((angle - upright) % (2 * Math.PI)) + 2 * Math.PI) % (2 * Math.PI)
        if (Math.min(turn, 2 * Math.PI - turn) > 0.2) continue
      }
      let baseline = box.y + box.h * 0.8
      if (mod.FPDFText_GetCharOrigin(tp, i, point, point + 8)) {
        baseline = internals.convertPagePointToDevicePoint(doc!, p, { x: getD(point), y: getD(point + 8) }).y
      }
      // The font's size is set before the text's matrix scales it (Word writes size 1, scaled 12 times).
      let size = mod.FPDFText_GetFontSize(tp, i)
      if (mod.FPDFText_GetMatrix(tp, i, matrix)) {
        const scale = Math.hypot(getF(matrix + 8), getF(matrix + 12))
        if (scale > 0) size *= scale
      }
      if (!(size > 0) || size > box.h * 3 || size < box.h * 0.4) size = box.h / 1.2
      const obj = mod.FPDFText_GetTextObject(tp, i)
      let style = styles.get(obj)
      if (!style) styles.set(obj, (style = textStyle(obj)))
      if (style.invisible) invisibleChars++
      else visibleChars++
      let color: string | undefined
      if (!style.invisible && mod.FPDFText_GetFillColor(tp, i, rgba, rgba + 4, rgba + 8, rgba + 12)) {
        const [cr, cg, cb] = [0, 4, 8].map((o) => mod.pdfium.getValue(rgba + o, 'i32') & 0xff)
        color = ((1 << 24) | (cr << 16) | (cg << 8) | cb).toString(16).slice(1).toUpperCase()
      }
      const cx = box.x + box.w / 2
      const cy = box.y + box.h / 2
      const link = links.find((k) => cx >= k.box.x && cx <= k.box.x + k.box.w && cy >= k.box.y && cy <= k.box.y + k.box.h)?.url
      const spanKey = `${style.font}|${style.bold}|${style.italic}|${color}|${link}`
      const isRtl = RTL_CHAR.test(ch)
      let joined = false
      if (frag) {
        const f: Building = frag
        const s = Math.max(f.size, size)
        const sameLine = Math.abs(baseline - f.baseline) < 0.35 * s || (cy > f.y0 && cy < f.y1)
        const ligature = Math.abs(box.x - f.lastLeft) < box.h * 0.05 && Math.abs(box.x + box.w - f.lastRight) < box.h * 0.05
        const forward = box.x >= f.lastRight - 0.5 * s && box.x - f.lastRight < 0.8 * s
        const backward = box.x + box.w <= f.lastLeft + 0.5 * s && f.lastLeft - (box.x + box.w) < 0.8 * s
        const rtlFlow = f.rtl ? backward || (LTR_CHAR.test(ch) && forward) : forward || (isRtl && backward)
        if (sameLine && (ligature || rtlFlow)) {
          joined = true
          const gap = ligature ? 0 : backward && (f.rtl || isRtl) ? f.lastLeft - (box.x + box.w) : box.x - f.lastRight
          let span = f.spans[f.spans.length - 1]
          const shifted = Math.abs(baseline - span.baseline) > 0.15 * s
          if (span.text && (spanKey !== f.key || Math.abs(size - span.size) > 0.25 || shifted)) {
            span = { text: '', x: box.x, w: 0, baseline, size, font: style.font, bold: style.bold, italic: style.italic, color, link }
            f.spans.push(span)
            f.key = spanKey
          }
          const prev = f.spans.length > 1 && !span.text ? f.spans[f.spans.length - 2] : span
          if (!ligature && (pendingSpace || gap > 0.25 * s) && !/\s$/.test(prev.text)) span.text = span.text ? `${span.text} ` : ' '
          if (!ligature || !span.text.endsWith(ch)) span.text += ch
          const right = Math.max(span.x + span.w, box.x + box.w)
          span.x = Math.min(span.x, box.x)
          span.w = right - span.x
          f.x0 = Math.min(f.x0, box.x)
          f.y0 = Math.min(f.y0, box.y)
          f.x1 = Math.max(f.x1, box.x + box.w)
          f.y1 = Math.max(f.y1, box.y + box.h)
          if (size > f.size + 0.25) {
            f.size = size
            f.baseline = baseline
          }
          if (!f.rtl && isRtl && !f.spans.some((sp) => LTR_CHAR.test(sp.text))) f.rtl = true
          f.lastLeft = box.x
          f.lastRight = box.x + box.w
        }
      }
      if (!joined) {
        flush()
        frag = {
          x0: box.x,
          y0: box.y,
          x1: box.x + box.w,
          y1: box.y + box.h,
          baseline,
          size,
          rtl: isRtl,
          spans: [{ text: ch, x: box.x, w: box.w, baseline, size, font: style.font, bold: style.bold, italic: style.italic, color, link }],
          lastLeft: box.x,
          lastRight: box.x + box.w,
          key: spanKey,
        }
      }
      pendingSpace = false
    }
    flush()

    // Lines, pictures and drawings.
    const rules: Box[] = []
    const images: { box: Box; scale: number }[] = []
    const shapes: Box[] = []
    const others: number[] = []
    const pageBox = { x: 0, y: 0, w: p.size.width, h: p.size.height }
    const seg = scratch(8)
    const bounds = scratch(24)
    const visit = (obj: number, toPage: Matrix, depth: number) => {
      const type = mod.FPDFPageObj_GetType(obj)
      if (type === 5 /* form */) {
        if (depth >= 4 || !mod.FPDFPageObj_GetMatrix(obj, bounds)) return
        const inner = compose(toPage, [0, 4, 8, 12, 16, 20].map((o) => getF(bounds + o)) as Matrix)
        const n = mod.FPDFFormObj_CountObjects(obj)
        for (let i = 0; i < n; i++) visit(mod.FPDFFormObj_GetObject(obj, i), inner, depth + 1)
        return
      }
      if (type !== 3) others.push(obj)
      if ((type !== 2 && type !== 3) || !mod.FPDFPageObj_GetBounds(obj, bounds, bounds + 4, bounds + 8, bounds + 12)) return
      const [l, bt, r, t] = [getF(bounds), getF(bounds + 4), getF(bounds + 8), getF(bounds + 12)]
      const [ma, mb, mc, md, me, mf] = toPage
      const xs = [l * ma + bt * mc + me, r * ma + bt * mc + me, l * ma + t * mc + me, r * ma + t * mc + me]
      const ys = [l * mb + bt * md + mf, r * mb + bt * md + mf, l * mb + t * md + mf, r * mb + t * md + mf]
      const box = displayBox(p, Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys))
      if (!overlaps(box, pageBox)) return
      if (type === 3) {
        if (box.w < 4 || box.h < 4) return
        const px = scratch(8)
        const pixels = mod.FPDFImageObj_GetImagePixelSize(obj, px, px + 4) ? mod.pdfium.getValue(px, 'i32') : 0
        free(px)
        images.push({ box, scale: Math.min(3, Math.max(1.5, pixels / Math.max(1, box.w))) })
        return
      }
      // A path: a rule, a box's sides, or a drawn shape.
      if (!mod.FPDFPath_GetDrawMode(obj, seg, seg + 4)) return
      const filled = mod.pdfium.getValue(seg, 'i32') !== 0
      const stroked = mod.pdfium.getValue(seg + 4, 'i32') !== 0
      if (!filled && !stroked) return
      if (filled && !stroked && mod.FPDFPageObj_GetFillColor(obj, seg, seg + 4, rgba, rgba + 4)) {
        const white = [seg, seg + 4, rgba].every((ptr) => (mod.pdfium.getValue(ptr, 'i32') & 0xff) >= 250)
        if (white) return
      }
      if ((box.h <= 2.5 && box.w >= 4) || (box.w <= 2.5 && box.h >= 4)) {
        rules.push(box)
        return
      }
      const n = mod.FPDFPath_CountSegments(obj)
      let curved = false
      let slanted = false
      let px: number | null = null
      let py = 0
      for (let i = 0; i < n && !(curved && slanted); i++) {
        const s = mod.FPDFPath_GetPathSegment(obj, i)
        const kind = mod.FPDFPathSegment_GetType(s)
        if (kind === 1) curved = true
        if (!mod.FPDFPathSegment_GetPoint(s, rect, rect + 4)) continue
        const [x, y] = [getF(rect), getF(rect + 4)]
        if (kind === 0 && px !== null && Math.abs(x - px) > 0.5 && Math.abs(y - py) > 0.5) slanted = true
        px = x
        py = y
      }
      if (curved || slanted) shapes.push(box)
      else if (stroked && n <= 6 && box.w > 4 && box.h > 4) {
        rules.push({ x: box.x, y: box.y, w: box.w, h: 0.5 }, { x: box.x, y: box.y + box.h - 0.5, w: box.w, h: 0.5 })
        rules.push({ x: box.x, y: box.y, w: 0.5, h: box.h }, { x: box.x + box.w - 0.5, y: box.y, w: 0.5, h: box.h })
      }
    }
    const n = mod.FPDFPage_CountObjects(pc.pagePtr)
    for (let i = 0; i < n; i++) visit(mod.FPDFPage_GetObject(pc.pagePtr, i), [1, 0, 0, 1, 0, 0], 0)
    free(seg)
    free(bounds)

    // Drawings: shapes with curves or slants close together, with little text inside (a
    // chart's labels, not a box of text with rounded corners).
    const pageArea = p.size.width * p.size.height
    const textIn = (box: Box) =>
      fragments.reduce((sum, f) => {
        const inside = f.x + f.w / 2 >= box.x && f.x + f.w / 2 <= box.x + box.w && f.y + f.h / 2 >= box.y && f.y + f.h / 2 <= box.y + box.h
        return inside ? sum + f.spans.reduce((n, s) => n + s.text.length, 0) * f.size * f.size * 0.5 : sum
      }, 0)
    let figures = mergeBoxes(
      shapes.map((box) => ({ box, count: 1 })),
      8,
      (a, b) => ({ box: unionBox(a.box, b.box), count: a.count + b.count }),
    ).filter((f) => {
      const area = f.box.w * f.box.h
      return area >= 2500 && area <= pageArea * 0.6 && (f.count >= 3 || area >= 5000) && textIn(f.box) < area * 0.15
    })
    // Pictures drawn in strips, or overlapping, are one picture; one inside a drawing is part of it.
    let pictures = mergeBoxes(images, 1, (a, b) => ({ box: unionBox(a.box, b.box), scale: Math.max(a.scale, b.scale) }))
    pictures = pictures.filter((pic) => {
      const figure = figures.findIndex((f) => overlaps(f.box, pic.box))
      if (figure === -1) return true
      figures[figure] = { ...figures[figure], box: unionBox(figures[figure].box, pic.box) }
      return false
    })
    figures = mergeBoxes(figures, 0, (a, b) => ({ box: unionBox(a.box, b.box), count: a.count + b.count }))

    const out: LayoutPicture[] = []
    if (withPictures) {
      const clipToPage = (b: Box): Box => {
        const x = Math.max(0, b.x)
        const y = Math.max(0, b.y)
        return { x, y, w: Math.min(p.size.width, b.x + b.w) - x, h: Math.min(p.size.height, b.y + b.h) - y }
      }
      const scaleFor = (b: Box, want: number) => Math.max(0.25, Math.min(want, Math.sqrt(MAX_PICTURE_PIXELS / Math.max(1, b.w * b.h))))
      for (const f of figures) {
        const box = clipToPage(f.box)
        if (box.w < 2 || box.h < 2) continue
        out.push({ box, figure: true, ...renderRegion(pc.pagePtr, box, scaleFor(box, 2)) })
      }
      if (pictures.length) {
        // Draw only the pictures: the text over them is converted as text.
        for (const obj of others) mod.FPDFPageObj_SetIsActive(obj, false)
        try {
          for (const pic of pictures) {
            const box = clipToPage(pic.box)
            if (box.w < 2 || box.h < 2) continue
            out.push({ box, figure: false, ...renderRegion(pc.pagePtr, box, scaleFor(box, pic.scale)) })
          }
        } finally {
          for (const obj of others) mod.FPDFPageObj_SetIsActive(obj, true)
        }
      }
    }
    return { width: p.size.width, height: p.size.height, fragments, pictures: out, rules, invisibleChars, visibleChars }
  } finally {
    free(rect)
    free(point)
    free(rgba)
    free(matrix)
    pc.release()
  }
}

// ── Annotations and forms

const fieldKind: Record<number, Field['kind']> = {
  [PDF_FORM_FIELD_TYPE.TEXTFIELD]: 'text',
  [PDF_FORM_FIELD_TYPE.CHECKBOX]: 'checkbox',
  [PDF_FORM_FIELD_TYPE.RADIOBUTTON]: 'radio',
  [PDF_FORM_FIELD_TYPE.COMBOBOX]: 'combo',
  [PDF_FORM_FIELD_TYPE.LISTBOX]: 'list',
  [PDF_FORM_FIELD_TYPE.PUSHBUTTON]: 'button',
  [PDF_FORM_FIELD_TYPE.SIGNATURE]: 'signature',
}

function linkTarget(target: PdfLinkTarget | undefined): Annot['link'] | undefined {
  if (!target) return undefined
  const dest = target.type === 'destination' ? target.destination : target.action.type === PdfActionType.Goto ? target.action.destination : null
  if (dest) {
    const y = dest.zoom.mode === PdfZoomMode.XYZ ? dest.zoom.params.y : undefined
    return { page: dest.pageIndex, y: topFromPdfY(dest.pageIndex, y) }
  }
  if (target.type === 'action' && target.action.type === PdfActionType.URI) return { url: target.action.uri }
  return undefined
}

/** Destinations give a y in PDF space (from the bottom); turn it into display points from the top. */
function topFromPdfY(index: number, y: number | undefined) {
  const p = doc?.pages[index]
  if (!p || y === undefined || y === null || !Number.isFinite(y)) return undefined
  const point = internals.convertPagePointToDevicePoint(doc!, p, { x: 0, y })
  return p.rotation % 2 === 0 ? Math.max(0, point.y) : undefined
}

async function annotations(key: number): Promise<Annot[]> {
  const list = await task(engine.getPageAnnotations(doc!, page(key)))
  const out: Annot[] = []
  for (const raw of list) {
    const a: Annot = { id: raw.id, type: raw.type, box: toBox(raw.rect), raw }
    if (raw.type === PdfAnnotationSubtype.LINK) {
      a.link = linkTarget((raw as { target?: PdfLinkTarget }).target)
      if (!a.link) continue
    } else if (raw.type === PdfAnnotationSubtype.WIDGET) {
      const w = raw as PdfWidgetAnnoObject
      const f = w.field
      const options = 'options' in f ? f.options : []
      a.field = {
        kind: fieldKind[f.type] ?? 'other',
        name: f.name,
        value: f.value ?? '',
        checked: f.type === PDF_FORM_FIELD_TYPE.CHECKBOX || f.type === PDF_FORM_FIELD_TYPE.RADIOBUTTON ? !!f.value && f.value !== 'Off' && f.value === w.exportValue : false,
        readOnly: !!(f.flag & PDF_FORM_FIELD_FLAG.READONLY) || !!w.flags?.includes('readOnly'),
        multiline: !!(f.flag & PDF_FORM_FIELD_FLAG.TEXT_MULTIPLINE),
        password: !!(f.flag & PDF_FORM_FIELD_FLAG.TEXT_PASSWORD),
        fontSize: w.fontSize,
        options: options.map((o) => ({ label: o.label, selected: o.isSelected })),
      }
    }
    out.push(a)
  }
  return out
}

/** Stamps read back from PDFium carry `name: -1`; writing that back wipes the stamp's image. */
function clean(annot: PdfAnnotationObject, index: number): PdfAnnotationObject {
  const a = { ...annot, pageIndex: index } as PdfAnnotationObject & { name?: unknown; icon?: unknown }
  if (typeof a.name !== 'string') delete a.name
  if (typeof a.icon !== 'string') delete a.icon
  return a
}

/** New marks print. Without the flag, printing and the flattened copy leave them out. */
function printable(annot: PdfAnnotationObject): PdfAnnotationObject {
  return annot.flags ? annot : { ...annot, flags: ['print'] }
}

async function createAnnotation(key: number, annot: PdfAnnotationObject, image: ArrayBuffer | null) {
  const p = page(key)
  const id = await task(engine.createPageAnnotation(doc!, p, printable(clean(annot, p.index)), (image ? { data: image } : undefined) as never))
  if (annot.type === PdfAnnotationSubtype.FREETEXT && ((p.rotation % 4) + 4) % 4 !== 0) {
    rotateFreeTextAppearance(p, annot, id as string)
  }
  return id
}

/**
 * On a rotated page the FreeText /Rect is stored in unrotated page space, which is
 * turned on its side (or upside down) relative to what the user sees. PDFium generates
 * the appearance inside that rect, so the text wraps into short columns and clips.
 * Fix: briefly give the annotation the display-space rect, let PDFium lay the text out
 * the way it was typed, restore the correct rect, and rotate the appearance into it
 * with the form's /Matrix.
 */
function rotateFreeTextAppearance(p: PdfPageObject, annot: PdfAnnotationObject, id: string) {
  const r = ((p.rotation % 4) + 4) % 4
  const pc = ctx().acquirePage(p.index)
  try {
    // Find the new annotation by its /NM (the id createPageAnnotation returned).
    let a = 0
    const n = mod.FPDFPage_GetAnnotCount(pc.pagePtr)
    for (let i = n - 1; i >= 0; i--) {
      const cand = mod.FPDFPage_GetAnnot(pc.pagePtr, i)
      const len = mod.FPDFAnnot_GetStringValue(cand, 'NM', 0, 0)
      const buf = scratch(Math.max(2, len * 2))
      mod.FPDFAnnot_GetStringValue(cand, 'NM', buf, len * 2)
      const nm = mod.pdfium.UTF16ToString(buf)
      free(buf)
      if (nm === id) {
        a = cand
        break
      }
    }
    if (!a) return
    const rect = scratch(16)
    mod.FPDFAnnot_GetRect(a, rect)
    const [l, t, rr, b] = [getF(rect), getF(rect + 4), getF(rect + 8), getF(rect + 12)]
    const setRect = (left: number, top: number, right: number, bottom: number) => {
      mod.pdfium.setValue(rect + 0, left, 'float')
      mod.pdfium.setValue(rect + 4, top, 'float')
      mod.pdfium.setValue(rect + 8, right, 'float')
      mod.pdfium.setValue(rect + 12, bottom, 'float')
      mod.FPDFAnnot_SetRect(a, rect)
    }
    const setMatrix = (m: number[]) => {
      const mp = scratch(24)
      m.forEach((v, i) => mod.pdfium.setValue(mp + i * 4, v, 'float'))
      mod.EPDFAnnot_SetAPMatrix(a, 0, mp)
      free(mp)
    }
    const { x, y } = annot.rect.origin
    const { width: dw, height: dh } = annot.rect.size
    const DH = p.size.height
    if (r === 2) {
      mod.EPDFAnnot_GenerateAppearance(a)
      setMatrix([-1, 0, 0, -1, l + rr, t + b])
    } else {
      setRect(x, DH - y, x + dw, DH - y - dh)
      mod.EPDFAnnot_GenerateAppearance(a)
      setRect(l, t, rr, b)
      if (r === 1) setMatrix([0, 1, -1, 0, l + DH - y, b - x])
      else setMatrix([0, -1, 1, 0, l - (DH - y - dh), t + x + dw])
    }
    mod.FPDFAnnot_UpdateObject(pc.pagePtr, a)
    free(rect)
  } finally {
    pc.release()
  }
}

async function updateAnnotation(key: number, annot: PdfAnnotationObject) {
  const p = page(key)
  await task(engine.updatePageAnnotation(doc!, p, clean(annot, p.index)))
}

async function removeAnnotation(key: number, annot: PdfAnnotationObject) {
  const p = page(key)
  // Images can't be rebuilt from the annotation object, so keep their drawing for undo.
  let appearance: ArrayBuffer | null = null
  if (annot.type === PdfAnnotationSubtype.STAMP) {
    appearance = await internals
      .exportAnnotationAppearanceAsPdf(doc!, p, annot)
      .toPromise()
      .catch(() => null)
  }
  await task(engine.removePageAnnotation(doc!, p, annot))
  const token = nextToken++
  removedAnnots.set(token, { page: key, annot, appearance })
  return token
}

async function restoreAnnotation(token: number) {
  const saved = removedAnnots.get(token)
  if (!saved) throw new Error('Nothing to restore')
  removedAnnots.delete(token)
  const p = page(saved.page)
  return task(engine.createPageAnnotation(doc!, p, printable(clean(saved.annot, p.index)), (saved.appearance ? { appearance: saved.appearance } : undefined) as never))
}

async function setField(key: number, annot: PdfAnnotationObject, value: { text: string } | { checked: boolean } | { option: number }) {
  const v = 'text' in value ? { kind: 'text' as const, text: value.text } : 'checked' in value ? { kind: 'checked' as const, checked: value.checked } : { kind: 'selection' as const, index: value.option, isSelected: true }
  await task(engine.setFormFieldValue(doc!, page(key), annot as PdfWidgetAnnoObject, v))
}

// ── Search and outline

async function search(key: number, query: string, matchCase: boolean): Promise<Box[][]> {
  const hits = await task(engine.searchInPage(doc!, page(key), query, matchCase ? 1 : 0))
  return hits.map((h) => h.rects.map(toBox))
}

async function searchBatch(keys: number[], query: string, matchCase: boolean): Promise<{ key: number; boxes: Box[][] }[]> {
  const out: { key: number; boxes: Box[][] }[] = []
  for (const key of keys) {
    const boxes = await search(key, query, matchCase)
    if (boxes.length) out.push({ key, boxes })
  }
  return out
}

async function outline(): Promise<OutlineItem[]> {
  const { bookmarks } = await task(engine.getBookmarks(doc!))
  const convert = (b: PdfBookmarkObject): OutlineItem => {
    const target = linkTarget(b.target)
    return { title: b.title, page: target?.page, y: target?.y, url: target?.url, children: (b.children ?? []).map(convert) }
  }
  return bookmarks.map(convert)
}

// ── Page edits happen in place: no re-parsing or re-saving the file.

function rotate(keys: number[], quarterTurns: number) {
  const c = ctx()
  for (const key of keys) {
    const pc = c.acquirePage(indexOf(key))
    try {
      mod.FPDFPage_SetRotation(pc.pagePtr, (((mod.FPDFPage_GetRotation(pc.pagePtr) + quarterTurns) % 4) + 4) % 4)
    } finally {
      pc.release()
    }
  }
  return readPages()
}

function withIndices<T>(indices: number[], fn: (ptr: number) => T): T {
  const ptr = scratch(Math.max(4, indices.length * 4))
  indices.forEach((v, i) => mod.pdfium.setValue(ptr + i * 4, v, 'i32'))
  try {
    return fn(ptr)
  } finally {
    free(ptr)
  }
}

function move(key: number, to: number) {
  if (!withIndices([indexOf(key)], (ptr) => mod.FPDF_MovePages(ctx().docPtr, ptr, 1, to))) throw new Error('Couldn’t move the page')
  return readPages()
}

function duplicate(key: number) {
  const index = indexOf(key)
  const docPtr = ctx().docPtr
  if (!withIndices([index], (ptr) => mod.FPDF_ImportPagesByIndex(docPtr, docPtr, ptr, 1, index + 1))) throw new Error('Couldn’t duplicate the page')
  return readPages()
}

function insertBlank(at: number, width: number, height: number) {
  const ptr = mod.FPDFPage_New(ctx().docPtr, at, width, height)
  if (!ptr) throw new Error('Couldn’t add a page')
  mod.FPDF_ClosePage(ptr)
  return readPages()
}

function insertPdf(at: number, bytes: ArrayBuffer, password = '') {
  const src = new Uint8Array(bytes)
  const filePtr = scratch(src.length)
  heap().HEAPU8.set(src, filePtr)
  const srcDoc = mod.FPDF_LoadMemDocument(filePtr, src.length, password)
  try {
    if (!srcDoc) {
      throw new Error(mod.FPDF_GetLastError() === 4 ? 'That PDF is password protected, so its pages can’t be inserted.' : 'That file isn’t a PDF Writan can read.')
    }
    const count = mod.FPDF_GetPageCount(srcDoc)
    if (!mod.FPDF_ImportPagesByIndex(ctx().docPtr, srcDoc, 0, 0, at)) throw new Error('Couldn’t insert the pages')
    return { pages: readPages(), count }
  } finally {
    if (srcDoc) mod.FPDF_CloseDocument(srcDoc)
    free(filePtr)
  }
}

function deletePages(keys: number[]) {
  const c = ctx()
  const sorted = [...new Set(keys.map(indexOf))].sort((a, b) => a - b)
  if (sorted.length >= mod.FPDF_GetPageCount(c.docPtr)) throw new Error('A PDF needs at least one page.')
  // Keep a copy for undo.
  trashDoc ||= mod.FPDF_CreateNewDocument()
  const base = mod.FPDF_GetPageCount(trashDoc)
  withIndices(sorted, (ptr) => mod.FPDF_ImportPagesByIndex(trashDoc, c.docPtr, ptr, sorted.length, base))
  for (const i of [...sorted].reverse()) mod.FPDFPage_Delete(c.docPtr, i)
  const token = nextToken++
  deletedPages.set(token, { at: sorted, trashIndices: sorted.map((_, i) => base + i) })
  return { pages: readPages(), token }
}

function restorePages(token: number) {
  const saved = deletedPages.get(token)
  if (!saved) throw new Error('Nothing to restore')
  deletedPages.delete(token)
  const docPtr = ctx().docPtr
  // Ascending order, so each page lands where it was.
  saved.at.forEach((at, i) => withIndices([saved.trashIndices[i]], (ptr) => mod.FPDF_ImportPagesByIndex(docPtr, trashDoc, ptr, 1, at)))
  return readPages()
}

async function extract(keys: number[]) {
  return task(engine.extractPages(doc!, keys.map(indexOf)))
}

// ── Open and save

async function open(bytes: ArrayBuffer, password: string): Promise<OpenResult> {
  // A preview opens each new version in the same worker; the old one goes first.
  if (doc) {
    const old = doc
    doc = null
    await task(engine.closeDocument(old)).catch(() => {})
  }
  const opened = await task(engine.openDocumentBuffer({ id: DOC_ID, content: bytes }, { password }))
  doc = opened
  openPassword = password
  setPages(opened.pages)
  hasForms = mod.FPDF_GetFormType(ctx().docPtr) !== 0
  const pages = opened.pages.map(info)
  const meta: DocMeta = {}
  try {
    const m = await task(engine.getMetadata(doc))
    meta.title = m.title || undefined
    meta.author = m.author || undefined
    meta.producer = m.creator || m.producer || undefined
  } catch {
    /* metadata is optional */
  }
  const version = scratch(4)
  if (mod.FPDF_GetFileVersion(ctx().docPtr, version)) {
    const v = mod.pdfium.getValue(version, 'i32')
    meta.version = `${Math.floor(v / 10)}.${v % 10}`
  }
  free(version)
  return { pages, meta, encrypted: opened.isEncrypted, hasForm: hasForms }
}

const save = () => task(engine.saveAsCopy(doc!))

/**
 * Flattening keeps what prints, so in the copy, marks shown on screen are made to print too
 * (files from other apps and earlier versions often lack the flag), and marks that will be baked
 * in but have no drawing of their own get one, as the editor draws them.
 */
function printShownMarks(pagePtr: number) {
  const count = mod.FPDFPage_GetAnnotCount(pagePtr)
  for (let i = 0; i < count; i++) {
    const annot = mod.FPDFPage_GetAnnot(pagePtr, i)
    if (!annot) continue
    try {
      const subtype = mod.FPDFAnnot_GetSubtype(annot)
      // Form fields are flattened as they print; links and popups have nothing to draw.
      if (subtype === PdfAnnotationSubtype.WIDGET || subtype === PdfAnnotationSubtype.LINK || subtype === PdfAnnotationSubtype.POPUP) continue
      let flags = mod.FPDFAnnot_GetFlags(annot)
      if (flags & PdfAnnotationFlags.HIDDEN) continue
      if (!(flags & (PdfAnnotationFlags.PRINT | PdfAnnotationFlags.NO_VIEW))) {
        flags |= PdfAnnotationFlags.PRINT
        mod.FPDFAnnot_SetFlags(annot, flags)
      }
      if (flags & PdfAnnotationFlags.PRINT && !mod.EPDFAnnot_HasAppearanceStream(annot, AppearanceMode.Normal)) mod.EPDFAnnot_GenerateAppearance(annot)
    } finally {
      mod.FPDFPage_CloseAnnot(annot)
    }
  }
}

/**
 * Save, open the result as a second document, bake every page's annotations and form fields
 * into its content (what prints, plus marks shown on screen; hidden ones go), and save that.
 * The open document is untouched.
 */
async function flatten(): Promise<ArrayBuffer> {
  const bytes = await save()
  const copy = await task(engine.openDocumentBuffer({ id: FLAT_ID, content: bytes }, { password: openPassword }))
  try {
    const c = internals.cache.getContext(FLAT_ID)
    if (!c) throw new Error('Couldn’t make the copy')
    for (let i = 0; i < copy.pageCount; i++) {
      const pc = c.acquirePage(i)
      try {
        printShownMarks(pc.pagePtr)
        // A page with nothing to flatten reports NothingToDo, which is fine.
        if (mod.FPDFPage_Flatten(pc.pagePtr, PdfPageFlattenFlag.Print) === PdfPageFlattenResult.Fail) throw new Error(`Couldn’t flatten page ${i + 1}`)
      } finally {
        pc.release()
      }
    }
    c.pageCache.forceReleaseAll()
    return await task(engine.saveAsCopy(copy))
  } finally {
    await task(engine.closeDocument(copy)).catch(() => {})
  }
}

// ── Message loop

const api: { [M in Method]: (...args: never[]) => unknown } = {
  open,
  render,
  renderGrayscale,
  text,
  pageTextStats,
  pageTextStatsBatch,
  documentTextStats,
  textBoxes,
  layout,
  annotations,
  createAnnotation,
  updateAnnotation,
  removeAnnotation,
  restoreAnnotation,
  setField,
  search,
  searchBatch,
  outline,
  rotate,
  move,
  duplicate,
  insertBlank,
  insertPdf,
  deletePages,
  restorePages,
  extract,
  save,
  flatten,
  memory: () => heap().HEAPU8.length,
} satisfies { [M in Method]: (...args: Parameters<Api[M]>) => ReturnType<Api[M]> | Promise<ReturnType<Api[M]>> }

/** Requests can arrive before the boot message (the module compiles meanwhile); they wait for it. */
let startBoot!: (msg: Boot) => void
const ready = new Promise<void>((resolve, reject) => {
  startBoot = (msg) => void boot(msg).then(resolve, reject)
})
ready.catch((err) => console.error('PDFium failed to start', err))
const queue: { id: number; method: Method; args: unknown[]; priority: number }[] = []
let running = false
// Yield between requests through a message channel (setTimeout would add 4 ms each time).
const tick = new MessageChannel()
tick.port1.onmessage = () => void runNext()

async function runNext() {
  if (running || !queue.length) return
  running = true
  await ready.catch(() => {})
  if (!queue.length) {
    // Everything waiting was cancelled meanwhile.
    running = false
    return
  }
  let best = 0
  for (let i = 1; i < queue.length; i++) if (queue[i].priority < queue[best].priority) best = i
  const [req] = queue.splice(best, 1)
  const response: Response = { id: req.id }
  const transfer: Transferable[] = []
  try {
    await ready
    const result = await (api[req.method] as (...a: unknown[]) => unknown)(...req.args)
    response.result = result
    if (result instanceof ArrayBuffer) transfer.push(result)
    else if (result && typeof result === 'object' && 'bands' in result) {
      for (const band of (result as Bitmap).bands) transfer.push((band.image ?? band.data)!)
    } else if (result && typeof result === 'object' && 'data' in result && result.data instanceof ArrayBuffer) {
      transfer.push(result.data)
    } else if (result && typeof result === 'object' && 'pictures' in result) {
      for (const picture of (result as PageLayout).pictures) transfer.push(picture.data)
    }
  } catch (err) {
    const e = err as Error & { code?: number }
    response.error = { message: e.message || String(err), code: e.code }
  }
  self.postMessage(response, transfer)
  running = false
  if (queue.length) tick.port2.postMessage(0)
}

self.onmessage = (e: MessageEvent<Request | Boot>) => {
  const msg = e.data
  if ('boot' in msg) {
    startBoot(msg)
    return
  }
  if ('cancel' in msg) {
    const i = queue.findIndex((q) => q.id === msg.cancel)
    if (i !== -1) {
      queue.splice(i, 1)
      self.postMessage({ id: msg.cancel, error: { message: 'cancelled', code: -1 } } satisfies Response)
    }
    return
  }
  queue.push(msg)
  tick.port2.postMessage(0)
}

async function boot({ module, wasmUrl }: Boot) {
  const overrides: Record<string, unknown> = {}
  if (module) {
    overrides.instantiateWasm = (imports: WebAssembly.Imports, done: (instance: WebAssembly.Instance, module: WebAssembly.Module) => void) => {
      void WebAssembly.instantiate(module, imports).then((instance) => done(instance, module))
      return {}
    }
  } else {
    overrides.wasmBinary = await (await fetch(wasmUrl)).arrayBuffer()
  }
  mod = await init(overrides)
  engine = new PdfiumNative(mod, { fontFallback: null })
  internals = engine as unknown as NativeInternals
  // Open pages keep their decoded images; let them go soon after use.
  internals.cache.updateConfig({ maxPagesPerDocument: 4, pageTtl: 2000 })
}
