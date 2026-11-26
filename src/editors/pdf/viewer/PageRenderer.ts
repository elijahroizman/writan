// Draws one page: a whole-page canvas (capped in size) plus sharp tiles for the part on screen
// when zoomed in past what the cap allows. Old drawings stay up, stretched, until new ones
// arrive, so zooming and rotating never flash blank.

import { discardBitmap, isCancelled, Priority } from '../engine/client'
import type { Bitmap, Box, PageInfo } from '../engine/protocol'
import type { DocStore } from '../engine/store'
import { devicePixelRatioCapped, drawBitmap } from '../engine/store'

/** Biggest whole-page bitmap, in pixels (about 16 MB). */
const MAX_BASE_PIXELS = 4_200_000
/** Tile edge in device pixels. */
const TILE = 1024
/** Extra CSS pixels around the window to have tiles ready for. */
const TILE_MARGIN = 200

export interface RenderParams {
  page: PageInfo
  rev: number
  /** CSS pixels per point. */
  unit: number
  priority: number
}

interface Tile {
  canvas: HTMLCanvasElement
  /** Area covered, in points. */
  rect: Box
}

export class PageRenderer {
  private base: HTMLCanvasElement | null = null
  private baseRotation = 0
  private baseSig = ''
  private contentSig = ''
  private baseJob: AbortController | null = null
  private tiles = new Map<string, Tile>()
  private tileJobs = new Map<string, AbortController>()
  private params: RenderParams | null = null
  /** The part of the page on screen, in page CSS pixels. */
  private visibleRect: Box | null = null
  private tileTimer = 0
  private destroyed = false

  constructor(
    private host: HTMLElement,
    private pageEl: HTMLElement,
    private store: DocStore,
  ) {}

  update(params: RenderParams) {
    this.params = params
    const { page, rev, unit, priority } = params
    const dpr = devicePixelRatioCapped()
    const full = unit * dpr
    const baseScale = Math.min(full, Math.sqrt(MAX_BASE_PIXELS / (page.width * page.height)))
    const contentSig = `${page.key}:${page.rotation}:${rev}`
    const sig = `${contentSig}:${baseScale.toFixed(5)}`
    if (sig !== this.baseSig) {
      if (this.base && page.rotation !== this.baseRotation) this.rotateStale(page.rotation - this.baseRotation, page)
      if (contentSig.split(':')[1] !== this.contentSig.split(':')[1]) this.dropTiles(() => true)
      this.baseSig = sig
      this.contentSig = contentSig
      this.baseJob?.abort()
      const job = new AbortController()
      this.baseJob = job
      this.pageEl.dataset.loaded = 'false'
      this.store.engine
        .call('render', [page.key, baseScale, null], { priority, signal: job.signal })
        .then((bitmap) => {
          if (job.signal.aborted || this.destroyed) return discardBitmap(bitmap)
          this.showBase(bitmap, page.rotation)
          this.pageEl.dataset.loaded = 'true'
          this.pageEl.dispatchEvent(new CustomEvent('pagerendered', { bubbles: true, detail: { key: page.key, rev } }))
          if (this.params && this.wantedTiles().size === 0) this.dropTiles(() => true)
          const index = this.store.pages.findIndex((p) => p.key === page.key)
          if (index !== -1 && this.base && this.store.hasThumbSubscribers()) this.store.offerThumb(index, this.base)
        })
        .catch((err) => {
          if (!isCancelled(err)) console.warn(err)
        })
    }
    this.scheduleTiles()
  }

  /** Where the window is, relative to this page (CSS pixels). */
  setVisibleRect(rect: Box | null) {
    this.visibleRect = rect
    this.scheduleTiles()
  }

  private showBase(bitmap: Bitmap, rotation: number) {
    const canvas = document.createElement('canvas')
    canvas.className = 'page-base'
    canvas.width = bitmap.width
    canvas.height = bitmap.height
    drawBitmap(canvas, bitmap)
    this.host.prepend(canvas)
    if (this.base) release(this.base)
    this.base = canvas
    this.baseRotation = rotation
  }

  /** While a rotated page redraws, turn the old drawing so it already looks right. */
  private rotateStale(quarterTurns: number, page: PageInfo) {
    const base = this.base!
    const turns = ((quarterTurns % 4) + 4) % 4
    if (turns % 2 === 1) {
      base.style.width = `${(page.height / page.width) * 100}%`
      base.style.height = `${(page.width / page.height) * 100}%`
    } else {
      base.style.width = base.style.height = ''
    }
    base.style.left = base.style.top = '50%'
    base.style.transform = `translate(-50%, -50%) rotate(${turns * 90}deg)`
  }

  private scheduleTiles() {
    window.clearTimeout(this.tileTimer)
    this.tileTimer = window.setTimeout(() => this.updateTiles(), 50)
  }

  /** Tile ids that should be on screen now, with the page rectangle (points) each covers. */
  private wantedTiles(): Map<string, Box> {
    const wanted = new Map<string, Box>()
    const p = this.params
    const vis = this.visibleRect
    if (!p || !vis) return wanted
    const { page, unit } = p
    const dpr = devicePixelRatioCapped()
    const full = unit * dpr
    const baseScale = Math.min(full, Math.sqrt(MAX_BASE_PIXELS / (page.width * page.height)))
    if (full <= baseScale * 1.1) return wanted
    const W = Math.round(page.width * full)
    const H = Math.round(page.height * full)
    const x0 = Math.max(0, (vis.x - TILE_MARGIN) * dpr)
    const y0 = Math.max(0, (vis.y - TILE_MARGIN) * dpr)
    const x1 = Math.min(W, (vis.x + vis.w + TILE_MARGIN) * dpr)
    const y1 = Math.min(H, (vis.y + vis.h + TILE_MARGIN) * dpr)
    if (x1 <= x0 || y1 <= y0) return wanted
    const scale = W / page.width
    for (let ty = Math.floor(y0 / TILE); ty * TILE < y1; ty++) {
      for (let tx = Math.floor(x0 / TILE); tx * TILE < x1; tx++) {
        const w = Math.min(TILE, W - tx * TILE)
        const h = Math.min(TILE, H - ty * TILE)
        wanted.set(`${this.contentSig}:${W}:${tx}:${ty}`, { x: (tx * TILE) / scale, y: (ty * TILE) / scale, w: w / scale, h: h / scale })
      }
    }
    return wanted
  }

  private updateTiles() {
    if (this.destroyed || !this.params) return
    const wanted = this.wantedTiles()
    for (const [id, job] of this.tileJobs) {
      if (!wanted.has(id)) {
        job.abort()
        this.tileJobs.delete(id)
      }
    }
    if (!wanted.size) {
      // Nothing to sharpen: the whole-page drawing is enough (old tiles go once it's redrawn).
      if (this.pageEl.dataset.loaded === 'true') this.dropTiles(() => true)
      return
    }
    const { page, unit } = this.params
    const dpr = devicePixelRatioCapped()
    const W = Math.round(page.width * unit * dpr)
    const scale = W / page.width
    const vis = this.visibleRect!
    const cx = (vis.x + vis.w / 2) / unit
    const cy = (vis.y + vis.h / 2) / unit
    for (const [id, rect] of wanted) {
      if (this.tiles.has(id) || this.tileJobs.has(id)) continue
      const job = new AbortController()
      this.tileJobs.set(id, job)
      const distance = Math.hypot(rect.x + rect.w / 2 - cx, rect.y + rect.h / 2 - cy) / Math.max(page.width, page.height)
      this.store.engine
        .call('render', [page.key, scale, rect], { priority: Priority.tiles + distance, signal: job.signal })
        .then((bitmap) => {
          this.tileJobs.delete(id)
          if (job.signal.aborted || this.destroyed) return discardBitmap(bitmap)
          const canvas = document.createElement('canvas')
          canvas.className = 'page-tile'
          canvas.width = bitmap.width
          canvas.height = bitmap.height
          drawBitmap(canvas, bitmap)
          canvas.style.left = `${(rect.x / page.width) * 100}%`
          canvas.style.top = `${(rect.y / page.height) * 100}%`
          canvas.style.width = `${(rect.w / page.width) * 100}%`
          canvas.style.height = `${(rect.h / page.height) * 100}%`
          this.host.append(canvas)
          this.tiles.set(id, { canvas, rect })
          // Once everything on screen is sharp at the new zoom, drop leftovers from the old one.
          const now = this.wantedTiles()
          if ([...now.keys()].every((k) => this.tiles.has(k))) this.dropTiles((t) => !now.has(t))
        })
        .catch((err) => {
          this.tileJobs.delete(id)
          if (!isCancelled(err)) console.warn(err)
        })
    }
    // Tiles (of any zoom) well off screen go right away.
    const m = TILE_MARGIN * 2
    const area = { x: (vis.x - m) / unit, y: (vis.y - m) / unit, w: (vis.w + 2 * m) / unit, h: (vis.h + 2 * m) / unit }
    this.dropTiles((id, tile) => !wanted.has(id) && !intersects(tile.rect, area))
  }

  private dropTiles(which: (id: string, tile: Tile) => boolean) {
    for (const [id, tile] of this.tiles) {
      if (!which(id, tile)) continue
      release(tile.canvas)
      this.tiles.delete(id)
    }
  }

  destroy() {
    this.destroyed = true
    window.clearTimeout(this.tileTimer)
    this.baseJob?.abort()
    for (const job of this.tileJobs.values()) job.abort()
    this.tileJobs.clear()
    this.dropTiles(() => true)
    if (this.base) release(this.base)
    this.base = null
  }
}

const intersects = (a: Box, b: Box) => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h

/** Remove a canvas and free its pixels now (WebKit otherwise holds them until GC). */
function release(canvas: HTMLCanvasElement) {
  canvas.remove()
  canvas.width = canvas.height = 0
}
