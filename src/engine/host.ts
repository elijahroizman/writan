// The engine measures text through the app: a canvas with the fonts the page draws in, so the
// lines it breaks are the lines that show. Fonts load on first use; when one arrives the engine
// is told to measure again (onFontsChanged).

const canvas: OffscreenCanvas | HTMLCanvasElement = typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(1, 1) : document.createElement('canvas')
const ctx = canvas.getContext('2d') as OffscreenCanvasRenderingContext2D | CanvasRenderingContext2D
let current = ''
const metricsCache = new Map<string, [number, number, number]>()
// Widths are deterministic for a loaded font, while documents repeat most of their words. Keep
// them across documents too: the Rust engine's cache is per document, so opening a second file
// otherwise pays the same canvas measurements again.
const widthCache = new Map<string, number>()
const MAX_WIDTH_CACHE = 100_000
let probe: HTMLSpanElement | null = null

/** A font's normal line height, from a line box: the ascent and descent plus the line gap. */
function normalLineHeight(font: string) {
  if (!probe) {
    probe = document.createElement('span')
    probe.style.cssText = 'position:absolute;visibility:hidden;left:-9999px;top:0;line-height:normal;white-space:pre'
    probe.textContent = 'Hg'
    document.body.appendChild(probe)
  }
  probe.style.font = font
  probe.style.lineHeight = 'normal'
  return probe.getBoundingClientRect().height
}
const requested = new Set<string>()
const listeners = new Set<() => void>()
let notifyTimer = 0

/** Invalid document font names must not throw through a borrowed WASM document. */
function usableFont(font: string) {
  if (CSS.supports('font', font)) return font
  const size = parseFloat(font.replace(/^(italic |bold )*/, ''))
  return `${Number.isFinite(size) && size > 0 ? size : 16}px sans-serif`
}

function requestFont(font: string): Promise<FontFace[]> {
  try {
    return document.fonts.check(font) ? Promise.resolve([]) : document.fonts.load(font).catch(() => [])
  } catch {
    return Promise.resolve([])
  }
}

function setFont(font: string) {
  if (font === current) return
  const usable = usableFont(font)
  ctx.font = usable
  current = font
  if (!requested.has(usable)) {
    requested.add(usable)
    void requestFont(usable).then((faces) => {
      if (faces.length) fontsChanged()
    })
  }
}

export const host = {
  width(font: string, text: string): number {
    // Text advances scale linearly with size, so cache a word once per face rather than once
    // per face and size. Body, headings, footnotes and captions then share the same entries.
    const scaled = /^(?:(italic) )?(?:(bold) )?(\d+(?:\.\d+)?)px (.+)$/.exec(font)
    const key = scaled ? `${scaled[1] ?? ''} ${scaled[2] ?? ''} ${scaled[4]}\0${text}` : `${font}\0${text}`
    const hit = widthCache.get(key)
    if (hit !== undefined) return hit
    const size = scaled ? Number(scaled[3]) : 0
    const measureFont = scaled && size !== 100 ? font.replace(`${size}px `, '100px ') : font
    setFont(measureFont)
    let width = ctx.measureText(text).width
    if (scaled && size !== 100) width *= size / 100
    if (widthCache.size >= MAX_WIDTH_CACHE) widthCache.clear()
    widthCache.set(key, width)
    return width
  },
  metrics(font: string): number[] {
    const hit = metricsCache.get(font)
    if (hit) return hit
    setFont(font)
    const m = ctx.measureText('Hg')
    const px = parseFloat(font.replace(/^(italic |bold )*/, '')) || 16
    const ascent = m.fontBoundingBoxAscent || px * 0.8
    const descent = m.fontBoundingBoxDescent || px * 0.2
    const gap = Math.max(0, normalLineHeight(ctx.font) - ascent - descent)
    const v: [number, number, number] = [ascent, descent, gap < 0.05 ? 0 : gap]
    metricsCache.set(font, v)
    return v
  },
}

/** A font arrived or went (one finished loading, or a downloaded font was added or removed): measure again. */
export function fontsChanged() {
  metricsCache.clear()
  widthCache.clear()
  window.clearTimeout(notifyTimer)
  notifyTimer = window.setTimeout(() => listeners.forEach((f) => f()), 30)
}

/** Called when a font the engine measured with finishes loading: lay out again. */
export function onFontsChanged(f: () => void) {
  listeners.add(f)
  return () => listeners.delete(f)
}

/** Loads the regular and bold faces of `families` (CSS family lists) before the first layout. */
export async function preloadFonts(families: string[]) {
  const loads: Promise<unknown>[] = []
  for (const family of new Set(families)) {
    // Regular and bold are worth waiting for: most body text uses one of them. Italic faces stay
    // lazy, because preloading them here would swallow the font-change invalidation that actual
    // italic use relies on when the face arrives.
    for (const prefix of ['', 'bold ']) {
      const font = usableFont(`${prefix}16px ${family}`)
      requested.add(font)
      loads.push(requestFont(font))
    }
  }
  await Promise.all(loads)
  metricsCache.clear()
}
