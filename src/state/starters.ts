import { create } from 'zustand'
import type { DocKind } from '../lib/kinds'
import { useWorkspace } from './workspace'

/**
 * The starter decks New presentation offers. Each sets the deck's theme colors and fonts and
 * starts with six slides in its style; the engine builds them (crates/office-slides, starters.rs).
 */
export const STARTERS = [
  { id: 'plain', name: 'Plain' },
  { id: 'harbor', name: 'Harbor' },
  { id: 'graphite', name: 'Graphite' },
  { id: 'signal', name: 'Signal' },
  { id: 'lagoon', name: 'Lagoon' },
  { id: 'blank', name: 'Blank' },
] as const
export type StarterId = (typeof STARTERS)[number]['id']

const KEY = 'workspace.starter'

function lastPick(): StarterId {
  try {
    const saved = localStorage.getItem(KEY)
    return STARTERS.find((s) => s.id === saved)?.id ?? 'plain'
  } catch {
    return 'plain'
  }
}

interface StarterState {
  open: boolean
  /** The one picked last time, which the gallery selects first. */
  last: StarterId
  setOpen(open: boolean): void
  remember(id: StarterId): void
}

export const useStarters = create<StarterState>((set) => ({
  open: false,
  last: lastPick(),
  setOpen: (open) => set({ open }),
  remember: (id) => {
    set({ last: id })
    try {
      localStorage.setItem(KEY, id)
    } catch {
      // Not remembered; the gallery starts on Plain next time.
    }
  },
}))

/** New document from the UI: a presentation opens the starter gallery, anything else starts blank. */
export function createNew(kind: DocKind) {
  if (kind === 'pptx') useStarters.getState().setOpen(true)
  else useWorkspace.getState().newDocument(kind)
}
