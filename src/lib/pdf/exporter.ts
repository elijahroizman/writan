import { LATEX_MATH_PREFIX, renderLatexTagToSvgBytes } from '../../math/latexMath'
import type { PaintItem, PaintStyle, StyleSheet } from '../../engine/paint'
import type { ChartView, PictureView } from '../../engine/sheets/engine'
import { packBlobs } from './blobs'
import { pdfEngineClient } from './client'
import { loadFontsForDocument, loadFontsForOcrLayers } from './fonts'

const PX_TO_PT = 72 / 96

export interface OcrLayerInput {
  pageIndex: number
  pageWidthPt: number
  pageHeightPt: number
  words: Array<{
    text: string
    xPt: number
    yPt: number
    wPt: number
    hPt: number
    baselineYPt?: number
    rtl: boolean
  }>
}

interface PaintStyleInput {
  font: string
  color: string
  decoration?: string
  background?: string
  smallCaps: boolean
  shift?: number
  ascent?: number
  descent?: number
}

interface SvgOverlayInput {
  xPx: number
  yPx: number
  wPx: number
  hPx: number
  svg: string
}

interface PdfPageInput {
  widthPt: number
  heightPt: number
  scaleX: number
  scaleY: number
  offsetXPt: number
  offsetYPt: number
  clipRectPx?: [number, number, number, number]
  items: PaintItem[]
  svgs: SvgOverlayInput[]
}

const utf8Decoder = new TextDecoder('utf-8', { fatal: false })
const utf8Encoder = new TextEncoder()

function isJpeg(bytes: Uint8Array): boolean {
  return bytes.length >= 3 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff
}

function isPng(bytes: Uint8Array): boolean {
  return (
    bytes.length >= 8 &&
    bytes[0] === 0x89 &&
    bytes[1] === 0x50 &&
    bytes[2] === 0x4e &&
    bytes[3] === 0x47 &&
    bytes[4] === 0x0d &&
    bytes[5] === 0x0a &&
    bytes[6] === 0x1a &&
    bytes[7] === 0x0a
  )
}

function isSvg(bytes: Uint8Array, mime: string): boolean {
  if (mime.toLowerCase().includes('svg')) return true
  const head = utf8Decoder.decode(bytes.subarray(0, 256)).trimStart().replace(/^\uFEFF/, '')
  return head.startsWith('<svg') || (head.startsWith('<?xml') && head.includes('<svg'))
}

function isHeicBytes(bytes: Uint8Array, nameOrMime: string): boolean {
  const lower = nameOrMime.toLowerCase()
  if (lower.includes('heic') || lower.includes('heif')) return true
  if (bytes.length < 12) return false
  const box = String.fromCharCode(...bytes.slice(4, 12))
  return box.startsWith('ftyp') && /^(hei[cmsx]|hev[cmsx]|mif1|msf1)$/.test(box.slice(4))
}

function guessImageMime(bytes: Uint8Array, hint: string): string {
  if (isPng(bytes)) return 'image/png'
  if (isJpeg(bytes)) return 'image/jpeg'
  if (isSvg(bytes, hint)) return 'image/svg+xml'
  if (isHeicBytes(bytes, hint)) return 'image/heic'
  const lower = hint.toLowerCase()
  if (lower.includes('webp') || (bytes.length >= 12 && String.fromCharCode(...bytes.slice(0, 4)) === 'RIFF' && String.fromCharCode(...bytes.slice(8, 12)) === 'WEBP')) {
    return 'image/webp'
  }
  if (lower.includes('gif')) return 'image/gif'
  if (lower.includes('bmp')) return 'image/bmp'
  return hint || 'image/png'
}

export async function decodeImageToRgba(bytes: Uint8Array, hint: string): Promise<{ data: Uint8ClampedArray; width: number; height: number }> {
  const mime = guessImageMime(bytes, hint)
  const blob = new Blob([bytes as Uint8Array<ArrayBuffer>], { type: mime })
  if (typeof createImageBitmap === 'function' && mime !== 'image/heic') {
    try {
      const bitmap = await createImageBitmap(blob)
      try {
        const width = bitmap.width
        const height = bitmap.height
        if (typeof OffscreenCanvas !== 'undefined') {
          const canvas = new OffscreenCanvas(width, height)
          const ctx = canvas.getContext('2d')!
          ctx.drawImage(bitmap, 0, 0)
          const imgData = ctx.getImageData(0, 0, width, height)
          return { data: imgData.data, width, height }
        }
        const canvas = document.createElement('canvas')
        canvas.width = width
        canvas.height = height
        const ctx = canvas.getContext('2d')!
        ctx.drawImage(bitmap, 0, 0)
        const imgData = ctx.getImageData(0, 0, width, height)
        return { data: imgData.data, width, height }
      } finally {
        bitmap.close()
      }
    } catch {
      // Fall through to Image / libheif-js decoding below.
    }
  }
  if (mime !== 'image/heic') {
    const url = URL.createObjectURL(blob)
    try {
      const img = new Image()
      img.src = url
      await img.decode()
      const canvas = document.createElement('canvas')
      canvas.width = img.naturalWidth
      canvas.height = img.naturalHeight
      const ctx = canvas.getContext('2d')!
      ctx.drawImage(img, 0, 0)
      const imgData = ctx.getImageData(0, 0, canvas.width, canvas.height)
      return { data: imgData.data, width: canvas.width, height: canvas.height }
    } catch (err) {
      if (!isHeicBytes(bytes, hint)) throw err
    } finally {
      URL.revokeObjectURL(url)
    }
  }
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
    return await new Promise<{ data: Uint8ClampedArray; width: number; height: number }>((resolve, reject) => {
      image.display({ data: new Uint8ClampedArray(width * height * 4), width, height }, (data) => {
        if (data) resolve({ data: new Uint8ClampedArray(data.data), width: data.width, height: data.height })
        else reject(new Error('Could not decode this HEIC photo.'))
      })
    })
  } finally {
    for (const each of images) each.free()
  }
}

async function encodeRgbaToPng(rgba: { data: Uint8ClampedArray; width: number; height: number }): Promise<Uint8Array> {
  if (typeof OffscreenCanvas !== 'undefined') {
    const canvas = new OffscreenCanvas(rgba.width, rgba.height)
    const ctx = canvas.getContext('2d')!
    ctx.putImageData(new ImageData(new Uint8ClampedArray(rgba.data), rgba.width, rgba.height), 0, 0)
    const blob = await canvas.convertToBlob({ type: 'image/png' })
    return new Uint8Array(await blob.arrayBuffer())
  }
  const canvas = document.createElement('canvas')
  canvas.width = rgba.width
  canvas.height = rgba.height
  const ctx = canvas.getContext('2d')!
  ctx.putImageData(new ImageData(new Uint8ClampedArray(rgba.data), rgba.width, rgba.height), 0, 0)
  const blob = await new Promise<Blob>((resolve, reject) =>
    canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('Failed to encode PNG image.'))), 'image/png'),
  )
  return new Uint8Array(await blob.arrayBuffer())
}

/** Targeted single-element PNG conversion for formats not natively accepted by PDF (WebP/GIF/HEIC/TIFF). */
async function convertRasterToPng(bytes: Uint8Array, hint: string): Promise<{ bytes: Uint8Array; width: number; height: number }> {
  const rgba = await decodeImageToRgba(bytes, hint)
  const pngBytes = await encodeRgbaToPng(rgba)
  return { bytes: pngBytes, width: rgba.width, height: rgba.height }
}

function pngDimensions(bytes: Uint8Array): { width: number; height: number } | null {
  if (!isPng(bytes) || bytes.length < 24) return null
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  return { width: view.getUint32(16, false), height: view.getUint32(20, false) }
}

function jpegDimensions(bytes: Uint8Array): { width: number; height: number } | null {
  if (!isJpeg(bytes)) return null
  let i = 2
  while (i + 4 <= bytes.length) {
    if (bytes[i] !== 0xff) {
      i++
      continue
    }
    while (i < bytes.length && bytes[i] === 0xff) i++
    if (i >= bytes.length) break
    const marker = bytes[i++]
    if (marker === 0xd9 || marker === 0xda) break
    if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) continue
    if (i + 2 > bytes.length) break
    const len = (bytes[i] << 8) | bytes[i + 1]
    if (len < 2 || i + len > bytes.length) break
    if ((marker >= 0xc0 && marker <= 0xc3) || (marker >= 0xc5 && marker <= 0xc7) || (marker >= 0xc9 && marker <= 0xcb) || (marker >= 0xcd && marker <= 0xcf)) {
      if (len >= 7) {
        return {
          height: (bytes[i + 3] << 8) | bytes[i + 4],
          width: (bytes[i + 5] << 8) | bytes[i + 6],
        }
      }
    }
    i += len
  }
  return null
}

async function resolveImageDimensions(bytes: Uint8Array, mime: string): Promise<{ width: number; height: number }> {
  const png = pngDimensions(bytes)
  if (png && png.width > 0 && png.height > 0) return png
  const jpg = jpegDimensions(bytes)
  if (jpg && jpg.width > 0 && jpg.height > 0) return jpg
  if (typeof createImageBitmap === 'function') {
    try {
      const bm = await createImageBitmap(new Blob([bytes as Uint8Array<ArrayBuffer>], { type: mime }))
      try {
        return { width: bm.width, height: bm.height }
      } finally {
        bm.close()
      }
    } catch {
      // Fallback below
    }
  }
  const rgba = await decodeImageToRgba(bytes, mime)
  return { width: rgba.width, height: rgba.height }
}

interface CollectContext {
  usedStyleIds: Set<number>
  texts: string[]
  mediaKeys: Set<string>
  hasSvg: boolean
}

function walkItems(items: PaintItem[], ctx: CollectContext) {
  for (const it of items) {
    if (it.t === 'text') {
      ctx.usedStyleIds.add(it.s)
      if (it.text) ctx.texts.push(it.text)
    } else if (it.t === 'image') {
      ctx.mediaKeys.add(it.media)
      if (it.media.startsWith(LATEX_MATH_PREFIX)) ctx.hasSvg = true
    } else if (it.t === 'group') {
      walkItems(it.items, ctx)
    }
  }
}

function buildStyleTable(styleSheet: StyleSheet, usedStyleIds: Set<number>): { all: PaintStyleInput[]; used: PaintStyleInput[] } {
  let maxId = -1
  for (const id of usedStyleIds) if (id > maxId) maxId = id
  const fallback: PaintStyleInput = { font: '16px "Liberation Sans", sans-serif', color: '#000000', smallCaps: false }
  const all: PaintStyleInput[] = []
  const used: PaintStyleInput[] = []
  for (let i = 0; i <= maxId; i++) {
    const resolved = styleSheet.style(i)
    if (resolved) {
      const entry: PaintStyleInput = {
        font: resolved.style.font,
        color: resolved.style.color,
        decoration: resolved.style.decoration,
        background: resolved.style.background,
        smallCaps: !!resolved.style.smallCaps,
        shift: resolved.style.shift,
        ascent: resolved.ascent,
        descent: resolved.descent,
      }
      all.push(entry)
      if (usedStyleIds.has(i)) used.push(entry)
    } else {
      all.push(fallback)
    }
  }
  return { all, used }
}

async function resolveMediaPack(
  keys: Set<string>,
  lookup: (id: string) => { bytes: Uint8Array | undefined; type: string } | null,
): Promise<{ pack: Uint8Array; hasSvg: boolean }> {
  const entries: Array<[string, Uint8Array]> = []
  let hasSvg = false
  for (const id of keys) {
    if (id.startsWith(LATEX_MATH_PREFIX)) {
      const rendered = renderLatexTagToSvgBytes(id)
      if (rendered) {
        hasSvg = true
        entries.push([id, rendered.bytes])
      }
      continue
    }
    const raw = lookup(id)
    if (!raw?.bytes?.length) continue
    if (isSvg(raw.bytes, raw.type)) {
      hasSvg = true
      entries.push([id, raw.bytes])
    } else if (isJpeg(raw.bytes) || isPng(raw.bytes)) {
      entries.push([id, raw.bytes])
    } else {
      try {
        const converted = await convertRasterToPng(raw.bytes, raw.type)
        entries.push([id, converted.bytes])
      } catch {
        // Skip unreadable image rather than crashing export.
      }
    }
  }
  return { pack: packBlobs(entries), hasSvg }
}

export async function exportWordToPdf(bytes: Uint8Array, title?: string, signal?: AbortSignal): Promise<Uint8Array> {
  const { WordEngine } = await import('../../engine/word/engine')
  const engine = await WordEngine.open(bytes.length ? bytes : null)
  try {
    const sizes = engine.sizes
    if (!sizes.length) throw new Error('This document has no pages to export.')
    const ctx: CollectContext = { usedStyleIds: new Set(), texts: [], mediaKeys: new Set(), hasSvg: false }
    const pages: PdfPageInput[] = []
    for (let i = 0; i < sizes.length; i++) {
      signal?.throwIfAborted?.()
      const p = engine.paint(i, true)
      if (!p) continue
      walkItems(p.items, ctx)
      pages.push({
        widthPt: p.width * PX_TO_PT,
        heightPt: p.height * PX_TO_PT,
        scaleX: PX_TO_PT,
        scaleY: PX_TO_PT,
        offsetXPt: 0,
        offsetYPt: 0,
        items: p.items,
        svgs: [],
      })
    }
    if (!pages.length) throw new Error('This document has no pages to export.')
    const { all: styles, used } = buildStyleTable(engine.styles, ctx.usedStyleIds)
    const [{ pack: mediaPack, hasSvg }, fontPack] = await Promise.all([
      resolveMediaPack(ctx.mediaKeys, (id) => {
        const b = engine.doc.media(id)
        return b ? { bytes: b, type: engine.doc.mediaType(id) ?? '' } : null
      }),
      loadFontsForDocument(used, ctx.texts, ctx.hasSvg),
    ])
    const finalFontPack = hasSvg && !ctx.hasSvg ? await loadFontsForDocument(used, ctx.texts, true) : fontPack
    const docJson = JSON.stringify({
      pages,
      styles,
      metadata: { title: title?.replace(/\.[^.]+$/, '') || undefined, creator: 'Writan' },
    })
    return await pdfEngineClient.renderDocument(docJson, mediaPack, finalFontPack, signal)
  } finally {
    engine.dispose()
  }
}

export async function exportSlidesToPdf(bytes: Uint8Array, title?: string, signal?: AbortSignal): Promise<Uint8Array> {
  const { SlidesEngine } = await import('../../engine/slides/engine')
  const engine = await SlidesEngine.open(bytes.length ? bytes : null)
  try {
    const visible = engine.slides.map((s, i) => ({ s, i })).filter(({ s }) => !s.hidden)
    if (!visible.length) throw new Error('This presentation has no visible slides.')
    const ctx: CollectContext = { usedStyleIds: new Set(), texts: [], mediaKeys: new Set(), hasSvg: false }
    const wPt = engine.width * PX_TO_PT
    const hPt = engine.height * PX_TO_PT
    const pages: PdfPageInput[] = []
    for (const { i } of visible) {
      signal?.throwIfAborted?.()
      const items = engine.paintItems(i)
      walkItems(items, ctx)
      pages.push({
        widthPt: wPt,
        heightPt: hPt,
        scaleX: PX_TO_PT,
        scaleY: PX_TO_PT,
        offsetXPt: 0,
        offsetYPt: 0,
        items,
        svgs: [],
      })
    }
    const { all: styles, used } = buildStyleTable(engine.styles, ctx.usedStyleIds)
    const [{ pack: mediaPack, hasSvg }, fontPack] = await Promise.all([
      resolveMediaPack(ctx.mediaKeys, (id) => {
        const b = engine.doc.media(id)
        return b ? { bytes: b, type: engine.doc.mediaType(id) ?? '' } : null
      }),
      loadFontsForDocument(used, ctx.texts, ctx.hasSvg),
    ])
    const finalFontPack = hasSvg && !ctx.hasSvg ? await loadFontsForDocument(used, ctx.texts, true) : fontPack
    const docJson = JSON.stringify({
      pages,
      styles,
      metadata: { title: title?.replace(/\.[^.]+$/, '') || undefined, creator: 'Writan' },
    })
    return await pdfEngineClient.renderDocument(docJson, mediaPack, finalFontPack, signal)
  } finally {
    engine.dispose()
  }
}

function buildRowSlices(yForRow: (r: number) => number, rows: number, totalH: number, maxSlicePx: number): Array<[number, number]> {
  const slices: Array<[number, number]> = []
  let yStart = 0
  let rowCursor = 0
  while (yStart < totalH - 0.5) {
    const gridEnd = yForRow(rows)
    let yEnd = yStart
    if (rowCursor < rows && yStart < gridEnd - 0.5) {
      while (rowCursor < rows) {
        const nextRowY = yForRow(rowCursor + 1)
        if (nextRowY - yStart <= maxSlicePx) {
          yEnd = nextRowY
          rowCursor++
        } else {
          break
        }
      }
    }
    if (yEnd <= yStart + 0.5) {
      yEnd = Math.min(totalH, yStart + maxSlicePx)
      while (rowCursor < rows && yForRow(rowCursor) < yEnd) rowCursor++
    }
    // Extend final slice cleanly if trailing area fits below maxSlicePx.
    if (yEnd >= gridEnd - 0.5 && totalH - yStart <= maxSlicePx) {
      yEnd = totalH
    }
    slices.push([yStart, yEnd])
    yStart = yEnd
  }
  if (!slices.length) slices.push([0, Math.max(1, totalH)])
  return slices
}

export async function exportSheetsToPdf(bytes: Uint8Array, name: string, signal?: AbortSignal): Promise<Uint8Array> {
  const { SheetsEngine } = await import('../../engine/sheets/engine')
  const engine = await SheetsEngine.open(bytes.length ? bytes : null, name)
  try {
    const visible = engine.sheets.map((s, i) => ({ s, i })).filter(({ s }) => !s.hidden)
    if (!visible.length) throw new Error('This workbook has no visible sheets.')

    const pageW = 792
    const pageH = 612
    const margin = 28
    const contentW = pageW - margin * 2
    const contentH = pageH - margin * 2

    const ctx: CollectContext = { usedStyleIds: new Set(), texts: [], mediaKeys: new Set(), hasSvg: false }
    const pages: PdfPageInput[] = []
    let chartTools: {
      createElement: typeof import('react').createElement
      renderToStaticMarkup: typeof import('react-dom/server').renderToStaticMarkup
      ChartSvg: typeof import('../../engine/sheets/Chart').ChartSvg
    } | null = null

    for (const { i: sheetIdx } of visible) {
      signal?.throwIfAborted?.()
      const g = engine.geometryOf(sheetIdx)
      const [contentRows, contentCols] = g.info.content
      const rows = Math.max(1, contentRows)
      const cols = Math.max(1, contentCols)
      const raw = JSON.parse(engine.doc.paint(sheetIdx, 0, rows, 0, cols)) as { items: PaintItem[]; styles: PaintStyle[] }
      engine.styles.add(raw.styles)
      const pictures: PictureView[] = engine.picturesOf(sheetIdx)
      const charts: ChartView[] = engine.chartsOf(sheetIdx)
      const items: PaintItem[] = [
        ...raw.items,
        ...pictures.map((p): PaintItem => ({ t: 'image', media: p.media, x: p.x, y: p.y, w: p.w, h: p.h, crop: p.crop })),
      ]
      walkItems(items, ctx)

      let usedW = Math.max(1, g.x(cols))
      let usedH = Math.max(1, g.y(rows))
      for (const b of [...pictures, ...charts]) {
        usedW = Math.max(usedW, b.x + b.w)
        usedH = Math.max(usedH, b.y + b.h)
      }

      const renderedCharts: Array<{ chart: ChartView; svg: string }> = []
      if (charts.length) {
        if (!chartTools) {
          const [react, reactDomServer, chartMod] = await Promise.all([
            import('react'),
            import('react-dom/server'),
            import('../../engine/sheets/Chart'),
          ])
          chartTools = {
            createElement: react.createElement,
            renderToStaticMarkup: reactDomServer.renderToStaticMarkup,
            ChartSvg: chartMod.ChartSvg,
          }
        }
        ctx.hasSvg = true
        for (const c of charts) {
          const markup = chartTools.renderToStaticMarkup(chartTools.createElement(chartTools.ChartSvg, { chart: c, width: c.w, height: c.h }))
          const start = markup.indexOf('<svg')
          const end = markup.lastIndexOf('</svg>')
          if (start >= 0 && end > start) {
            const svg = markup.slice(start, end + 6).replace('<svg', '<svg xmlns="http://www.w3.org/2000/svg"')
            renderedCharts.push({ chart: c, svg })
            if (c.title) ctx.texts.push(c.title)
          }
        }
      }

      const scale = Math.min(PX_TO_PT, contentW / usedW)
      const maxSlicePx = Math.max(1, contentH / scale)
      const slices = buildRowSlices((r) => g.y(r), rows, usedH, maxSlicePx)
      for (const [yStart, yEnd] of slices) {
        const sliceH = Math.max(1, yEnd - yStart)
        const svgs: SvgOverlayInput[] = []
        for (const rc of renderedCharts) {
          if (rc.chart.y + rc.chart.h > yStart && rc.chart.y < yEnd) {
            svgs.push({ xPx: rc.chart.x, yPx: rc.chart.y, wPx: rc.chart.w, hPx: rc.chart.h, svg: rc.svg })
            ctx.hasSvg = true
          }
        }
        pages.push({
          widthPt: pageW,
          heightPt: pageH,
          scaleX: scale,
          scaleY: scale,
          offsetXPt: margin,
          offsetYPt: margin - yStart * scale,
          clipRectPx: [0, yStart, usedW, sliceH],
          items,
          svgs,
        })
      }
    }

    const { all: styles, used } = buildStyleTable(engine.styles, ctx.usedStyleIds)
    const [{ pack: mediaPack, hasSvg }, fontPack] = await Promise.all([
      resolveMediaPack(ctx.mediaKeys, (id) => {
        const b = engine.doc.media(id)
        return b ? { bytes: b, type: engine.doc.mediaType(id) ?? '' } : null
      }),
      loadFontsForDocument(used, ctx.texts, ctx.hasSvg),
    ])
    const finalFontPack = hasSvg && !ctx.hasSvg ? await loadFontsForDocument(used, ctx.texts, true) : fontPack
    const docJson = JSON.stringify({
      pages,
      styles,
      metadata: { title: name.replace(/\.[^.]+$/, '') || undefined, creator: 'Writan' },
    })
    return await pdfEngineClient.renderDocument(docJson, mediaPack, finalFontPack, signal)
  } finally {
    engine.dispose()
  }
}

export interface ImagePdfPageSpec {
  bytes: Uint8Array
  nameOrMime: string
  ocrLayer?: OcrLayerInput
}

export async function exportImageToPdf(bytes: Uint8Array, nameOrMime: string, ocrLayer?: OcrLayerInput, signal?: AbortSignal): Promise<Uint8Array> {
  return exportImagesToPdf([{ bytes, nameOrMime, ocrLayer }], signal)
}

export async function exportImagesToPdf(images: ImagePdfPageSpec[], signal?: AbortSignal): Promise<Uint8Array> {
  if (!images.length) throw new Error('At least one image is required for PDF export.')
  const pagesJson: Array<{
    mediaKey: string
    mime: string
    widthPt: number
    heightPt: number
    ocrLayer?: OcrLayerInput
  }> = []
  const mediaEntries: Array<[string, Uint8Array]> = []
  const ocrLayers: OcrLayerInput[] = []

  for (let i = 0; i < images.length; i++) {
    signal?.throwIfAborted?.()
    const item = images[i]
    const mediaKey = `img-${i}`
    let outBytes = item.bytes
    let outMime = guessImageMime(item.bytes, item.nameOrMime)
    let width: number
    let height: number

    if (isJpeg(item.bytes) || isPng(item.bytes)) {
      const dims = await resolveImageDimensions(item.bytes, outMime)
      width = dims.width
      height = dims.height
    } else {
      const converted = await convertRasterToPng(item.bytes, item.nameOrMime)
      outBytes = converted.bytes
      outMime = 'image/png'
      width = converted.width
      height = converted.height
    }

    const widthPt = item.ocrLayer?.pageWidthPt ?? Math.max(72, width * PX_TO_PT)
    const heightPt = item.ocrLayer?.pageHeightPt ?? Math.max(72, height * PX_TO_PT)
    const layer = item.ocrLayer ? { ...item.ocrLayer, pageIndex: i, pageWidthPt: widthPt, pageHeightPt: heightPt } : undefined
    if (layer) ocrLayers.push(layer)

    mediaEntries.push([mediaKey, outBytes])
    pagesJson.push({ mediaKey, mime: outMime, widthPt, heightPt, ocrLayer: layer })
  }

  const fontPack = ocrLayers.length ? await loadFontsForOcrLayers(ocrLayers) : packBlobs([])
  return await pdfEngineClient.imagesToPdf(JSON.stringify(pagesJson), packBlobs(mediaEntries), fontPack, signal)
}

export function utf8ToBytes(s: string): Uint8Array {
  return utf8Encoder.encode(s)
}
