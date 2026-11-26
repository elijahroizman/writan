import { LATEX_MATH_PREFIX } from '../../math/latexTag'
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
    baselineX0Pt?: number
    baselineY0Pt?: number
    baselineX1Pt?: number
    baselineY1Pt?: number
    orientedHPt?: number
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

type PdfPaintItem =
  | Exclude<PaintItem, { t: 'text' | 'group' }>
  | (Extract<PaintItem, { t: 'text' }> & { invisible?: boolean })
  | (Omit<Extract<PaintItem, { t: 'group' }>, 'items'> & { items: PdfPaintItem[] })

interface PdfPageInput {
  widthPt: number
  heightPt: number
  scaleX: number
  scaleY: number
  offsetXPt: number
  offsetYPt: number
  clipRectPx?: [number, number, number, number]
  items: PdfPaintItem[]
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

async function encodeRgba(rgba: { data: Uint8ClampedArray; width: number; height: number }, type: 'image/png' | 'image/jpeg'): Promise<Uint8Array> {
  const quality = type === 'image/jpeg' ? 0.92 : undefined
  const pixels =
    rgba.data instanceof Uint8ClampedArray && rgba.data.buffer instanceof ArrayBuffer
      ? (rgba.data as Uint8ClampedArray<ArrayBuffer>)
      : new Uint8ClampedArray(rgba.data)
  if (typeof OffscreenCanvas !== 'undefined') {
    const canvas = new OffscreenCanvas(rgba.width, rgba.height)
    try {
      const ctx = canvas.getContext('2d')!
      ctx.putImageData(new ImageData(pixels, rgba.width, rgba.height), 0, 0)
      const blob = await canvas.convertToBlob({ type, quality })
      return new Uint8Array(await blob.arrayBuffer())
    } finally {
      canvas.width = canvas.height = 0
    }
  }
  const canvas = document.createElement('canvas')
  try {
    canvas.width = rgba.width
    canvas.height = rgba.height
    const ctx = canvas.getContext('2d')!
    ctx.putImageData(new ImageData(pixels, rgba.width, rgba.height), 0, 0)
    const blob = await new Promise<Blob>((resolve, reject) =>
      canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('Failed to encode image.'))), type, quality),
    )
    return new Uint8Array(await blob.arrayBuffer())
  } finally {
    canvas.width = canvas.height = 0
  }
}

/**
 * A picture PDF can't hold as it is (WebP, GIF, HEIC, TIFF) as PNG or JPEG. Opaque photos (HEIC, WebP)
 * become JPEG, as the canvas export made them: PNG would make a phone photo's PDF many times bigger.
 */
async function convertRasterForPdf(bytes: Uint8Array, hint: string): Promise<{ bytes: Uint8Array; mime: 'image/png' | 'image/jpeg'; width: number; height: number }> {
  const rgba = await decodeImageToRgba(bytes, hint)
  const photo = ['image/heic', 'image/webp'].includes(guessImageMime(bytes, hint))
  let opaque = true
  for (let i = 3; opaque && i < rgba.data.length; i += 4) opaque = rgba.data[i] === 255
  const mime = photo && opaque ? 'image/jpeg' : 'image/png'
  return { bytes: await encodeRgba(rgba, mime), mime, width: rgba.width, height: rgba.height }
}

function pngDimensions(bytes: Uint8Array): { width: number; height: number } | null {
  if (!isPng(bytes) || bytes.length < 24) return null
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  return { width: view.getUint32(16, false), height: view.getUint32(20, false) }
}

async function rasterizeSvgToPng(bytes: Uint8Array, scale = 4): Promise<Uint8Array> {
  const url = URL.createObjectURL(new Blob([bytes as Uint8Array<ArrayBuffer>], { type: 'image/svg+xml' }))
  const canvas = document.createElement('canvas')
  try {
    const img = new Image()
    img.src = url
    await img.decode()
    const w = Math.max(1, Math.round((img.naturalWidth || 128) * scale))
    const h = Math.max(1, Math.round((img.naturalHeight || 128) * scale))
    canvas.width = w
    canvas.height = h
    const ctx = canvas.getContext('2d')!
    ctx.drawImage(img, 0, 0, w, h)
    const pngBlob = await new Promise<Blob>((resolve, reject) =>
      canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('Failed to encode SVG PNG fallback.'))), 'image/png'),
    )
    return new Uint8Array(await pngBlob.arrayBuffer())
  } finally {
    canvas.width = canvas.height = 0
    URL.revokeObjectURL(url)
  }
}

/** The elements crates/office-pdf/src/svg_path.rs draws; anything else goes through the browser. */
const VECTOR_SVG_ELEMENTS = new Set(['svg', 'g', 'symbol', 'a', 'use', 'defs', 'title', 'desc', 'metadata', 'path', 'rect', 'line', 'circle', 'ellipse', 'text'])

function svgNeedsRasterFallback(bytes: Uint8Array): boolean {
  const text = utf8Decoder.decode(bytes)
  if (/<!DOCTYPE|url\(\s*['"]?\s*#|\sclass\s*=/i.test(text) || RASTER_TEXT_RE.test(svgText(text))) return true
  for (const [, name] of text.matchAll(/<(?:[^:<>\s/!?]+:)?([A-Za-z][\w.-]*)/g)) {
    if (!VECTOR_SVG_ELEMENTS.has(name)) return true
  }
  return false
}

/** The text an SVG shows, roughly: enough to choose fonts and spot characters they lack. */
function svgText(svg: string): string {
  return svg.replace(/<[^>]*>/g, ' ')
}

function jpegExifOrientation(bytes: Uint8Array): number {
  if (!isJpeg(bytes)) return 1
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
    if (
      marker === 0xe1 &&
      len >= 16 &&
      bytes[i + 2] === 0x45 &&
      bytes[i + 3] === 0x78 &&
      bytes[i + 4] === 0x69 &&
      bytes[i + 5] === 0x66 &&
      bytes[i + 6] === 0x00 &&
      bytes[i + 7] === 0x00
    ) {
      const tiffStart = i + 8
      const view = new DataView(bytes.buffer, bytes.byteOffset + tiffStart, i + len - tiffStart)
      if (view.byteLength >= 8) {
        const tag = view.getUint16(0, false)
        const le = tag === 0x4949 ? true : tag === 0x4d4d ? false : null
        if (le !== null && view.getUint16(2, le) === 42) {
          const ifd = view.getUint32(4, le)
          if (ifd + 2 <= view.byteLength) {
            const count = Math.min(128, view.getUint16(ifd, le))
            for (let k = 0; k < count; k++) {
              const entry = ifd + 2 + k * 12
              if (entry + 12 > view.byteLength) break
              if (view.getUint16(entry, le) === 0x0112 && view.getUint16(entry + 2, le) === 3 && view.getUint32(entry + 4, le) === 1) {
                const val = view.getUint16(entry + 8, le)
                if (val >= 1 && val <= 8) return val
              }
            }
          }
        }
      }
    }
    i += len
  }
  return 1
}

function jpegDimensions(bytes: Uint8Array): { width: number; height: number } | null {
  if (!isJpeg(bytes)) return null
  const orientation = jpegExifOrientation(bytes)
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
        const height = (bytes[i + 3] << 8) | bytes[i + 4]
        const width = (bytes[i + 5] << 8) | bytes[i + 6]
        return orientation >= 5 && orientation <= 8 ? { width: height, height: width } : { width, height }
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
  extraEntries: ReadonlyMap<string, Uint8Array> = new Map(),
): Promise<{ pack: Uint8Array; hasSvg: boolean }> {
  const entries: Array<[string, Uint8Array]> = []
  let hasSvg = false
  for (const id of keys) {
    const extra = extraEntries.get(id)
    if (extra) {
      entries.push([id, extra])
      continue
    }
    if (id.startsWith(LATEX_MATH_PREFIX)) {
      const { renderLatexTagToSvgBytes } = await import('../../math/latexMath')
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
      if (svgNeedsRasterFallback(raw.bytes)) {
        try {
          entries.push([id, await rasterizeSvgToPng(raw.bytes)])
          continue
        } catch {
          // Fall back to direct SVG path parser if browser rasterization fails.
        }
      }
      hasSvg = true
      entries.push([id, raw.bytes])
    } else if (isJpeg(raw.bytes) || isPng(raw.bytes)) {
      entries.push([id, raw.bytes])
    } else {
      try {
        const converted = await convertRasterForPdf(raw.bytes, raw.type)
        entries.push([id, converted.bytes])
      } catch {
        // Skip unreadable image rather than crashing export.
      }
    }
  }
  return { pack: packBlobs(entries), hasSvg }
}

/**
 * Characters outside what the bundled PDF fonts (Liberation, Carlito, Caladea, Noto Sans, Noto Sans
 * Arabic, Noto Sans Math) cover: CJK, Indic, Thai, emoji and the rest. Text with any of them is drawn
 * by the browser with the system's fonts, as the canvas export did, over invisible searchable text.
 */
const RASTER_TEXT_RE =
  /[^\u0000-\u052F\u0591-\u06FF\u0750-\u077F\u0870-\u08FF\u1AB0-\u1ACE\u1C80-\u1C88\u1D00-\u23E1\u2460-\u2475\u24EA-\u266F\u2776-\u277F\u27C0-\u27FF\u2900-\u2B4C\u2C60-\u2C7F\u2DE0-\u2E5D\uA640-\uA69F\uA700-\uA7FF\uFB00-\uFE2F\uFE70-\uFEFF\uFFFC\uFFFD\u{1D400}-\u{1D7FF}\u{1EE00}-\u{1EEF1}]/u

let textProbeCtx: CanvasRenderingContext2D | null = null

type TextOverlayMeta = { key: string; boxW: number; offsetX: number; offsetY: number; width: number; height: number }

async function renderTextToPng(
  text: string,
  font: string,
  color: string,
  widthPx: number,
  ascent: number,
  descent: number,
  rtl: boolean,
): Promise<{ bytes: Uint8Array; boxW: number; offsetX: number; offsetY: number; width: number; height: number }> {
  const scale = 4
  const probe = (textProbeCtx ??= document.createElement('canvas').getContext('2d')!)
  probe.font = font
  probe.textBaseline = 'alphabetic'
  const metrics = probe.measureText(text)
  const measuredW = metrics.width
  const boxW = Math.max(1, widthPx > 0 ? widthPx : measuredW)
  let sx = 1
  let drawX = 0
  if (widthPx > 0 && measuredW > 0) {
    if (measuredW > widthPx * 1.005) sx = Math.max(0.5, widthPx / measuredW)
    else if (rtl && widthPx - measuredW > 0.5) drawX = widthPx - measuredW
  }
  const padLeft = Math.max(0, Math.ceil((metrics.actualBoundingBoxLeft ?? 0) * sx + 1))
  const padRight = Math.max(0, Math.ceil(Math.max(drawX + (metrics.actualBoundingBoxRight ?? measuredW) * sx, boxW) - boxW + 1))
  const topPx = Math.max(ascent, Math.ceil((metrics.actualBoundingBoxAscent ?? ascent) + 1))
  const botPx = Math.max(descent, Math.ceil((metrics.actualBoundingBoxDescent ?? descent) + 1))
  const width = boxW + padLeft + padRight
  const height = topPx + botPx
  const canvas = document.createElement('canvas')
  try {
    canvas.width = Math.max(1, Math.ceil(width * scale))
    canvas.height = Math.max(1, Math.ceil(height * scale))
    const ctx = canvas.getContext('2d')!
    ctx.scale(scale, scale)
    ctx.translate(padLeft + drawX, topPx)
    if (sx !== 1) ctx.scale(sx, 1)
    ctx.font = font
    ctx.fillStyle = color || '#000000'
    ctx.textBaseline = 'alphabetic'
    ctx.fillText(text, 0, 0)
    const blob = await new Promise<Blob>((resolve, reject) =>
      canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('Failed to encode text fallback PNG.'))), 'image/png'),
    )
    return { bytes: new Uint8Array(await blob.arrayBuffer()), boxW, offsetX: -padLeft, offsetY: -topPx, width, height }
  } finally {
    canvas.width = canvas.height = 0
  }
}

async function prepareTextOverlays(
  items: PaintItem[],
  styleSheet: StyleSheet,
  ctx: CollectContext,
  extraMedia: Map<string, Uint8Array>,
  overlayCache = new Map<string, TextOverlayMeta>(),
): Promise<PdfPaintItem[]> {
  const out: PdfPaintItem[] = []
  for (const item of items) {
    if (item.t === 'group') {
      out.push({ ...item, items: await prepareTextOverlays(item.items, styleSheet, ctx, extraMedia, overlayCache) })
      continue
    }
    if (item.t !== 'text' || !RASTER_TEXT_RE.test(item.text)) {
      out.push(item)
      continue
    }
    const resolved = styleSheet.style(item.s)
    const color = resolved?.style.color ?? '#000000'
    if (color === 'transparent') {
      out.push(item)
      continue
    }
    const font = resolved?.style.font ?? '16px sans-serif'
    const fontSize = parseFloat(/(\d+(?:\.\d+)?)px/.exec(font)?.[1] ?? '16') || 16
    const ascent = resolved?.ascent ?? fontSize * 0.85
    const descent = resolved?.descent ?? fontSize * 0.25
    const baseline = item.y - (resolved?.style.shift ?? 0)
    const cacheKey = `${item.text}\0${font}\0${color}\0${item.w}\0${item.rtl ? 1 : 0}`
    let meta = overlayCache.get(cacheKey)
    if (!meta) {
      const overlay = await renderTextToPng(item.text, font, color, item.w, ascent, descent, !!item.rtl)
      const key = `__wpdf_text_${extraMedia.size}`
      extraMedia.set(key, overlay.bytes)
      meta = { key, boxW: overlay.boxW, offsetX: overlay.offsetX, offsetY: overlay.offsetY, width: overlay.width, height: overlay.height }
      overlayCache.set(cacheKey, meta)
    }
    ctx.mediaKeys.add(meta.key)
    const yTop = baseline - ascent
    if (resolved?.style.background) {
      out.push({ t: 'rect', x: item.x, y: yTop, w: meta.boxW, h: ascent + descent, fill: resolved.style.background })
    }
    out.push({
      t: 'image',
      media: meta.key,
      x: item.x + meta.offsetX,
      y: baseline + meta.offsetY,
      w: meta.width,
      h: meta.height,
    })
    out.push({ ...item, invisible: true })
  }
  return out
}

function itemYBounds(item: Exclude<PdfPaintItem, { t: 'text' | 'group' }>): [number, number] {
  switch (item.t) {
    case 'rect':
    case 'image':
      return [item.y, item.y + item.h]
    case 'line':
      return [Math.min(item.y1, item.y2) - item.w, Math.max(item.y1, item.y2) + item.w]
    case 'path': {
      const nums = item.d.match(/-?\d+(?:\.\d+)?/g)
      if (!nums?.length) return [item.y - 64, item.y + 64]
      const hasRel = /[mlhvcsqt]/.test(item.d)
      const radius = hasRel
        ? nums.reduce((acc, n) => acc + Math.abs(Number(n)), 24)
        : Math.max(24, ...nums.map((n) => Math.abs(Number(n))))
      return [item.y - radius, item.y + radius]
    }
  }
}

/**
 * Bucket sheet paint items across vertical page slices in a single pass with binary search,
 * preserving the exact inclusion semantics of `filterItemsForSlice`.
 */
function bucketItemsBySlices(items: readonly PdfPaintItem[], slices: ReadonlyArray<[number, number]>): PdfPaintItem[][] {
  const n = slices.length
  if (n === 1) return [items.slice()]
  const buckets: PdfPaintItem[][] = Array.from({ length: n }, () => [])

  const firstSliceWhereEndGt = (y: number, inclusive = false) => {
    let lo = 0
    let hi = n
    while (lo < hi) {
      const mid = (lo + hi) >> 1
      const end = slices[mid][1]
      if (inclusive ? end < y : end <= y) lo = mid + 1
      else hi = mid
    }
    return lo
  }

  for (const item of items) {
    if (item.t === 'text') {
      if (item.y < slices[0][1]) buckets[0].push(item)
      else if (item.y >= slices[n - 1][0]) buckets[n - 1].push(item)
      else {
        const k = firstSliceWhereEndGt(item.y, false)
        if (k < n && item.y >= slices[k][0]) buckets[k].push(item)
      }
      continue
    }
    if (item.t === 'group') {
      const pad = item.rot ? Math.hypot(item.w, item.h) * 0.5 : 0
      const loY = item.y - pad
      const hiY = item.y + item.h + pad
      let k = firstSliceWhereEndGt(loY, false)
      if (loY < slices[0][1]) k = 0
      for (; k < n && (k === n - 1 || loY < slices[k][1]); k++) {
        if (k === 0 || hiY > slices[k][0]) buckets[k].push(item)
      }
      continue
    }
    const [minY, maxY] = itemYBounds(item)
    const y0 = minY - 24
    const y1 = maxY + 24
    for (let k = firstSliceWhereEndGt(y0, true); k < n && slices[k][0] <= y1; k++) {
      buckets[k].push(item)
    }
  }
  return buckets
}

export async function exportWordToPdf(bytes: Uint8Array, title?: string, signal?: AbortSignal): Promise<Uint8Array> {
  const { WordEngine } = await import('../../engine/word/engine')
  const engine = await WordEngine.open(bytes.length ? bytes : null)
  try {
    const sizes = engine.sizes
    if (!sizes.length) throw new Error('This document has no pages to export.')
    const ctx: CollectContext = { usedStyleIds: new Set(), texts: [], mediaKeys: new Set(), hasSvg: false }
    const extraMedia = new Map<string, Uint8Array>()
    const overlayCache = new Map<string, TextOverlayMeta>()
    const pages: PdfPageInput[] = []
    for (let i = 0; i < sizes.length; i++) {
      signal?.throwIfAborted?.()
      const p = engine.paint(i, true)
      if (!p) continue
      walkItems(p.items, ctx)
      const items = await prepareTextOverlays(p.items, engine.styles, ctx, extraMedia, overlayCache)
      pages.push({
        widthPt: p.width * PX_TO_PT,
        heightPt: p.height * PX_TO_PT,
        scaleX: PX_TO_PT,
        scaleY: PX_TO_PT,
        offsetXPt: 0,
        offsetYPt: 0,
        items,
        svgs: [],
      })
    }
    if (!pages.length) throw new Error('This document has no pages to export.')
    const { all: styles, used } = buildStyleTable(engine.styles, ctx.usedStyleIds)
    const [{ pack: mediaPack, hasSvg }, fontPack] = await Promise.all([
      resolveMediaPack(
        ctx.mediaKeys,
        (id) => {
          const b = engine.doc.media(id)
          return b ? { bytes: b, type: engine.doc.mediaType(id) ?? '' } : null
        },
        extraMedia,
      ),
      loadFontsForDocument(used, ctx.texts, ctx.hasSvg),
    ])
    const finalFontPack = hasSvg && !ctx.hasSvg ? await loadFontsForDocument(used, ctx.texts, true) : fontPack
    const docJson = JSON.stringify({
      pages,
      styles,
      metadata: { title: title || undefined, creator: 'Writan' },
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
    const extraMedia = new Map<string, Uint8Array>()
    const overlayCache = new Map<string, TextOverlayMeta>()
    const wPt = engine.width * PX_TO_PT
    const hPt = engine.height * PX_TO_PT
    const pages: PdfPageInput[] = []
    for (const { i } of visible) {
      signal?.throwIfAborted?.()
      const rawItems = engine.paintItems(i)
      walkItems(rawItems, ctx)
      const items = await prepareTextOverlays(rawItems, engine.styles, ctx, extraMedia, overlayCache)
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
      resolveMediaPack(
        ctx.mediaKeys,
        (id) => {
          const b = engine.doc.media(id)
          return b ? { bytes: b, type: engine.doc.mediaType(id) ?? '' } : null
        },
        extraMedia,
      ),
      loadFontsForDocument(used, ctx.texts, ctx.hasSvg),
    ])
    const finalFontPack = hasSvg && !ctx.hasSvg ? await loadFontsForDocument(used, ctx.texts, true) : fontPack
    const docJson = JSON.stringify({
      pages,
      styles,
      metadata: { title: title || undefined, creator: 'Writan' },
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
    const extraMedia = new Map<string, Uint8Array>()
    const overlayCache = new Map<string, TextOverlayMeta>()
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
      const rawPaintItems: PaintItem[] = [
        ...raw.items,
        ...pictures.map((p): PaintItem => ({ t: 'image', media: p.media, x: p.x, y: p.y, w: p.w, h: p.h, crop: p.crop })),
      ]
      walkItems(rawPaintItems, ctx)
      const preparedItems = await prepareTextOverlays(rawPaintItems, engine.styles, ctx, extraMedia, overlayCache)

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
            const labels = svgText(svg)
            if (RASTER_TEXT_RE.test(labels)) {
              // Labels the bundled fonts can't draw: let the browser draw the chart as a picture.
              try {
                const key = `__wpdf_chart_${extraMedia.size}`
                extraMedia.set(key, await rasterizeSvgToPng(utf8Encoder.encode(svg)))
                ctx.mediaKeys.add(key)
                preparedItems.push({ t: 'image', media: key, x: c.x, y: c.y, w: c.w, h: c.h })
                continue
              } catch {
                // Draw it as vectors below.
              }
            }
            renderedCharts.push({ chart: c, svg })
            ctx.texts.push(labels)
          }
        }
      }

      const scale = Math.min(PX_TO_PT, contentW / usedW)
      const maxSlicePx = Math.max(1, contentH / scale)
      const slices = buildRowSlices((r) => g.y(r), rows, usedH, maxSlicePx)
      const bucketed = bucketItemsBySlices(preparedItems, slices)
      for (const [index, [yStart, yEnd]] of slices.entries()) {
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
          items: bucketed[index],
          svgs,
        })
      }
    }

    const { all: styles, used } = buildStyleTable(engine.styles, ctx.usedStyleIds)
    const [{ pack: mediaPack, hasSvg }, fontPack] = await Promise.all([
      resolveMediaPack(
        ctx.mediaKeys,
        (id) => {
          const b = engine.doc.media(id)
          return b ? { bytes: b, type: engine.doc.mediaType(id) ?? '' } : null
        },
        extraMedia,
      ),
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
      const converted = await convertRasterForPdf(item.bytes, item.nameOrMime)
      outBytes = converted.bytes
      outMime = converted.mime
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
