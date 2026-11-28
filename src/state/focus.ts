import { create } from 'zustand'
import { track } from './analytics'

/**
 * Focus mode hides everything but the document. Typewriter scrolling keeps the line you're
 * typing at the same height on screen, and dimming fades every paragraph but the one with the
 * caret. The two can be on without focus mode; whether they are is remembered.
 */
interface FocusState {
  on: boolean
  typewriter: boolean
  dim: boolean
  toggle(): void
  setOn(on: boolean): void
  toggleTypewriter(): void
  toggleDim(): void
}

const KEY = 'workspace.focus'

function load(): { typewriter: boolean; dim: boolean } {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) || '{}') as { typewriter?: unknown; dim?: unknown }
    return { typewriter: saved.typewriter === true, dim: saved.dim !== false }
  } catch {
    return { typewriter: false, dim: true }
  }
}

const persist = () => {
  const { typewriter, dim } = useFocus.getState()
  localStorage.setItem(KEY, JSON.stringify({ typewriter, dim }))
}

export const useFocus = create<FocusState>((set, get) => ({
  on: false,
  ...load(),
  toggle: () => {
    if (!get().on) track('feature_used', { feature: 'focus_mode' })
    set({ on: !get().on })
  },
  setOn: (on) => set({ on }),
  toggleTypewriter: () => {
    if (!get().typewriter) track('feature_used', { feature: 'typewriter_scrolling' })
    set({ typewriter: !get().typewriter })
    persist()
  },
  toggleDim: () => {
    set({ dim: !get().dim })
    persist()
  },
}))

/** Dimming applies in focus mode only; typewriter scrolling whenever it's on. */
export const dimming = (s: FocusState) => s.on && s.dim
