// A presentation open in the engine: the WebAssembly document, its slides and their drawings
// (kept per slide until the slide changes), the selection, and what the inspector reads.

import { host, onFontsChanged } from '../host'
import { expectKind, loadEngine } from '../load'
import type { PaintItem } from '../paint'
import { itemsHtml, MediaCache, StyleSheet } from '../paint'
import { BASE_FONTS } from '../preload'
import { preloadFonts } from '../host'
import type { SlidesDoc } from '../wasm/office_wasm'

export interface Transition {
  kind: string
  direction?: string
  durationMs?: number
  onClick: boolean
  afterMs?: number
}

/** One animation effect on a frame, as PowerPoint names it. */
export interface Effect {
  frame: number
  /** `entr`, `exit`, `emph` or `path`. */
  class: string
  /** 1 Appear, 2 Fly, 10 Fade, 22 Wipe… */
  preset: number
  subtype: number
  delayMs: number
  durationMs: number
  /** The paragraph of the frame's text it animates, when the text builds a paragraph at a time. */
  paragraph?: number
}

/** What one click plays; `auto` steps play by themselves after the one before. */
export interface AnimationStep {
  auto: boolean
  effects: Effect[]
}

export interface SlideInfo {
  id: string
  hidden: boolean
  layout: string | null
  layoutName: string | null
  notes: string
  transition?: Transition
  animations?: AnimationStep[]
  revision: number
  showMasterShapes: boolean
}

export interface LayoutInfo {
  path: string
  name: string
  type: string | null
}

export interface TextLook {
  fontFamily: string | null
  fontSize: number | null
  bold: boolean
  italic: boolean
  underline: boolean
  strikethrough: boolean
  color: string | null
  align: 'left' | 'center' | 'right' | 'justify'
}

export interface ShapeLook {
  fillMode: 'solid' | 'none'
  fillColor: string | null
  strokeColor: string | null
  strokeWidth: number
}

/** A frame on a slide, as the inspector reads it. Positions in slide pixels. */
export interface Element {
  id: string
  type: 'text' | 'shape' | 'connector' | 'image' | 'table' | 'group' | 'other'
  name: string
  linkTag?: string
  mathTag?: string
  x: number
  y: number
  width: number
  height: number
  rotation: number
  flipHorizontal: boolean
  flipVertical: boolean
  link?: { url?: string; anchor?: string; tooltip?: string }
  textStyle?: TextLook
  shapeStyle?: ShapeLook
  table?: { rows: number; columns: number; header: boolean; banded: boolean; style: string | null; textStyle?: TextLook }
  text: string
  placeholder?: string
  children?: Element[]
  media?: string
  crop?: [number, number, number, number]
}

export interface Box {
  id: string
  x: number
  y: number
  w: number
  h: number
  rot: number
}

export interface TextOverlay {
  frame: string
  x: number
  y: number
  w: number
  h: number
  rot: number
  caret?: [number, number, number]
  rects: [number, number, number, number][]
}

export interface DeckState {
  active: number
  frames: string[]
  editing: string | null
  canUndo: boolean
  canRedo: boolean
  dirty: boolean
  boxes: Box[]
  text: TextOverlay | null
  restack: string[]
}

let sheets = 0

export class SlidesEngine {
  /** Set while the document is shared (collab/session.ts): hears about every change, to send it. */
  collab: { changed(): void } | null = null
  readonly doc: SlidesDoc
  readonly styles: StyleSheet
  readonly media: MediaCache
  width = 1280
  height = 720
  slides: SlideInfo[] = []
  layouts: LayoutInfo[] = []
  theme: { heading: string; body: string; colors: Record<string, string> }
  state: DeckState = { active: 0, frames: [], editing: null, canUndo: false, canRedo: false, dirty: false, boxes: [], text: null, restack: [] }
  /** The active slide's frames. */
  elements: Element[] = []
  tick = 0
  /** Bumped when the slide list's JSON changes (content, notes, transitions, order). */
  slidesVersion = 0
  /** The JSON the slides and elements were last parsed from: unchanged ones keep their objects. */
  private slidesJson = ''
  private elementsJson = ''
  private drawn = new Map<string, { revision: number; html: string }>()
  private listeners = new Set<() => void>()
  private stopFonts: () => void
  private disposed = false

  private constructor(doc: SlidesDoc) {
    this.doc = doc
    this.styles = new StyleSheet(`p${++sheets}s`)
    this.media = new MediaCache((id) => {
      const bytes = this.doc.media(id)
      return bytes ? { bytes, type: this.doc.mediaType(id) ?? 'image/png' } : null
    })
    const size = JSON.parse(doc.size()) as { width: number; height: number }
    this.width = size.width
    this.height = size.height
    this.layouts = JSON.parse(doc.layouts())
    this.theme = JSON.parse(doc.theme())
    this.stopFonts = onFontsChanged(() => {
      if (this.disposed) return
      this.styles.remeasure()
      this.doc.fontsChanged()
      this.drawn.clear()
      this.refresh()
    })
  }

  /** Opens a PowerPoint file, or a new deck for `null`. */
  static async open(bytes: Uint8Array | null): Promise<SlidesEngine> {
    const engine = await loadEngine()
    expectKind(bytes, 'slides')
    await preloadFonts(BASE_FONTS)
    const doc = new engine.SlidesDoc(bytes ?? new Uint8Array(), host)
    const e = new SlidesEngine(doc)
    e.refresh()
    return e
  }

  subscribe = (f: () => void) => {
    this.listeners.add(f)
    return () => {
      this.listeners.delete(f)
    }
  }

  getTick = () => this.tick

  /** Runs a command; returns its result. */
  run<T = unknown>(cmd: Record<string, unknown> & { cmd: string }): T {
    const out = JSON.parse(this.doc.command(JSON.stringify(cmd))) as T
    this.refresh()
    return out
  }

  refresh() {
    if (this.disposed) return
    const slides = this.doc.slides()
    if (slides !== this.slidesJson) {
      this.slidesJson = slides
      this.slides = JSON.parse(slides)
      this.slidesVersion++
    }
    this.state = JSON.parse(this.doc.state())
    const elements = this.doc.elements(this.state.active)
    if (elements !== this.elementsJson) {
      this.elementsJson = elements
      this.elements = JSON.parse(elements)
    }
    this.tick++
    for (const f of this.listeners) f()
    this.collab?.changed()
  }

  /** A slide drawn, as HTML in slide pixels; kept until the slide changes. */
  html(index: number): string {
    const info = this.slides[index]
    if (!info) return ''
    const hit = this.drawn.get(info.id)
    if (hit && hit.revision === info.revision) return hit.html
    const p = JSON.parse(this.doc.paint(index)) as { items: PaintItem[]; styles: import('../paint').PaintStyle[] }
    this.styles.add(p.styles)
    const html = itemsHtml(p.items, this.styles, this.media.url)
    this.drawn.set(info.id, { revision: info.revision, html })
    return html
  }

  /** A slide's display list, with its styles in the style sheet: for drawing it elsewhere. */
  paintItems(index: number): PaintItem[] {
    const p = JSON.parse(this.doc.paint(index)) as { items: PaintItem[]; styles: import('../paint').PaintStyle[] }
    this.styles.add(p.styles)
    return p.items
  }

  insertLinkedTable(tag: string, rows: import('../sheets/engine').RangeCellSnapshot[][], name: string): string {
    const id = this.doc.insertLinkedTable(tag, JSON.stringify(rows), name)
    this.refresh()
    return id
  }

  updateLinkedTable(tag: string, rows: import('../sheets/engine').RangeCellSnapshot[][]): boolean {
    const ok = this.doc.updateLinkedTable(tag, JSON.stringify(rows))
    if (ok) this.refresh()
    return ok
  }

  insertLinkedImage(tag: string, bytes: Uint8Array, contentType: string, width: number, height: number, name: string): string {
    const id = this.doc.insertLinkedImage(tag, bytes, contentType, width, height, name)
    this.refresh()
    return id
  }

  updateLinkedImage(tag: string, bytes: Uint8Array, contentType: string): boolean {
    const ok = this.doc.updateLinkedImage(tag, bytes, contentType)
    if (ok) this.refresh()
    return ok
  }

  clearLinkTag(tag: string): boolean {
    const ok = this.doc.clearLinkTag(tag)
    if (ok) this.refresh()
    return ok
  }

  linkedTags(): { slide: number; frameId: string; tag: string }[] {
    return JSON.parse(this.doc.linkedTags()) as { slide: number; frameId: string; tag: string }[]
  }

  insertMathImage(tag: string, bytes: Uint8Array, contentType: string, width: number, height: number): string {
    const id = this.doc.insertMathImage(tag, bytes, contentType, width, height)
    this.refresh()
    return id
  }

  updateMathImage(id: number, tag: string, bytes: Uint8Array, contentType: string, width: number, height: number): boolean {
    const ok = this.doc.updateMathImage(id, tag, bytes, contentType, width, height)
    if (ok) this.refresh()
    return ok
  }

  /** Where this copy is, for the others while shared (collab/session.ts). */
  presence(): string {
    return this.doc.collabPresence()
  }

  /** Where someone else is on the slide shown, from their `presence`. */
  remoteOverlay(at: unknown): { boxes: Box[]; text?: TextOverlay } {
    return (JSON.parse(this.doc.remoteOverlay(JSON.stringify(at))) as { boxes: Box[]; text?: TextOverlay } | null) ?? { boxes: [] }
  }

  save(): Uint8Array {
    return this.doc.save()
  }

  dispose() {
    this.disposed = true
    this.stopFonts()
    this.styles.dispose()
    this.media.dispose()
    this.listeners.clear()
    this.doc.free()
  }
}
