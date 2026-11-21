// Draws a slide or page from the engine's display list (src/engine/paint.ts) onto a canvas, so
// an agent can see it. The editor shows the same items as DOM; a canvas turns into a picture
// for MCP without the DOM, and draws with the fonts the page has loaded, like the editor.

import type { MediaCache, PaintItem, StyleSheet } from '../../engine/paint'

/** A box to mark on the picture with a label, in the drawing's pixels. */
export interface Mark {
  label: string
  x: number
  y: number
  w: number
  h: number
  rot?: number
}

export interface Drawing {
  width: number
  height: number
  items: PaintItem[]
  styles: StyleSheet
  media: MediaCache
}

/** Loads the pictures the items show. */
async function pictures(items: PaintItem[], media: MediaCache): Promise<Map<string, HTMLImageElement>> {
  const ids = new Set<string>()
  const walk = (list: PaintItem[]) => {
    for (const it of list) {
      if (it.t === 'image') ids.add(it.media)
      else if (it.t === 'group') walk(it.items)
    }
  }
  walk(items)
  const out = new Map<string, HTMLImageElement>()
  await Promise.all(
    [...ids].map(async (id) => {
      const url = media.url(id)
      if (!url) return
      const img = new Image()
      img.src = url
      try {
        await img.decode()
        out.set(id, img)
      } catch {
        // A picture the browser can't show (EMF, TIFF) is left out, as in the editor.
      }
    }),
  )
  return out
}

const DASHES: Record<string, number[]> = { dashed: [6, 4], dash: [6, 4], dotted: [1.5, 3], dot: [1.5, 3], sysDot: [1.5, 3] }

/**
 * Draws `d` scaled to `scale`, the part from `view` (x, y, w, h in the drawing's pixels) when
 * given, with `marks` outlined and labelled on top.
 */
export async function draw(d: Drawing, scale: number, marks: Mark[] = [], view?: { x: number; y: number; w: number; h: number }): Promise<HTMLCanvasElement> {
  const images = await pictures(d.items, d.media)
  const area = view ?? { x: 0, y: 0, w: d.width, h: d.height }
  const canvas = document.createElement('canvas')
  canvas.width = Math.max(1, Math.round(area.w * scale))
  canvas.height = Math.max(1, Math.round(area.h * scale))
  const ctx = canvas.getContext('2d')!
  ctx.fillStyle = '#ffffff'
  ctx.fillRect(0, 0, canvas.width, canvas.height)
  ctx.scale(scale, scale)
  ctx.translate(-area.x, -area.y)
  ctx.textBaseline = 'alphabetic'
  ctx.textAlign = 'left'
  paint(ctx, d.items, d.styles, images)
  for (const m of marks) mark(ctx, m, 1 / scale)
  return canvas
}

function paint(ctx: CanvasRenderingContext2D, items: PaintItem[], styles: StyleSheet, images: Map<string, HTMLImageElement>) {
  for (const it of items) {
    switch (it.t) {
      case 'text': {
        const s = styles.style(it.s)
        if (!s) break
        const { style, ascent, descent } = s
        const baseline = it.y - (style.shift ?? 0)
        ctx.font = style.font
        ctx.fontKerning = 'normal'
        ;(ctx as CanvasRenderingContext2D & { fontVariantCaps?: string }).fontVariantCaps = style.smallCaps ? 'small-caps' : 'normal'
        ;(ctx as CanvasRenderingContext2D & { wordSpacing?: string }).wordSpacing = it.ws ? `${it.ws}px` : '0px'
        ctx.direction = it.rtl ? 'rtl' : 'ltr'
        const width = ctx.measureText(it.text).width
        // With rtl, canvas measures from the right; the editor places the run's left edge at x.
        const left = it.x
        const x = it.rtl ? it.x + width : it.x
        ctx.textAlign = it.rtl ? 'right' : 'left'
        if (style.background) {
          ctx.fillStyle = style.background
          ctx.fillRect(left, baseline - ascent, width, ascent + descent)
        }
        ctx.fillStyle = style.color
        ctx.fillText(it.text, x, baseline)
        const deco = style.decoration ?? ''
        if (deco.includes('underline') || deco.includes('line-through')) {
          const thick = Math.max(1, (ascent + descent) / 18)
          ctx.fillStyle = style.color
          if (deco.includes('underline')) ctx.fillRect(left, baseline + Math.max(1, descent * 0.35), width, thick)
          if (deco.includes('line-through')) ctx.fillRect(left, baseline - ascent * 0.32, width, thick)
        }
        ctx.textAlign = 'left'
        ctx.direction = 'ltr'
        break
      }
      case 'rect':
        if (it.fill) {
          ctx.fillStyle = it.fill
          ctx.fillRect(it.x, it.y, it.w, it.h)
        }
        break
      case 'line': {
        ctx.save()
        ctx.strokeStyle = it.color
        ctx.lineWidth = it.w
        ctx.setLineDash(DASHES[it.dash ?? ''] ?? [])
        ctx.beginPath()
        ctx.moveTo(it.x1, it.y1)
        ctx.lineTo(it.x2, it.y2)
        ctx.stroke()
        ctx.restore()
        break
      }
      case 'image': {
        const img = images.get(it.media)
        if (!img) {
          ctx.fillStyle = '#e6e6e6'
          ctx.fillRect(it.x, it.y, it.w, it.h)
          break
        }
        const [l, t, r, b] = it.crop ?? [0, 0, 0, 0]
        const iw = it.w / Math.max(0.01, 1 - l - r)
        const ih = it.h / Math.max(0.01, 1 - t - b)
        ctx.save()
        ctx.beginPath()
        ctx.rect(it.x, it.y, it.w, it.h)
        ctx.clip()
        ctx.drawImage(img, it.x - l * iw, it.y - t * ih, iw, ih)
        ctx.restore()
        break
      }
      case 'path': {
        ctx.save()
        ctx.translate(it.x, it.y)
        let path: Path2D
        try {
          path = new Path2D(it.d)
        } catch {
          ctx.restore()
          break
        }
        if (it.fill && it.fill !== 'none') {
          ctx.fillStyle = it.fill
          ctx.fill(path)
        }
        if (it.stroke && it.stroke !== 'none') {
          ctx.strokeStyle = it.stroke
          ctx.lineWidth = it.sw ?? 1
          ctx.setLineDash(DASHES[it.dash ?? ''] ?? [])
          ctx.stroke(path)
        }
        ctx.restore()
        break
      }
      case 'group': {
        ctx.save()
        // Turned and flipped about its middle, as the editor's CSS does.
        ctx.translate(it.x + it.w / 2, it.y + it.h / 2)
        if (it.rot) ctx.rotate((it.rot * Math.PI) / 180)
        if (it.fh || it.fv) ctx.scale(it.fh ? -1 : 1, it.fv ? -1 : 1)
        ctx.translate(-it.w / 2, -it.h / 2)
        if (it.clip) {
          ctx.beginPath()
          ctx.rect(0, 0, it.w, it.h)
          ctx.clip()
        }
        paint(ctx, it.items, styles, images)
        ctx.restore()
        break
      }
    }
  }
}

/** A box outlined, with its label in a tag at its top left. `px` is one picture pixel. */
function mark(ctx: CanvasRenderingContext2D, m: Mark, px: number) {
  ctx.save()
  ctx.translate(m.x + m.w / 2, m.y + m.h / 2)
  if (m.rot) ctx.rotate((m.rot * Math.PI) / 180)
  ctx.translate(-m.w / 2, -m.h / 2)
  ctx.strokeStyle = '#E5007E'
  ctx.lineWidth = 2 * px
  ctx.setLineDash([6 * px, 4 * px])
  ctx.strokeRect(0, 0, m.w, m.h)
  ctx.restore()
  ctx.save()
  const size = 13 * px
  ctx.font = `bold ${size}px sans-serif`
  const w = ctx.measureText(m.label).width + 8 * px
  const h = size + 6 * px
  ctx.fillStyle = '#E5007E'
  ctx.fillRect(m.x, m.y, w, h)
  ctx.fillStyle = '#ffffff'
  ctx.textBaseline = 'middle'
  ctx.fillText(m.label, m.x + 4 * px, m.y + h / 2)
  ctx.restore()
}

/** The picture's natural size in pixels, from its bytes. */
export async function naturalSize(media: MediaCache, id: string): Promise<{ width: number; height: number } | null> {
  const url = media.url(id)
  if (!url) return null
  const img = new Image()
  img.src = url
  try {
    await img.decode()
    return { width: img.naturalWidth, height: img.naturalHeight }
  } catch {
    return null
  }
}
