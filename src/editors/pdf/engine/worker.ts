/// <reference lib="webworker" />
// One PDFium (WebAssembly) instance per open PDF. Requests are queued by priority, so drawing
// the visible pages always comes before thumbnails or search, and the editor can cancel work
// for pages that scrolled away before it starts.

import { PdfiumNative } from '@embedpdf/engines/pdfium'
import type { PdfAnnotationObject, PdfDocumentObject, PdfPageObject, PdfBookmarkObject, PdfLinkTarget, PdfWidgetAnnoObject, Rect } from '@embedpdf/models'
import { AppearanceMode, PDF_FORM_FIELD_FLAG, PDF_FORM_FIELD_TYPE, PdfActionType, PdfAnnotationFlags, PdfAnnotationSubtype, PdfPageFlattenFlag, PdfPageFlattenResult, PdfZoomMode } from '@embedpdf/models'
import { init } from '@embedpdf/pdfium'
import type { Annot, Api, Band, Bitmap, Boot, Box, DocMeta, Field, Method, OpenResult, OutlineItem, PageInfo, Request, Response, TextRun } from './protocol'

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
    let run: { text: string; map: number[]; x0: number; y0: number; x1: number; y1: number; lastX: number; vertical: boolean } | null = null
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
          : midY > run.y0 && midY < run.y1 && box.x >= run.lastX - h * 0.5 && box.x - run.x1 < h * 3
        if (!sameLine) {
          // A single character can't tell us the direction yet; a second one straight below makes it vertical.
          const below = run.text.length === 1 && Math.abs(box.x - run.x0) < box.w * 0.5 && box.y >= run.y1 - 1
          if (below) run.vertical = true
          else flush()
        } else if (!run.vertical && box.x - run.x1 > h * 0.3 && !/\s$/.test(run.text) && !/\s/.test(ch)) {
          run.text += ' '
          run.map.push(i)
        }
      }
      if (!run) run = { text: '', map: [], x0: box.x, y0: box.y, x1: box.x + box.w, y1: box.y + box.h, lastX: box.x, vertical: false }
      run.text += ch
      // One entry per UTF-16 unit, so characters outside the BMP take two.
      for (let u = 0; u < ch.length; u++) run.map.push(i)
      run.x0 = Math.min(run.x0, box.x)
      run.y0 = Math.min(run.y0, box.y)
      run.x1 = Math.max(run.x1, box.x + box.w)
      run.y1 = Math.max(run.y1, box.y + box.h)
      run.lastX = box.x + box.w
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
  text,
  textBoxes,
  annotations,
  createAnnotation,
  updateAnnotation,
  removeAnnotation,
  restoreAnnotation,
  setField,
  search,
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
