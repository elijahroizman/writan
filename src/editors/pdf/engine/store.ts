// Shared state for one open PDF: the engine, the page list, and small caches the page view and
// the thumbnails both use. Pages are identified by `key` (their object number), so cached text
// and thumbnails follow a page when it moves.

import type { PdfEngine } from './client'
import { isCancelled, Priority } from './client'
import type { PageOcrText } from '../ocr'
import type { Annot, Bitmap, PageInfo, TextRun } from './protocol'

export const THUMB_WIDTH = 150

/** Least-recently-used map with an optional size budget. */
class Lru<V> {
  private map = new Map<string, { value: V; size: number }>()
  private total = 0
  constructor(
    private maxEntries: number,
    private maxSize = Infinity,
    private onEvict?: (value: V) => void,
  ) {}
  get(key: string) {
    const hit = this.map.get(key)
    if (!hit) return undefined
    this.map.delete(key)
    this.map.set(key, hit)
    return hit.value
  }
  set(key: string, value: V, size = 0) {
    this.delete(key)
    this.map.set(key, { value, size })
    this.total += size
    for (const [k] of this.map) {
      if (this.map.size <= this.maxEntries && this.total <= this.maxSize) break
      this.delete(k)
    }
  }
  delete(key: string) {
    const hit = this.map.get(key)
    if (!hit) return
    this.map.delete(key)
    this.total -= hit.size
    this.onEvict?.(hit.value)
  }
  clear() {
    for (const key of [...this.map.keys()]) this.delete(key)
  }
}

export interface Thumb {
  url: string
  rotation: number
  rev: number
}

export class DocStore {
  pages: PageInfo[] = []
  /** Bumped when a page's content changes (annotations), so its renders are redone. */
  private revs = new Map<number, number>()
  private listeners = new Set<() => void>()
  private text = new Lru<Promise<TextRun[]>>(48)
  private annots = new Lru<Promise<Annot[]>>(48)
  /** Text recognized on scanned pages, by `key:rotation`. Small, and slow to make again. */
  private ocr = new Map<string, PageOcrText>()
  /** Thumbnails as small JPEGs, newest version per page. Cheap enough to keep many. */
  private thumbs = new Lru<Thumb>(4000, 48 * 1024 * 1024, (t) => URL.revokeObjectURL(t.url))
  private thumbJobs = new Map<string, Promise<Thumb | null>>()
  private destroyed = false

  constructor(readonly engine: PdfEngine) {}

  subscribe(fn: () => void) {
    this.listeners.add(fn)
    return () => void this.listeners.delete(fn)
  }
  private emit() {
    for (const fn of this.listeners) fn()
  }

  setPages(pages: PageInfo[]) {
    this.pages = pages
    this.emit()
  }

  /** After the engine reopened the file: keys may now name other pages, so forget what we cached. */
  reopened(pages: PageInfo[]) {
    const same = pages.length === this.pages.length && pages.every((p, i) => p.key === this.pages[i].key && p.rotation === this.pages[i].rotation)
    if (!same) {
      this.text.clear()
      this.ocr.clear()
      this.annots.clear()
      this.thumbs.clear()
      this.revs.clear()
    }
    this.setPages(pages)
  }

  /** A different document now (a recompiled preview): drop every cache and redraw every page. */
  replaced(pages: PageInfo[]) {
    this.text.clear()
    this.ocr.clear()
    this.annots.clear()
    this.thumbs.clear()
    // Revs only go up, so pages that keep their key still redraw.
    for (const page of pages) this.revs.set(page.key, this.rev(page.key) + 1)
    this.setPages(pages)
  }

  rev(key: number) {
    return this.revs.get(key) ?? 0
  }

  /** A page's drawing changed (annotation added, moved, removed, form filled). */
  touch(index: number) {
    const page = this.pages[index]
    if (!page) return
    this.revs.set(page.key, this.rev(page.key) + 1)
    this.pages = this.pages.slice()
    this.emit()
  }

  /** Text recognized on a scanned page, as it's now turned. */
  ocrText(page: PageInfo): PageOcrText | undefined {
    return this.ocr.get(`${page.key}:${page.rotation}`)
  }

  setOcr(page: PageInfo, text: PageOcrText) {
    const id = `${page.key}:${page.rotation}`
    this.ocr.set(id, text)
    this.text.delete(id)
    this.pages = this.pages.slice()
    this.emit()
  }

  /** The page's own text, then any recognized on it (scanned pages have little or none of their own). */
  getText(index: number): Promise<TextRun[]> {
    const page = this.pages[index]
    const id = `${page.key}:${page.rotation}`
    let hit = this.text.get(id)
    if (!hit) {
      const ocr = this.ocr.get(id)
      hit = this.engine.call('text', [page.key], { priority: Priority.text }).then((runs) => (ocr ? [...runs, ...ocr.runs] : runs))
      hit.catch(() => this.text.delete(id))
      this.text.set(id, hit)
    }
    return hit
  }

  getAnnotations(index: number): Promise<Annot[]> {
    const page = this.pages[index]
    const id = `${page.key}:${page.rotation}:${this.rev(page.key)}`
    let hit = this.annots.get(id)
    if (!hit) {
      hit = this.engine.call('annotations', [page.key], { priority: Priority.text })
      hit.catch(() => this.annots.delete(id))
      this.annots.set(id, hit)
    }
    return hit
  }

  /** The freshest thumbnail we have for a page, even if it's from before a rotation. */
  thumb(index: number): Thumb | undefined {
    const page = this.pages[index]
    return page && this.thumbs.get(String(page.key))
  }

  /** Make an up-to-date thumbnail (low priority; cancelled with `signal`). */
  loadThumb(index: number, signal: AbortSignal, priority: number = Priority.thumbnails): Promise<Thumb | null> {
    const page = this.pages[index]
    const rev = this.rev(page.key)
    const have = this.thumbs.get(String(page.key))
    if (have && have.rotation === page.rotation && have.rev === rev) return Promise.resolve(have)
    const job = `${page.key}:${page.rotation}:${rev}`
    let pending = this.thumbJobs.get(job)
    if (!pending) {
      const scale = (THUMB_WIDTH * devicePixelRatioCapped()) / page.width
      pending = this.engine
        .call('render', [page.key, scale, null], { priority, signal })
        .then((bitmap) => this.storeThumb(page.key, page.rotation, rev, bitmap))
        .catch((err) => {
          if (!isCancelled(err)) console.warn(err)
          return null
        })
        .finally(() => forget())
      const forget = () => this.thumbJobs.get(job) === pending && this.thumbJobs.delete(job)
      signal.addEventListener('abort', forget, { once: true })
      this.thumbJobs.set(job, pending)
    }
    return pending
  }

  /** Reuse a full-size page render for the thumbnail, so scanned pages aren't decoded twice. */
  offerThumb(index: number, source: HTMLCanvasElement) {
    const page = this.pages[index]
    if (!page) return
    const rev = this.rev(page.key)
    const have = this.thumbs.get(String(page.key))
    if (have && have.rotation === page.rotation && have.rev === rev) return
    const w = Math.round(THUMB_WIDTH * devicePixelRatioCapped())
    const h = Math.max(1, Math.round((w * source.height) / source.width))
    const canvas = document.createElement('canvas')
    canvas.width = w
    canvas.height = h
    const ctx = canvas.getContext('2d')!
    ctx.imageSmoothingQuality = 'high'
    ctx.drawImage(source, 0, 0, w, h)
    void this.encode(canvas).then((blob) => blob && this.putThumb(page.key, page.rotation, rev, blob))
  }

  private async storeThumb(key: number, rotation: number, rev: number, bitmap: Bitmap): Promise<Thumb | null> {
    const canvas = document.createElement('canvas')
    canvas.width = bitmap.width
    canvas.height = bitmap.height
    drawBitmap(canvas, bitmap)
    const blob = await this.encode(canvas)
    return blob ? this.putThumb(key, rotation, rev, blob) : null
  }

  private encode(canvas: HTMLCanvasElement) {
    return new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/jpeg', 0.85)).finally(() => {
      canvas.width = canvas.height = 0
    })
  }

  private putThumb(key: number, rotation: number, rev: number, blob: Blob): Thumb | null {
    if (this.destroyed) return null
    const have = this.thumbs.get(String(key))
    if (have && have.rev > rev) return have
    const thumb = { url: URL.createObjectURL(blob), rotation, rev }
    this.thumbs.set(String(key), thumb, blob.size)
    this.emit()
    return thumb
  }

  destroy() {
    this.destroyed = true
    this.thumbs.clear()
    this.text.clear()
    this.ocr.clear()
    this.annots.clear()
    this.listeners.clear()
    this.engine.destroy()
  }
}

/** Retina is plenty for page images; higher ratios just cost memory. */
export function devicePixelRatioCapped() {
  return Math.min(2, window.devicePixelRatio || 1)
}

/** Paint a worker bitmap into a canvas of the same size. */
export function drawBitmap(canvas: HTMLCanvasElement, bitmap: Bitmap) {
  const ctx = canvas.getContext('2d')!
  for (const band of bitmap.bands) {
    if (band.image) {
      ctx.drawImage(band.image, 0, band.y)
      band.image.close()
    } else if (band.data) {
      ctx.putImageData(new ImageData(new Uint8ClampedArray(band.data), bitmap.width, band.height), 0, band.y)
    }
  }
}
