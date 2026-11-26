// One-click file conversion (Office formats, clean LLM-ready Markdown, PDF) and bundling
// multiple open tabs into a single PDF.

import type { TextRun } from '../editors/pdf/engine/protocol'
import { loadEngine } from '../engine/load'
import type { DocKind } from './kinds'
import { KINDS } from './kinds'
import * as platform from './platform'
import type { PdfOcrProgress } from './ocr/pdfOcr'
import type { DocTab } from '../state/workspace'
import { getEditor, useWorkspace } from '../state/workspace'

export type ConvertFormat = 'pdf' | 'md' | 'docx' | 'pptx' | 'xlsx' | 'csv' | 'txt'

export interface ConvertTarget {
  format: ConvertFormat
  label: string
  hint: string
  extension: string
}

const TARGET_INFO: Record<ConvertFormat, Omit<ConvertTarget, 'format'>> = {
  md: { label: 'Markdown (LLM-ready)', hint: 'Clean headings, lists, tables and formatting', extension: 'md' },
  pdf: { label: 'PDF', hint: 'Rendered pages as a PDF document', extension: 'pdf' },
  docx: { label: 'Word (.docx)', hint: 'Editable Word document', extension: 'docx' },
  pptx: { label: 'PowerPoint (.pptx)', hint: 'Slides deck', extension: 'pptx' },
  xlsx: { label: 'Excel (.xlsx)', hint: 'Spreadsheet workbook', extension: 'xlsx' },
  csv: { label: 'CSV (.csv)', hint: 'Comma-separated values', extension: 'csv' },
  txt: { label: 'Plain text (.txt)', hint: 'Unformatted text', extension: 'txt' },
}

const TARGETS_BY_KIND: Record<string, ConvertFormat[]> = {
  docx: ['md', 'pdf', 'pptx', 'xlsx', 'txt'],
  pptx: ['pdf', 'md', 'docx', 'txt'],
  xlsx: ['pdf', 'md', 'csv', 'docx', 'pptx'],
  csv: ['xlsx', 'md', 'pdf', 'docx'],
  md: ['pdf', 'docx', 'pptx', 'txt'],
  txt: ['pdf', 'docx', 'md'],
  pdf: ['md', 'txt', 'docx'],
  image: ['pdf', 'txt', 'md', 'docx'],
}

/** Target formats a tab of `kind` can be converted to in one click. */
export function targetsForKind(kind: DocKind): ConvertTarget[] {
  const list = TARGETS_BY_KIND[kind] ?? []
  return list.map((format) => ({
    format, ...TARGET_INFO[format],
    ...(usesOcr(kind, format) ? { hint: 'Recognize text with offline OCR' } : {}),
  }))
}

/** Whether converting a `kind` tab to `format` recognizes its text. A picture becomes a plain PDF. */
export function usesOcr(kind: DocKind, format: ConvertFormat): boolean {
  return (kind === 'image' && format !== 'pdf') || (kind === 'pdf' && format === 'docx')
}

export type OcrStatus = (message: string) => void

const packDownloads = new Map<string, { done: Promise<void>; callers: number }>()

/** Download absent or damaged packs before starting the offline recognizer. */
export async function prepareOcrPacks(langs: string[], signal?: AbortSignal, status?: OcrStatus): Promise<void> {
  const [{ normalizeOcrLangs, useOcrPacks }, { formatBytes, getPackMeta }] = await Promise.all([
    import('./ocr/packs'),
    import('./ocr/catalog'),
  ])
  const codes = normalizeOcrLangs(langs)
  const store = useOcrPacks.getState()
  status?.('Checking language packs…')
  const { installed } = await store.refreshPacks()
  for (const code of codes) {
    signal?.throwIfAborted()
    if ([...installed, ...useOcrPacks.getState().installed].some((pack) => pack.code === code && pack.sha256Verified)) continue
    const meta = getPackMeta(code)!
    const update = () => {
      const progress = useOcrPacks.getState().progress[code]
      status?.(`Downloading ${meta.name} · ${formatBytes(progress?.received ?? 0)} / ${formatBytes(meta.size)}`)
    }
    let download = packDownloads.get(code)
    const alreadyDownloading = !!useOcrPacks.getState().progress[code]
    if (!download && !alreadyDownloading) {
      download = {
        done: store.downloadPack(code).finally(() => packDownloads.delete(code)),
        callers: 0,
      }
      packDownloads.set(code, download)
    }
    if (download) download.callers++
    const unsubscribe = useOcrPacks.subscribe(update)
    update()
    try {
      await new Promise<void>((resolve, reject) => {
        const stop = download ? undefined : useOcrPacks.subscribe(({ progress }) => {
          if (!progress[code]) finish(resolve)
        })
        const finish = (settle: () => void) => {
          signal?.removeEventListener('abort', abort)
          stop?.()
          settle()
        }
        const abort = () => finish(() => reject(signal?.reason))
        signal?.addEventListener('abort', abort, { once: true })
        if (download) void download.done.then(() => finish(resolve), (err) => finish(() => reject(err)))
        if (signal?.aborted) abort()
      })
    } finally {
      unsubscribe()
      if (download && --download.callers === 0 && signal?.aborted && packDownloads.get(code) === download) {
        void store.cancelDownload(code).catch(() => {})
      }
    }
  }
  await store.ensurePacks(codes, signal)
}

export function pdfOcrStatus(progress: PdfOcrProgress): string {
  const { phase, done, total, pageIndex } = progress
  if (phase === 'inspecting') return 'Inspecting PDF…'
  if (phase === 'injecting') return `Building searchable PDF · ${done} / ${total} pages`
  return `Recognizing page ${pageIndex + 1} · ${done} / ${total} complete`
}

export async function recognizeImage(bytes: Uint8Array, name: string, signal?: AbortSignal, status?: OcrStatus) {
  const { useOcrPacks } = await import('./ocr/packs')
  const langs = useOcrPacks.getState().selectedLangs
  await prepareOcrPacks(langs, signal, status)
  const { ocrImageBytes } = await import('./ocr/pdfOcr')
  status?.('Recognizing text…')
  return await ocrImageBytes(bytes, name, langs, signal)
}

/** An OCR export picks a fresh destination, so it cannot replace its source. */
export async function saveOcrCopy(name: string, bytes: Uint8Array, signal?: AbortSignal): Promise<boolean> {
  signal?.throwIfAborted()
  if (!platform.isTauri) return platform.saveBytesAs(name, bytes)
  const path = await platform.pickSavePath(name, name.split('.').pop()!)
  if (!path) return false
  if (await platform.pathExists(path)) throw new Error('Choose a new file name for the OCR copy.')
  signal?.throwIfAborted()
  await platform.writeFile(path, bytes, { noClobber: true })
  return true
}

/** Whether a tab can be exported or copied as clean Markdown. */
export function canExportMarkdown(kind: DocKind): boolean {
  return kind === 'docx' || kind === 'pptx' || kind === 'xlsx' || kind === 'csv' || kind === 'pdf' || kind === 'md' || kind === 'txt'
}

/** Whether a tab can be included in a bundled PDF. */
export function canIncludeInPdfBundle(kind: DocKind): boolean {
  return kind === 'pdf' || kind === 'docx' || kind === 'pptx' || kind === 'xlsx' || kind === 'csv' || kind === 'md' || kind === 'txt' || kind === 'image'
}

function fileNameForTab(tab: DocTab): string {
  if (tab.path && !tab.path.startsWith('browser:')) return platform.basename(tab.path)
  const ext = KINDS[tab.kind]?.extensions[0] ?? tab.kind
  return `${tab.title}.${ext}`
}

/** Reads the live bytes of an open tab (including unsaved edits if its editor is mounted). */
export async function readTabBytes(tab: DocTab): Promise<Uint8Array> {
  const editor = getEditor(tab.id)
  if (editor) {
    editor.prepareSave?.()
    return await editor.getBytes()
  }
  if (tab.bytes) return tab.bytes
  if (tab.path) return await platform.readFile(tab.path)
  return new Uint8Array(0)
}

/** Converts an open tab into clean Markdown suitable for feeding to LLMs. */
export async function tabToMarkdown(tab: DocTab): Promise<string> {
  const bytes = await readTabBytes(tab)
  if (tab.kind === 'pdf') return await pdfToMarkdown(bytes, getEditor(tab.id)?.getPassword?.() ?? '')
  const engine = await loadEngine()
  return engine.toMarkdown(bytes, fileNameForTab(tab))
}

/** Converts an open tab to `target` format bytes. */
interface ConvertOptions {
  signal?: AbortSignal
  onProgress?: OcrStatus
}

export async function convertTab(tab: DocTab, target: ConvertFormat, options: ConvertOptions = {}): Promise<{ bytes: Uint8Array; name: string }> {
  const name = `${tab.title}.${TARGET_INFO[target].extension}`
  const { signal, onProgress } = options
  if (tab.kind === 'image' && target !== 'pdf') {
    const result = await recognizeImage(await readTabBytes(tab), fileNameForTab(tab), signal, onProgress)
    const { ocrResultsToPlainText, ocrResultsToMarkdown, ocrResultsToDocx } = await import('./ocr/toDocx')
    const pages = [result.page]
    if (target === 'docx') return { bytes: await ocrResultsToDocx(pages, tab.title), name }
    const text = target === 'md' ? ocrResultsToMarkdown(pages) : ocrResultsToPlainText(pages)
    return { bytes: new TextEncoder().encode(text), name }
  }
  if (tab.kind === 'pdf' && target === 'docx') {
    // Only the pages are read: a Word copy leaves the PDF, and any signature on it, as it was.
    const pdfBytes = await readTabBytes(tab)
    const { useOcrPacks } = await import('./ocr/packs')
    const langs = useOcrPacks.getState().selectedLangs
    await prepareOcrPacks(langs, signal, onProgress)
    const { ocrPdfDocument } = await import('./ocr/pdfOcr')
    const result = await ocrPdfDocument({
      pdfBytes, langs, signal, skipPagesWithText: false, searchable: false,
      onProgress: (progress) => onProgress?.(pdfOcrStatus(progress)),
    })
    signal?.throwIfAborted()
    onProgress?.('Building Word document…')
    const { ocrResultsToDocx } = await import('./ocr/toDocx')
    return { bytes: await ocrResultsToDocx(result.pages, tab.title), name }
  }
  if (target === 'pdf') {
    const bytes = await tabToPdf(tab)
    return { bytes, name }
  }
  const srcBytes = await readTabBytes(tab)
  if (tab.kind === 'pdf') {
    const md = await pdfToMarkdown(srcBytes, getEditor(tab.id)?.getPassword?.() ?? '')
    const text = target === 'md' ? md : md.replace(/^## Page \d+\n\n/gm, '')
    return { bytes: new TextEncoder().encode(text), name }
  }
  const engine = await loadEngine()
  const out = engine.convertDocument(srcBytes, fileNameForTab(tab), target)
  return { bytes: new Uint8Array(out), name }
}

/**
 * One-click convert: converts the tab (or active tab) to `target`, then either opens the result
 * as a new unsaved tab or prompts to save it directly to disk.
 */
export async function convertAndDeliver(tabId: string | undefined, target: ConvertFormat, mode: 'open' | 'save' = 'open', options?: ConvertOptions): Promise<boolean> {
  const ws = useWorkspace.getState()
  const tab = ws.tabs.find((t) => t.id === (tabId ?? ws.activeId))
  if (!tab) return false
  try {
    const { bytes, name } = await convertTab(tab, target, options)
    options?.signal?.throwIfAborted()
    const save = usesOcr(tab.kind, target) ? saveOcrCopy : platform.saveBytesAs
    if (mode === 'save') {
      return await save(name, bytes, options?.signal)
    }
    const opened = ws.openUnsaved(bytes, name)
    if (!opened) {
      return await save(name, bytes, options?.signal)
    }
    return true
  } catch (err) {
    if (options?.signal?.aborted || (err instanceof DOMException && err.name === 'AbortError')) return false
    if (options) throw err
    await platform.showError(err instanceof Error ? err.message : String(err), 'Couldn’t convert file')
    return false
  }
}

/** Copies the active (or specified) tab as clean LLM-ready Markdown to the clipboard. */
export async function copyTabAsMarkdown(tabId?: string): Promise<string | null> {
  const ws = useWorkspace.getState()
  const tab = ws.tabs.find((t) => t.id === (tabId ?? ws.activeId))
  if (!tab || !canExportMarkdown(tab.kind)) return null
  const md = await tabToMarkdown(tab)
  await navigator.clipboard.writeText(md)
  return md
}

// ── PDF rendering per document kind

/** Renders any supported open tab to PDF bytes. */
export async function tabToPdf(tab: DocTab, title?: string): Promise<Uint8Array> {
  const bytes = await readTabBytes(tab)
  const docTitle = title ?? tab.title
  switch (tab.kind) {
    case 'pdf':
      return bytes
    case 'docx':
      return await docxBytesToPdf(bytes, docTitle)
    case 'pptx':
      return await pptxBytesToPdf(bytes, docTitle)
    case 'xlsx':
    case 'csv':
      return await sheetBytesToPdf(bytes, fileNameForTab(tab))
    case 'md':
    case 'txt': {
      const engine = await loadEngine()
      const docxBytes = new Uint8Array(engine.convertDocument(bytes, fileNameForTab(tab), 'docx'))
      return await docxBytesToPdf(docxBytes, docTitle)
    }
    case 'image':
      return await imageBytesToPdf(bytes, fileNameForTab(tab))
    default:
      throw new Error(`“${tab.title}” (${tab.kind}) can’t be exported to PDF yet.`)
  }
}

async function docxBytesToPdf(bytes: Uint8Array, title?: string): Promise<Uint8Array> {
  const { exportWordToPdf } = await import('./pdf/exporter')
  return await exportWordToPdf(bytes, title)
}

async function pptxBytesToPdf(bytes: Uint8Array, title?: string): Promise<Uint8Array> {
  const { exportSlidesToPdf } = await import('./pdf/exporter')
  return await exportSlidesToPdf(bytes, title)
}

async function sheetBytesToPdf(bytes: Uint8Array, name: string): Promise<Uint8Array> {
  const { exportSheetsToPdf } = await import('./pdf/exporter')
  return await exportSheetsToPdf(bytes, name)
}

async function imageBytesToPdf(bytes: Uint8Array, name: string): Promise<Uint8Array> {
  const { exportImageToPdf } = await import('./pdf/exporter')
  return await exportImageToPdf(bytes, name)
}

// ── PDFium lossless merge & text extraction

export type PdfInput = Uint8Array | { bytes: Uint8Array; password?: string }

/** Merges multiple PDF byte buffers into a single PDF using PDFium. */
export async function mergePdfBytes(pdfs: PdfInput[]): Promise<Uint8Array> {
  if (pdfs.length === 0) throw new Error('Choose at least one document to merge.')
  const norm = pdfs.map((p) => (p instanceof Uint8Array ? { bytes: p, password: '' } : { bytes: p.bytes, password: p.password ?? '' }))
  if (norm.length === 1) return norm[0].bytes
  const { PdfEngine } = await import('../editors/pdf/engine/client')
  const engine = await PdfEngine.create()
  try {
    const first = await engine.call('open', [norm[0].bytes.slice().buffer, norm[0].password])
    let pageCount = first.pages.length
    for (let i = 1; i < norm.length; i++) {
      const res = await engine.call('insertPdf', [pageCount, norm[i].bytes.slice().buffer, norm[i].password])
      pageCount = res.pages.length
    }
    const saved = await engine.call('save', [])
    return new Uint8Array(saved)
  } finally {
    engine.destroy()
  }
}

/** Bundles the given open tabs (in order) into a single merged PDF. */
export async function bundleTabsToPdf(
  tabs: DocTab[],
  onProgress?: (done: number, total: number, currentTitle: string) => void,
): Promise<Uint8Array> {
  if (!tabs.length) throw new Error('Choose at least one open file to bundle.')
  const pdfs: PdfInput[] = []
  for (let i = 0; i < tabs.length; i++) {
    onProgress?.(i, tabs.length, tabs[i].title)
    const bytes = await tabToPdf(tabs[i])
    const password = tabs[i].kind === 'pdf' ? (getEditor(tabs[i].id)?.getPassword?.() ?? '') : ''
    pdfs.push({ bytes, password })
  }
  onProgress?.(tabs.length, tabs.length, 'Merging PDF…')
  return await mergePdfBytes(pdfs)
}

/** Extracts clean Markdown text from a PDF using PDFium. */
async function pdfToMarkdown(bytes: Uint8Array, password = ''): Promise<string> {
  const { PdfEngine } = await import('../editors/pdf/engine/client')
  const engine = await PdfEngine.create()
  try {
    const opened = await engine.call('open', [bytes.slice().buffer, password])
    const multi = opened.pages.length > 1
    const sections: string[] = []
    for (let i = 0; i < opened.pages.length; i++) {
      const runs = await engine.call('text', [opened.pages[i].key])
      const text = formatPdfPageRuns(runs)
      if (!text) continue
      sections.push(multi ? `## Page ${i + 1}\n\n${text}` : text)
    }
    return sections.length ? `${sections.join('\n\n')}\n` : ''
  } finally {
    engine.destroy()
  }
}

function formatPdfPageRuns(runs: TextRun[]): string {
  const sorted = [...runs]
    .filter((r) => r.text.trim().length > 0)
    .sort((a, b) => (Math.abs(a.box.y - b.box.y) > 4 ? a.box.y - b.box.y : a.box.x - b.box.x))
  const lines: string[] = []
  let currentY: number | null = null
  let currentLine = ''
  for (const r of sorted) {
    if (currentY !== null && Math.abs(r.box.y - currentY) > 4) {
      if (currentLine.trim()) lines.push(currentLine.trim())
      currentLine = r.text
      currentY = r.box.y
    } else {
      currentLine = currentLine ? `${currentLine} ${r.text}` : r.text
      currentY ??= r.box.y
    }
  }
  if (currentLine.trim()) lines.push(currentLine.trim())
  return lines.join('\n')
}
