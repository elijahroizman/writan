// Text recognition for scanned pages. When a PDF has pages that are only a picture, they are
// recognized in the background, nearest the page being read first. The words go into the page's
// text layer, so they can be selected, copied, highlighted and found with ⌘F. The file is not
// changed.

import type { OcrPageResult } from '../../lib/ocr/types'
import { isCancelled, Priority } from './engine/client'
import type { Bitmap, Box, PageInfo, PageTextStats, TextRun } from './engine/protocol'
import type { DocStore } from './engine/store'

/**
 * A page that is mostly a picture, with little or no text of its own. Scans often carry a line or
 * two of real text (a page number, a "downloaded from" footer, a stamp), so a few lines count as
 * little. Blank pages and drawings aren't scans.
 */
export function isScannedPage(stats: PageTextStats): boolean {
  return stats.imageCoverage >= 0.3 && (stats.nonWhitespaceCount < 10 || stats.textCoverage < 0.03)
}

/** What was recognized on one page, in display points. */
export interface PageOcrText {
  /** One per word, in reading order. */
  runs: TextRun[]
  /** The words, joined by single spaces, as searched. */
  text: string
  /** For each UTF-16 unit of `text`: the word it's in, or -1 for the spaces between words. */
  word: Int32Array
  /** Its position within that word. */
  offset: Int32Array
  /** The line each word is on. */
  line: Int32Array
}

/** `own` is where the page's own text is: words recognized there are left out, as they're already there. */
export function pageText(result: OcrPageResult, page: Pick<PageInfo, 'width' | 'height'>, own: Box[] = []): PageOcrText {
  const sx = result.widthPx > 0 ? page.width / result.widthPx : 1
  const sy = result.heightPx > 0 ? page.height / result.heightPx : 1
  const sideways = result.detectedRotation === 90 || result.detectedRotation === 270
  const runs: TextRun[] = []
  const lines: number[] = []
  let lineNo = 0
  for (const block of result.blocks) {
    for (const para of block.paragraphs) {
      for (const line of para.lines) {
        for (const word of line.words) {
          const text = word.text.trim()
          if (!text) continue
          const box = { x: word.bbox.x * sx, y: word.bbox.y * sy, w: word.bbox.w * sx, h: word.bbox.h * sy }
          if (box.w <= 0 || box.h <= 0) continue
          const cx = box.x + box.w / 2
          const cy = box.y + box.h / 2
          if (own.some((b) => cx >= b.x && cx <= b.x + b.w && cy >= b.y && cy <= b.y + b.h)) continue
          runs.push({ text, map: [], box, ocr: true, vertical: (sideways && box.h > box.w) || undefined })
          lines.push(lineNo)
        }
        lineNo++
      }
    }
  }
  let text = ''
  const word: number[] = []
  const offset: number[] = []
  runs.forEach((run, i) => {
    if (i) {
      text += ' '
      word.push(-1)
      offset.push(0)
    }
    text += run.text
    for (let j = 0; j < run.text.length; j++) {
      word.push(i)
      offset.push(j)
    }
  })
  return { runs, text, word: Int32Array.from(word), offset: Int32Array.from(offset), line: Int32Array.from(lines) }
}

/**
 * The page's own text and its recognized words in one reading order: each run of its own goes
 * before the first word to its right on its line, or else below it, so a selection dragged into a
 * footer doesn't take the whole page and a label stays before what follows it.
 */
export function withRecognized(own: TextRun[], words: TextRun[]): TextRun[] {
  const out = words.slice()
  for (const run of own) {
    const { x, y, h } = run.box
    const middle = y + h / 2
    const sameLine = (w: TextRun) => w.box.y < middle && w.box.y + w.box.h > middle
    const at = out.findIndex((w) => w.ocr && (sameLine(w) ? w.box.x > x : w.box.y > middle))
    out.splice(at === -1 ? out.length : at, 0, run)
  }
  return out
}

/** The part of a recognized word covering its characters [from, to), estimated by even spacing. */
export function wordPart(run: TextRun, from: number, to: number): Box {
  const n = run.text.length || 1
  const { x, y, w, h } = run.box
  if (run.vertical) return { x, y: y + (h * from) / n, w, h: (h * (to - from)) / n }
  // Hebrew and Arabic words start at the right.
  const rtl = /[\p{Script=Hebrew}\p{Script=Arabic}]/u.test(run.text)
  const start = rtl ? n - to : from
  return { x: x + (w * start) / n, y, w: (w * (to - from)) / n, h }
}

// Curly quotes and dashes, as recognition writes them, match what's typed.
const FOLD: Record<string, string> = { '‘': "'", '’': "'", '‚': "'", '“': '"', '”': '"', '„': '"', '–': '-', '—': '-', ' ': ' ' }

/** Lower case and plain punctuation, one UTF-16 unit for one, so positions still line up. */
function fold(text: string): string {
  let out = ''
  for (const ch of text) {
    const mapped = FOLD[ch] ?? ch.toLowerCase()
    out += mapped.length === ch.length ? mapped : ch
  }
  return out
}

/** Matches of `query` in the recognized text: for each, one box per line it spans. */
export function searchPageText(page: PageOcrText, query: string): Box[][] {
  const needle = fold(query.trim().replace(/\s+/g, ' '))
  if (!needle) return []
  const haystack = fold(page.text)
  const hits: Box[][] = []
  for (let at = haystack.indexOf(needle); at !== -1; at = haystack.indexOf(needle, at + needle.length)) {
    const boxes: Box[] = []
    let lineNo = -1
    for (let i = at; i < at + needle.length; ) {
      const w = page.word[i]
      if (w === -1) {
        i++
        continue
      }
      const from = page.offset[i]
      let to = from
      while (i < at + needle.length && page.word[i] === w) {
        to++
        i++
      }
      const part = wordPart(page.runs[w], from, to)
      const last = boxes[boxes.length - 1]
      if (last && page.line[w] === lineNo) boxes[boxes.length - 1] = union(last, part)
      else boxes.push(part)
      lineNo = page.line[w]
    }
    if (boxes.length) hits.push(boxes)
  }
  return hits
}

function union(a: Box, b: Box): Box {
  const x = Math.min(a.x, b.x)
  const y = Math.min(a.y, b.y)
  return { x, y, w: Math.max(a.x + a.w, b.x + b.w) - x, h: Math.max(a.y + a.h, b.y + b.h) - y }
}

/** Pixels per point to recognize at: about 220 dpi, and no side longer than 2800 pixels. */
function recognitionScale(page: PageInfo) {
  return Math.min(220 / 72, 2800 / Math.max(1, page.width, page.height))
}

function bitmapPixels(bitmap: Bitmap): Uint8ClampedArray {
  const { width, height, bands } = bitmap
  const out = new Uint8ClampedArray(width * height * 4)
  let canvas: HTMLCanvasElement | undefined
  for (const band of bands) {
    if (band.data) {
      out.set(new Uint8ClampedArray(band.data), band.y * width * 4)
    } else if (band.image) {
      canvas ??= Object.assign(document.createElement('canvas'), { width, height: band.height })
      if (canvas.height < band.height) canvas.height = band.height
      const ctx = canvas.getContext('2d', { willReadFrequently: true })!
      ctx.clearRect(0, 0, width, band.height)
      ctx.drawImage(band.image, 0, 0)
      out.set(ctx.getImageData(0, 0, width, band.height).data, band.y * width * 4)
      band.image.close()
    }
  }
  if (canvas) canvas.width = canvas.height = 0
  return out
}

export interface OcrProgress {
  done: number
  total: number
}

/**
 * Recognize `pages` one at a time, always taking the one nearest `current()` next, until all are
 * done or `signal` aborts. Each result goes to the store as soon as it's ready.
 */
export async function recognizePages(opts: {
  store: DocStore
  pages: PageInfo[]
  current: () => number
  signal: AbortSignal
  onProgress: (progress: OcrProgress | null) => void
  onPage: (page: PageInfo) => void
}): Promise<void> {
  const { store, signal, onProgress, onPage } = opts
  const remaining = opts.pages.slice()
  const total = remaining.length
  if (!total) return
  onProgress({ done: 0, total })
  const [{ OcrClient }, { availableLangs }] = await Promise.all([import('../../lib/ocr/client'), import('../../lib/ocr/packs')])
  const client = new OcrClient()
  let failures = 0
  try {
    const langs = await availableLangs(signal)
    while (remaining.length && !signal.aborted) {
      const here = opts.current()
      const indexOf = (p: PageInfo) => store.pages.findIndex((q) => q.key === p.key)
      remaining.sort((a, b) => Math.abs(indexOf(a) - here) - Math.abs(indexOf(b) - here))
      const page = remaining[0]
      try {
        const bitmap = await store.engine.call('render', [page.key, recognitionScale(page), null], { priority: Priority.ocr, signal })
        const result = await client.recognize({ data: bitmapPixels(bitmap), width: bitmap.width, height: bitmap.height }, { langs, signal })
        const own = await store.engine.call('text', [page.key], { priority: Priority.ocr, signal })
        signal.throwIfAborted()
        store.setOcr(page, pageText(result, page, own.map((run) => run.box)))
        onPage(page)
        remaining.shift()
      } catch (err) {
        if (signal.aborted || isCancelled(err)) return
        // A pack that changed under us is worth another try; a page that won't read is skipped.
        console.warn('Text recognition failed', err)
        if (++failures < 3) continue
        remaining.shift()
      }
      failures = 0
      onProgress({ done: total - remaining.length, total })
    }
  } finally {
    client.dispose()
    if (!signal.aborted) onProgress(null)
  }
}
