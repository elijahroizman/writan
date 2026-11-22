// One-click file conversion (Office formats, clean LLM-ready Markdown, PDF) and bundling
// multiple open tabs into a single PDF.

import type { TextRun } from '../editors/pdf/engine/protocol'
import { loadEngine } from '../engine/load'
import type { PaintItem, PaintStyle } from '../engine/paint'
import type { DocKind } from './kinds'
import { KINDS } from './kinds'
import * as platform from './platform'
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
  pdf: ['md', 'txt'],
  image: ['pdf'],
  latex: ['pdf'],
}

/** Target formats a tab of `kind` can be converted to in one click. */
export function targetsForKind(kind: DocKind): ConvertTarget[] {
  const list = TARGETS_BY_KIND[kind] ?? []
  return list.map((format) => ({ format, ...TARGET_INFO[format] }))
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
export async function convertTab(tab: DocTab, target: ConvertFormat): Promise<{ bytes: Uint8Array; name: string }> {
  const name = `${tab.title}.${TARGET_INFO[target].extension}`
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
export async function convertAndDeliver(tabId: string | undefined, target: ConvertFormat, mode: 'open' | 'save' = 'open'): Promise<boolean> {
  const ws = useWorkspace.getState()
  const tab = ws.tabs.find((t) => t.id === (tabId ?? ws.activeId))
  if (!tab) return false
  try {
    const { bytes, name } = await convertTab(tab, target)
    if (mode === 'save') {
      return await platform.saveBytesAs(name, bytes)
    }
    const opened = ws.openUnsaved(bytes, name)
    if (!opened) {
      return await platform.saveBytesAs(name, bytes)
    }
    return true
  } catch (err) {
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

/** Pixels (96 DPI CSS) to PDF points (72 DPI). */
const PX_TO_PT = 72 / 96
const RENDER_SCALE = 2

/** Renders any supported open tab to PDF bytes. */
export async function tabToPdf(tab: DocTab): Promise<Uint8Array> {
  const bytes = await readTabBytes(tab)
  switch (tab.kind) {
    case 'pdf':
      return bytes
    case 'docx':
      return await docxBytesToPdf(bytes)
    case 'pptx':
      return await pptxBytesToPdf(bytes)
    case 'xlsx':
    case 'csv':
      return await sheetBytesToPdf(bytes, fileNameForTab(tab))
    case 'md':
    case 'txt': {
      const engine = await loadEngine()
      const docxBytes = new Uint8Array(engine.convertDocument(bytes, fileNameForTab(tab), 'docx'))
      return await docxBytesToPdf(docxBytes)
    }
    case 'image':
      return await imageBytesToPdf(bytes, fileNameForTab(tab))
    default:
      throw new Error(`“${tab.title}” (${tab.kind}) can’t be exported to PDF yet.`)
  }
}

async function docxBytesToPdf(bytes: Uint8Array): Promise<Uint8Array> {
  const [{ jsPDF }, { draw }, { WordEngine }] = await Promise.all([
    import('jspdf'),
    import('../collab/agents/picture'),
    import('../engine/word/engine'),
  ])
  const engine = await WordEngine.open(bytes.length ? bytes : null)
  try {
    const sizes = engine.sizes
    if (!sizes.length) throw new Error('This document has no pages to export.')
    const firstW = sizes[0][0] * PX_TO_PT
    const firstH = sizes[0][1] * PX_TO_PT
    const pdf = new jsPDF({
      unit: 'pt',
      format: [firstW, firstH],
      orientation: firstW > firstH ? 'landscape' : 'portrait',
      compress: true,
    })
    for (let i = 0; i < sizes.length; i++) {
      const p = engine.paint(i, true)
      if (!p) continue
      const wPt = p.width * PX_TO_PT
      const hPt = p.height * PX_TO_PT
      if (i > 0) pdf.addPage([wPt, hPt], wPt > hPt ? 'landscape' : 'portrait')
      const canvas = await draw({ width: p.width, height: p.height, items: p.items, styles: engine.styles, media: engine.media }, RENDER_SCALE)
      const dataUrl = canvas.toDataURL('image/jpeg', 0.94)
      pdf.addImage(dataUrl, 'JPEG', 0, 0, wPt, hPt, undefined, 'FAST')
    }
    return new Uint8Array(pdf.output('arraybuffer'))
  } finally {
    engine.dispose()
  }
}

async function pptxBytesToPdf(bytes: Uint8Array): Promise<Uint8Array> {
  const [{ jsPDF }, { draw }, { SlidesEngine }] = await Promise.all([
    import('jspdf'),
    import('../collab/agents/picture'),
    import('../engine/slides/engine'),
  ])
  const engine = await SlidesEngine.open(bytes.length ? bytes : null)
  try {
    const visible = engine.slides.map((s, i) => ({ s, i })).filter(({ s }) => !s.hidden)
    if (!visible.length) throw new Error('This presentation has no visible slides.')
    const wPt = engine.width * PX_TO_PT
    const hPt = engine.height * PX_TO_PT
    const pdf = new jsPDF({
      unit: 'pt',
      format: [wPt, hPt],
      orientation: wPt > hPt ? 'landscape' : 'portrait',
      compress: true,
    })
    for (let idx = 0; idx < visible.length; idx++) {
      const slideIdx = visible[idx].i
      if (idx > 0) pdf.addPage([wPt, hPt], wPt > hPt ? 'landscape' : 'portrait')
      const canvas = await draw(
        { width: engine.width, height: engine.height, items: engine.paintItems(slideIdx), styles: engine.styles, media: engine.media },
        RENDER_SCALE,
      )
      const dataUrl = canvas.toDataURL('image/jpeg', 0.94)
      pdf.addImage(dataUrl, 'JPEG', 0, 0, wPt, hPt, undefined, 'FAST')
    }
    return new Uint8Array(pdf.output('arraybuffer'))
  } finally {
    engine.dispose()
  }
}

async function sheetBytesToPdf(bytes: Uint8Array, name: string): Promise<Uint8Array> {
  const [{ jsPDF }, { draw }, { SheetsEngine }] = await Promise.all([
    import('jspdf'),
    import('../collab/agents/picture'),
    import('../engine/sheets/engine'),
  ])
  const engine = await SheetsEngine.open(bytes.length ? bytes : null, name)
  try {
    const visible = engine.sheets.map((s, i) => ({ s, i })).filter(({ s }) => !s.hidden)
    if (!visible.length) throw new Error('This workbook has no visible sheets.')

    // Landscape Letter (792 x 612 pt) with 36pt margins.
    const pageW = 792
    const pageH = 612
    const margin = 36
    const innerW = pageW - margin * 2
    const innerH = pageH - margin * 2

    const pdf = new jsPDF({ unit: 'pt', format: 'letter', orientation: 'landscape', compress: true })
    let pagesAdded = 0

    for (const { i: sheetIdx } of visible) {
      const g = engine.geometryOf(sheetIdx)
      const [contentRows, contentCols] = g.info.content
      const rows = Math.max(1, contentRows)
      const cols = Math.max(1, contentCols)
      const raw = JSON.parse(engine.doc.paint(sheetIdx, 0, rows, 0, cols)) as { items: PaintItem[]; styles: PaintStyle[] }
      engine.styles.add(raw.styles)
      const pictures = engine.picturesOf(sheetIdx)
      const charts = engine.chartsOf(sheetIdx)
      const items: PaintItem[] = [
        ...raw.items,
        ...pictures.map((p): PaintItem => ({ t: 'image', media: p.media, x: p.x, y: p.y, w: p.w, h: p.h, crop: p.crop })),
      ]
      let usedW = Math.max(1, g.x(cols))
      let usedH = Math.max(1, g.y(rows))
      for (const b of [...pictures, ...charts]) {
        usedW = Math.max(usedW, b.x + b.w)
        usedH = Math.max(usedH, b.y + b.h)
      }

      // Scale width to fit cleanly within page width, then paginate vertically.
      const fitScalePt = Math.min(PX_TO_PT, innerW / usedW)
      const slicePxH = Math.max(1, Math.floor(innerH / fitScalePt))

      let chartTools: {
        createElement: typeof import('react').createElement
        renderToStaticMarkup: typeof import('react-dom/server').renderToStaticMarkup
        ChartSvg: typeof import('../engine/sheets/Chart').ChartSvg
      } | null = null
      if (charts.length) {
        const [react, reactDomServer, chartMod] = await Promise.all([
          import('react'),
          import('react-dom/server'),
          import('../engine/sheets/Chart'),
        ])
        chartTools = {
          createElement: react.createElement,
          renderToStaticMarkup: reactDomServer.renderToStaticMarkup,
          ChartSvg: chartMod.ChartSvg,
        }
      }

      for (let y = 0; y < usedH; y += slicePxH) {
        if (pagesAdded > 0) pdf.addPage('letter', 'landscape')
        pagesAdded++
        const h = Math.min(slicePxH, usedH - y)
        const canvas = await draw(
          { width: Math.max(g.width, usedW), height: Math.max(g.height, usedH), items, styles: engine.styles, media: engine.media },
          RENDER_SCALE,
          [],
          { x: 0, y, w: usedW, h },
        )
        if (chartTools) {
          const ctx = canvas.getContext('2d')!
          for (const c of charts) {
            if (c.y + c.h <= y || c.y >= y + h) continue
            const markup = chartTools.renderToStaticMarkup(chartTools.createElement(chartTools.ChartSvg, { chart: c, width: c.w, height: c.h }))
            const svg = markup
              .slice(markup.indexOf('<svg'), markup.lastIndexOf('</svg>') + 6)
              .replace('<svg', '<svg xmlns="http://www.w3.org/2000/svg"')
            const img = new Image()
            const svgUrl = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }))
            img.src = svgUrl
            try {
              await img.decode()
              ctx.drawImage(img, c.x, c.y, c.w, c.h)
            } catch {
              // Skip unrenderable chart SVG rather than failing export.
            } finally {
              URL.revokeObjectURL(svgUrl)
            }
          }
        }
        const drawW = usedW * fitScalePt
        const drawH = h * fitScalePt
        pdf.addImage(canvas.toDataURL('image/jpeg', 0.94), 'JPEG', margin, margin, drawW, drawH, undefined, 'FAST')
      }
    }

    return new Uint8Array(pdf.output('arraybuffer'))
  } finally {
    engine.dispose()
  }
}

function isHeicBytes(bytes: Uint8Array, ext: string): boolean {
  if (ext === 'heic' || ext === 'heif') return true
  if (bytes.length < 12) return false
  const box = String.fromCharCode(...bytes.slice(4, 12))
  return box.startsWith('ftyp') && /^(hei[cmsx]|hev[cmsx]|mif1|msf1)$/.test(box.slice(4))
}

async function decodeImageToCanvas(bytes: Uint8Array, name: string): Promise<HTMLCanvasElement> {
  const ext = name.split('.').pop()?.toLowerCase() ?? 'png'
  const heic = isHeicBytes(bytes, ext)
  const mime = heic
    ? 'image/heic'
    : ext === 'jpg' || ext === 'jpeg'
      ? 'image/jpeg'
      : ext === 'webp'
        ? 'image/webp'
        : ext === 'gif'
          ? 'image/gif'
          : 'image/png'
  const blob = new Blob([bytes as Uint8Array<ArrayBuffer>], { type: mime })
  const url = URL.createObjectURL(blob)
  try {
    const img = new Image()
    img.src = url
    await img.decode()
    const canvas = document.createElement('canvas')
    canvas.width = img.naturalWidth
    canvas.height = img.naturalHeight
    const ctx = canvas.getContext('2d')!
    ctx.fillStyle = '#ffffff'
    ctx.fillRect(0, 0, canvas.width, canvas.height)
    ctx.drawImage(img, 0, 0)
    return canvas
  } catch (err) {
    if (!heic) throw err
    // @ts-expect-error -- libheif-js ships no TypeScript declarations (see plugins/images/libheif.d.ts)
    const mod = (await import('libheif-js/libheif-wasm/libheif-bundle.mjs')) as {
      default: () => {
        HeifDecoder: new () => {
          decode(data: Uint8Array): Array<{
            is_primary(): boolean
            get_width(): number
            get_height(): number
            display(
              buf: { data: Uint8ClampedArray; width: number; height: number },
              cb: (out: { data: Uint8ClampedArray; width: number; height: number } | null) => void,
            ): void
            free(): void
          }>
        }
      }
    }
    const decoder = new (mod.default().HeifDecoder)()
    const images = decoder.decode(bytes)
    if (!images.length) throw new Error('This HEIC file has no picture in it.')
    const image = images.find((i) => i.is_primary()) ?? images[0]
    try {
      const width = image.get_width()
      const height = image.get_height()
      const imgData = await new Promise<ImageData>((resolve, reject) => {
        image.display({ data: new Uint8ClampedArray(width * height * 4), width, height }, (data) => {
          if (data) resolve(new ImageData(new Uint8ClampedArray(data.data), data.width, data.height))
          else reject(new Error('Could not decode this HEIC photo.'))
        })
      })
      const canvas = document.createElement('canvas')
      canvas.width = width
      canvas.height = height
      const ctx = canvas.getContext('2d')!
      ctx.fillStyle = '#ffffff'
      ctx.fillRect(0, 0, width, height)
      ctx.putImageData(imgData, 0, 0)
      return canvas
    } finally {
      for (const each of images) each.free()
    }
  } finally {
    URL.revokeObjectURL(url)
  }
}

async function imageBytesToPdf(bytes: Uint8Array, name: string): Promise<Uint8Array> {
  const { jsPDF } = await import('jspdf')
  const canvas = await decodeImageToCanvas(bytes, name)
  const wPt = Math.max(72, canvas.width * PX_TO_PT)
  const hPt = Math.max(72, canvas.height * PX_TO_PT)
  const pdf = new jsPDF({
    unit: 'pt',
    format: [wPt, hPt],
    orientation: wPt > hPt ? 'landscape' : 'portrait',
    compress: true,
  })
  pdf.addImage(canvas.toDataURL('image/jpeg', 0.95), 'JPEG', 0, 0, wPt, hPt, undefined, 'FAST')
  return new Uint8Array(pdf.output('arraybuffer'))
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
