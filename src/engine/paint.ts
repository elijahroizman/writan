// Paints the engine's display lists: pages, slides and grids arrive as items in pixels (text on
// a baseline in a numbered style, rectangles, lines, pictures, SVG paths, groups) and become
// absolutely placed DOM. Built as one HTML string per page, which is the quickest way to make
// a few thousand elements.

import { host } from './host'

export interface PaintStyle {
  font: string
  color: string
  decoration?: string
  background?: string
  smallCaps?: boolean
  /** Baseline shift in pixels, up positive. */
  shift?: number
}

export type PaintItem =
  | { t: 'text'; x: number; y: number; w: number; s: number; text: string; ws?: number; link?: string; rtl?: boolean }
  | { t: 'rect'; x: number; y: number; w: number; h: number; fill?: string }
  | { t: 'line'; x1: number; y1: number; x2: number; y2: number; color: string; w: number; dash?: string }
  | { t: 'image'; x: number; y: number; w: number; h: number; media: string; crop?: [number, number, number, number]; id?: string }
  | { t: 'path'; x: number; y: number; d: string; fill?: string; stroke?: string; sw?: number; dash?: string }
  | { t: 'group'; x: number; y: number; w: number; h: number; rot?: number; fh?: boolean; fv?: boolean; clip?: boolean; id?: string; items: PaintItem[] }

export interface Painted {
  width: number
  height: number
  items: PaintItem[]
  styles: PaintStyle[]
}

/**
 * A document's paint styles, numbered as the engine sent them, as CSS classes in one style sheet.
 * Each style's line box is exactly the font's ascent plus descent, so a span placed at
 * `baseline - ascent` draws its text on the engine's baseline.
 */
export class StyleSheet {
  private ascents: number[] = []
  private shifts: number[] = []
  private sent: PaintStyle[] = []
  private sheet: HTMLStyleElement
  readonly prefix: string

  constructor(prefix: string) {
    this.prefix = prefix
    this.sheet = document.createElement('style')
    document.head.appendChild(this.sheet)
  }

  get size() {
    return this.ascents.length
  }

  add(styles: PaintStyle[]) {
    if (!styles.length) return
    let css = ''
    for (const s of styles) {
      this.sent.push(s)
      const id = this.ascents.length
      const [ascent, descent] = host.metrics(s.font)
      this.ascents.push(ascent)
      this.shifts.push(s.shift ?? 0)
      css += `.${this.prefix}${id}{font:${s.font};color:${s.color};line-height:${(ascent + descent).toFixed(2)}px;height:${(ascent + descent).toFixed(2)}px`
      if (s.decoration) css += `;text-decoration:${s.decoration};text-decoration-skip-ink:auto`
      if (s.background) css += `;background:${s.background}`
      if (s.smallCaps) css += `;font-variant:small-caps`
      css += '}\n'
    }
    this.sheet.textContent += css
  }

  /** Starts over: the engine numbers its styles from 0 again. */
  reset() {
    this.ascents = []
    this.shifts = []
    this.sent = []
    this.sheet.textContent = ''
  }

  /** Keep style ids stable while replacing fallback font metrics with the loaded font's. */
  remeasure() {
    const styles = this.sent
    this.reset()
    this.add(styles)
  }

  top(style: number, baseline: number) {
    return baseline - (this.ascents[style] ?? 0) - (this.shifts[style] ?? 0)
  }

  dispose() {
    this.sheet.remove()
  }
}

const ESC: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }
const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ESC[c])
const n = (v: number) => (Math.round(v * 100) / 100).toString()

/** Pictures' URLs by media id. */
export type MediaUrl = (id: string) => string | undefined

export function itemsHtml(items: PaintItem[], styles: StyleSheet, media: MediaUrl): string {
  let out = ''
  for (const it of items) {
    switch (it.t) {
      case 'text': {
        const top = styles.top(it.s, it.y)
        let style = `left:${n(it.x)}px;top:${n(top)}px`
        if (it.ws) style += `;word-spacing:${n(it.ws)}px`
        // The engine orders mixed-direction text into pieces that each run one way.
        if (it.rtl) style += ';direction:rtl'
        const link = it.link ? ` data-link="${esc(it.link)}"` : ''
        out += `<span class="${styles.prefix}${it.s}${it.link ? ' ln' : ''}" style="${style}"${link}>${esc(it.text)}</span>`
        break
      }
      case 'rect':
        out += `<div class="rc" style="left:${n(it.x)}px;top:${n(it.y)}px;width:${n(it.w)}px;height:${n(it.h)}px${it.fill ? `;background:${esc(it.fill)}` : ''}"></div>`
        break
      case 'line': {
        const dash = it.dash && /^(solid|dashed|dotted|double|groove|ridge|inset|outset|none|hidden)$/.test(it.dash) ? it.dash : 'solid'
        if (Math.abs(it.y1 - it.y2) < 0.01) {
          const x = Math.min(it.x1, it.x2)
          out += `<div class="ln-h" style="left:${n(x)}px;top:${n(it.y1 - it.w / 2)}px;width:${n(Math.abs(it.x2 - it.x1))}px;border-top:${n(it.w)}px ${dash} ${esc(it.color)}"></div>`
        } else if (Math.abs(it.x1 - it.x2) < 0.01) {
          const y = Math.min(it.y1, it.y2)
          out += `<div class="ln-v" style="left:${n(it.x1 - it.w / 2)}px;top:${n(y)}px;height:${n(Math.abs(it.y2 - it.y1))}px;border-left:${n(it.w)}px ${dash} ${esc(it.color)}"></div>`
        } else {
          out += `<svg class="pa"><line x1="${n(it.x1)}" y1="${n(it.y1)}" x2="${n(it.x2)}" y2="${n(it.y2)}" stroke="${esc(it.color)}" stroke-width="${n(it.w)}"/></svg>`
        }
        break
      }
      case 'image': {
        const src = media(it.media)
        const id = it.id ? ` data-id="${esc(it.id)}"` : ''
        if (!src) {
          out += `<div class="im missing" style="left:${n(it.x)}px;top:${n(it.y)}px;width:${n(it.w)}px;height:${n(it.h)}px"${id}></div>`
          break
        }
        const [l, t, r, b] = it.crop ?? [0, 0, 0, 0]
        const iw = it.w / Math.max(0.01, 1 - l - r)
        const ih = it.h / Math.max(0.01, 1 - t - b)
        out += `<div class="im" style="left:${n(it.x)}px;top:${n(it.y)}px;width:${n(it.w)}px;height:${n(it.h)}px"${id}><img src="${esc(src)}" draggable="false" style="left:${n(-l * iw)}px;top:${n(-t * ih)}px;width:${n(iw)}px;height:${n(ih)}px" alt=""></div>`
        break
      }
      case 'path': {
        const dash = it.dash === 'dash' || it.dash === 'dashed' ? ' stroke-dasharray="6 4"' : it.dash === 'dot' || it.dash === 'sysDot' || it.dash === 'dotted' ? ' stroke-dasharray="1.5 3"' : ''
        out += `<svg class="pa" style="left:${n(it.x)}px;top:${n(it.y)}px"><path d="${esc(it.d)}" fill="${esc(it.fill ?? 'none')}" stroke="${esc(it.stroke ?? 'none')}"${it.sw ? ` stroke-width="${n(it.sw)}"` : ''}${dash}/></svg>`
        break
      }
      case 'group': {
        const tf: string[] = []
        if (it.rot) tf.push(`rotate(${n(it.rot)}deg)`)
        if (it.fh || it.fv) tf.push(`scale(${it.fh ? -1 : 1},${it.fv ? -1 : 1})`)
        const cls = `gr${it.clip ? ' clip' : ''}${it.id === 'hf-dim' ? ' dim' : ''}`
        const id = it.id ? ` data-id="${esc(it.id)}"` : ''
        out += `<div class="${cls}" style="left:${n(it.x)}px;top:${n(it.y)}px;width:${n(it.w)}px;height:${n(it.h)}px${tf.length ? `;transform:${tf.join(' ')}` : ''}"${id}>${itemsHtml(it.items, styles, media)}</div>`
        break
      }
    }
  }
  return out
}

/** Blob URLs for a document's pictures, made on first use and freed with the document. */
export class MediaCache {
  private urls = new Map<string, string>()
  constructor(private read: (id: string) => { bytes: Uint8Array; type: string } | null) {}

  url = (id: string): string | undefined => {
    let u = this.urls.get(id)
    if (u) return u
    const m = this.read(id)
    if (!m) return undefined
    u = URL.createObjectURL(new Blob([m.bytes as Uint8Array<ArrayBuffer>], { type: m.type || 'image/png' }))
    this.urls.set(id, u)
    return u
  }

  dispose() {
    for (const u of this.urls.values()) URL.revokeObjectURL(u)
    this.urls.clear()
  }
}
