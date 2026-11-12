// Misspelled words on the slide being edited: found a moment after its text changes, and placed
// by the engine (textRanges) so the slide view can underline them.

import { misspelled, onSpellingChanged, spellingAvailable, wordsIn } from '../../lib/spelling'
import type { SlidesEngine } from './engine'

export interface SlideWord {
  frame: string
  path: number[]
  /** UTF-16 offsets into the paragraph's text. */
  start: number
  end: number
  word: string
}

/** A frame's box and, inside it, rectangles under misspelled words: [word index, x, y, w, h]. */
export interface RangeBoxes {
  frame: string
  x: number
  y: number
  w: number
  h: number
  rot: number
  rects: [number, number, number, number, number][]
}

export interface SlideSpelling {
  slide: string
  words: SlideWord[]
  boxes: RangeBoxes[]
}

const MAX_WORDS = 500

/** Checks the active slide whenever it changes, and hands over what to underline. */
export function watchSlideSpelling(engine: SlidesEngine, show: (s: SlideSpelling | null) => void): () => void {
  if (!spellingAvailable()) return () => {}
  let timer = 0
  let stopped = false
  let seen = ''
  const current = () => engine.slides[engine.state.active]
  const keyOf = () => {
    const s = current()
    return s ? `${s.id}:${s.revision}` : ''
  }

  const check = async () => {
    const slide = current()
    if (stopped || !slide) return
    const key = keyOf()
    const texts = JSON.parse(engine.doc.slideText()) as { frame: string; path: number[]; text: string }[]
    const found = texts.map((t) => wordsIn(t.text))
    const bad = await misspelled(found.flatMap((ws) => ws.map((w) => w.word)))
    if (stopped) return
    if (keyOf() !== key) return schedule()
    const words: SlideWord[] = []
    texts.forEach((t, i) => {
      for (const w of found[i]) if (bad.has(w.word) && words.length < MAX_WORDS) words.push({ frame: t.frame, path: t.path, ...w })
    })
    let boxes: RangeBoxes[] = []
    try {
      if (words.length) boxes = JSON.parse(engine.doc.textRanges(JSON.stringify(words.map((w) => [w.frame, w.path, w.start, w.end]))))
    } catch {
      return
    }
    show({ slide: slide.id, words, boxes })
  }

  function schedule(delay = 400) {
    window.clearTimeout(timer)
    timer = window.setTimeout(() => void check(), delay)
  }

  const stop = engine.subscribe(() => {
    const key = keyOf()
    if (key === seen) return
    // Another slide: its words are checked straight away; an edit waits for a pause.
    const sameSlide = seen.split(':')[0] === key.split(':')[0]
    seen = key
    if (!sameSlide) show(null)
    schedule(sameSlide ? 400 : 0)
  })
  const stopChanges = onSpellingChanged(() => schedule(0))
  seen = keyOf()
  schedule(0)
  return () => {
    stopped = true
    window.clearTimeout(timer)
    stop()
    stopChanges()
  }
}

/** The underlined word at a point in slide pixels, if any. */
export function slideWordAt(s: SlideSpelling | null, x: number, y: number): SlideWord | null {
  if (!s) return null
  for (const b of s.boxes) {
    // Into the frame's own axes: it turns about its centre.
    const a = (-b.rot * Math.PI) / 180
    const cx = b.x + b.w / 2
    const cy = b.y + b.h / 2
    const lx = (x - cx) * Math.cos(a) - (y - cy) * Math.sin(a) + b.w / 2
    const ly = (x - cx) * Math.sin(a) + (y - cy) * Math.cos(a) + b.h / 2
    for (const [i, rx, ry, rw, rh] of b.rects) {
      if (lx >= rx && lx <= rx + rw && ly >= ry && ly <= ry + rh) return s.words[i] ?? null
    }
  }
  return null
}
