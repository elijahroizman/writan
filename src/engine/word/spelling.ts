// Misspelled words in a Word document, underlined with the engine's marks (set `spelling`) a
// moment after typing stops. The word being typed waits until the caret leaves it.

import { misspelled, onSpellingChanged, spellingAvailable, wordsIn } from '../../lib/spelling'
import type { WordEngine } from './engine'

export const SPELLING = 'spelling'
/** Underlining every word of a pasted foreign-language page is no help; Word stops too. */
const MAX_MARKS = 2000

export interface SpellingMark {
  path: number[]
  start: number
  end: number
  word: string
}

/** The misspelled words now underlined, by mark id. */
export const spellingMarks = new WeakMap<WordEngine, Map<string, SpellingMark>>()

/** Starts checking `engine`'s document; the returned function stops. */
export function watchSpelling(engine: WordEngine): () => void {
  if (!spellingAvailable()) return () => {}
  engine.markStyles[SPELLING] = { color: 'transparent', squiggle: true }
  let timer = 0
  let stopped = false
  let checkedRevision = -1
  /** A misspelled word left alone because the caret was at its end, still being typed. */
  let waiting: { path: string; start: number; end: number } | null = null

  const caret = () => {
    const sel = JSON.parse(engine.doc.selection()) as { anchor: [number[], number]; focus: [number[], number]; body: boolean }
    const collapsed = sel.anchor[0].join('.') === sel.focus[0].join('.') && sel.anchor[1] === sel.focus[1]
    return collapsed && sel.body ? { path: sel.focus[0].join('.'), offset: sel.focus[1] } : null
  }

  const check = async () => {
    if (stopped) return
    const revision = engine.revision
    const paragraphs = JSON.parse(engine.doc.paragraphs()) as { path: number[]; text: string }[]
    const found = paragraphs.map((p) => wordsIn(p.text))
    const bad = await misspelled(found.flatMap((ws) => ws.map((w) => w.word)))
    if (stopped) return
    // Typed into meanwhile: offsets have moved, so check again shortly.
    if (engine.revision !== revision) return schedule()
    const at = caret()
    waiting = null
    const marks = new Map<string, SpellingMark>()
    paragraphs.forEach((p, i) => {
      const key = p.path.join('.')
      for (const w of found[i]) {
        if (!bad.has(w.word) || marks.size >= MAX_MARKS) continue
        if (at && at.path === key && at.offset === w.end) {
          waiting = { path: key, start: w.start, end: w.end }
          continue
        }
        marks.set(String(marks.size), { path: p.path, start: w.start, end: w.end, word: w.word })
      }
    })
    spellingMarks.set(engine, marks)
    checkedRevision = revision
    try {
      engine.run({ cmd: 'setMarks', set: SPELLING, marks: [...marks].map(([id, m]) => [id, m.path, m.start, m.end]) })
    } catch {
      // The document closed while the checker answered.
    }
  }

  function schedule(delay = 400) {
    window.clearTimeout(timer)
    timer = window.setTimeout(() => void check(), delay)
  }

  const stopEngine = engine.subscribe(() => {
    if (engine.revision !== checkedRevision) return schedule()
    if (waiting) {
      const at = caret()
      if (!at || at.path !== waiting.path || at.offset < waiting.start || at.offset > waiting.end) schedule(0)
    }
  })
  const stopChanges = onSpellingChanged(() => schedule(0))
  schedule(0)
  return () => {
    stopped = true
    window.clearTimeout(timer)
    stopEngine()
    stopChanges()
  }
}

/** The underlined word at a point on the screen, if any. */
export function spellingAt(engine: WordEngine, clientX: number, clientY: number): SpellingMark | null {
  const at = engine.pagePoint?.(clientX, clientY)
  const marks = spellingMarks.get(engine)
  if (!at || !marks) return null
  for (const [page, x, y, w, h, set, id] of engine.overlay.marks ?? []) {
    if (set !== SPELLING || page !== at.page || at.x < x || at.x > x + w || at.y < y || at.y > y + h) continue
    return marks.get(id) ?? null
  }
  return null
}
