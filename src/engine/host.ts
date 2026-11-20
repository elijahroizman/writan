// The engine measures text through the app: a canvas with the fonts the page draws in, so the
// lines it breaks are the lines that show. Fonts load on first use; when one arrives the engine
// is told to measure again (onFontsChanged).

const canvas: OffscreenCanvas | HTMLCanvasElement = typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(1, 1) : document.createElement('canvas')
const ctx = canvas.getContext('2d') as OffscreenCanvasRenderingContext2D | CanvasRenderingContext2D
let current = ''
const metricsCache = new Map<string, [number, number, number]>()
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
    setFont(font)
    return ctx.measureText(text).width
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
    for (const prefix of ['', 'bold ', 'italic ']) {
      const font = usableFont(`${prefix}16px ${family}`)
      requested.add(font)
      loads.push(requestFont(font))
    }
  }
  await Promise.all(loads)
  metricsCache.clear()
}
