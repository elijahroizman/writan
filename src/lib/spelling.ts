// Spelling for the documents Writan draws itself, Word documents and presentations, whose text
// the web view's own spell checker never sees. Words are checked by the system's checker
// (src-tauri/src/spelling.rs: macOS's, Windows', or Hunspell on Linux), in the user's language,
// and remembered so each is asked about once. Markdown and text files use the web view's.

import type { MenuEntry } from '../components/ui'
import { isTauri } from './platform'

interface Backend {
  /** The words given that are misspelled. */
  check(words: string[]): Promise<string[]>
  suggest(word: string): Promise<string[]>
  learn(word: string): Promise<void>
}

/** Test builds can stand in for the system's checker (window.__spellFake). */
function backend(): Backend | null {
  if (import.meta.env.DEV || import.meta.env.VITE_TEST_HOOKS) {
    const fake = (window as { __spellFake?: Backend }).__spellFake
    if (fake) return fake
  }
  if (!isTauri) return null
  const invoke = async <T,>(cmd: string, args: Record<string, unknown>) => (await import('@tauri-apps/api/core')).invoke<T>(cmd, args)
  return {
    check: (words) => invoke<string[]>('spell_check', { words }),
    suggest: (word) => invoke<string[]>('spell_suggest', { word }),
    learn: (word) => invoke<void>('spell_learn', { word }),
  }
}

export const spellingAvailable = () => !!backend()

/** Words already asked about: true when spelled right. */
const known = new Map<string, boolean>()
/** Words the user chose to ignore, until Writan quits. */
const ignored = new Set<string>()
const listeners = new Set<() => void>()

/** Tells documents to check again, after a word was learned or ignored. */
export function onSpellingChanged(f: () => void) {
  listeners.add(f)
  return () => {
    listeners.delete(f)
  }
}
const changed = () => listeners.forEach((f) => f())

export interface Word {
  word: string
  /** UTF-16 offsets into the text. */
  start: number
  end: number
}

const WORD = /[\p{L}\p{M}\p{N}]+(?:['’][\p{L}\p{M}]+)*/gu

/**
 * The words in `text` worth checking: not numbers or words with digits, not all-capital
 * abbreviations, not single letters, and not parts of web or email addresses or file names.
 */
export function wordsIn(text: string): Word[] {
  const out: Word[] = []
  for (const m of text.matchAll(WORD)) {
    const word = m[0]
    const start = m.index
    const end = start + word.length
    if (word.length < 2 || /\p{N}/u.test(word) || (word === word.toUpperCase() && word !== word.toLowerCase())) continue
    const before = text[start - 1] ?? ''
    const after = text[end] ?? ''
    // A dot or colon between letters (example.com, a:b) joins the parts of an address.
    const joined = (c: string | undefined) => !!c && /[\p{L}\p{N}]/u.test(c)
    if (/[@/\\_#]/.test(before) || (/[.:]/.test(before) && joined(text[start - 2]))) continue
    if (/[@/\\_]/.test(after) || (/[.:]/.test(after) && joined(text[end + 1]))) continue
    out.push({ word, start, end })
  }
  return out
}

/** Which of `words` are misspelled: asks the system about the ones not seen before. */
export async function misspelled(words: Iterable<string>): Promise<Set<string>> {
  const b = backend()
  const out = new Set<string>()
  if (!b) return out
  const unique = [...new Set(words)]
  const ask = unique.filter((w) => !known.has(w))
  for (let i = 0; i < ask.length; i += 400) {
    const chunk = ask.slice(i, i + 400)
    try {
      const bad = new Set(await b.check(chunk))
      for (const w of chunk) known.set(w, !bad.has(w))
    } catch {
      // No checker: nothing is underlined.
      for (const w of chunk) known.set(w, true)
    }
  }
  for (const w of unique) if (known.get(w) === false && !ignored.has(w)) out.add(w)
  return out
}

/**
 * The right-click menu's entries for a misspelled word: up to six corrections, then Ignore
 * Spelling and Learn Spelling. `replace` puts a correction in the word's place.
 */
export async function spellingMenu(word: string, replace: (with_: string) => void): Promise<MenuEntry[]> {
  const b = backend()
  if (!b) return []
  const guesses = (await b.suggest(word).catch(() => [] as string[])).slice(0, 6)
  const items: MenuEntry[] = guesses.length
    ? guesses.map((g) => ({ label: g, onSelect: () => replace(g) }))
    : [{ label: 'No Guesses Found', disabled: true, onSelect: () => {} }]
  items.push(
    {
      label: 'Ignore Spelling',
      separatorBefore: true,
      onSelect: () => {
        ignored.add(word)
        changed()
      },
    },
    {
      label: 'Learn Spelling',
      onSelect: () => {
        known.set(word, true)
        changed()
        void b.learn(word).catch(() => {})
      },
    },
  )
  return items
}
