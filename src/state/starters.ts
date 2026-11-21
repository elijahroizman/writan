import { create } from 'zustand'
import type { DocKind } from '../lib/kinds'
import { useWorkspace } from './workspace'

/**
 * The starter decks. Each sets the deck's theme colors and fonts and starts with six slides in
 * its style; the engine builds them (crates/office-slides, starters.rs). `about` and `tags` are
 * what the gallery's search matches, besides the name.
 */
export const STARTERS = [
  { id: 'plain', name: 'Plain', about: 'White with a blue accent. Six slides.', tags: 'light white simple clean minimal arial' },
  { id: 'harbor', name: 'Harbor', about: 'Navy bands and blue blocks, for reports and plans. Six slides.', tags: 'light blue navy business report plan calibri' },
  { id: 'graphite', name: 'Graphite', about: 'Dark, for talks, with a slide for code. Six slides.', tags: 'dark black talk conference code tech developer' },
  { id: 'signal', name: 'Signal', about: 'Bold yellow, for pitches. Six slides.', tags: 'light yellow bold pitch startup' },
  { id: 'lagoon', name: 'Lagoon', about: 'Teal and coral with serif headings. Six slides.', tags: 'light teal green coral serif warm georgia' },
  { id: 'blank', name: 'Blank', about: 'One empty slide in the Office theme.', tags: 'empty nothing office white' },
] as const
export type StarterId = (typeof STARTERS)[number]['id']

const KEY = 'workspace.starter'

function savedDefault(): StarterId {
  try {
    const saved = localStorage.getItem(KEY)
    return STARTERS.find((s) => s.id === saved)?.id ?? 'plain'
  } catch {
    return 'plain'
  }
}

interface StarterState {
  open: boolean
  /** The deck New presentation makes without asking, and the one the gallery selects first. */
  default: StarterId
  setOpen(open: boolean): void
  setDefault(id: StarterId): void
}

export const useStarters = create<StarterState>((set) => ({
  open: false,
  default: savedDefault(),
  setOpen: (open) => set({ open }),
  setDefault: (id) => {
    set({ default: id })
    try {
      localStorage.setItem(KEY, id)
    } catch {
      // Kept for this session only.
    }
  },
}))

/** A new presentation from a starter deck. The engine is loaded here, the first time it's needed. */
export async function createFromStarter(id: StarterId) {
  let bytes: Uint8Array | undefined
  try {
    const { SlidesEngine } = await import('../engine/slides/engine')
    bytes = await SlidesEngine.starter(id)
  } catch (e) {
    // An empty deck is better than nothing happening; the editor reports a broken engine itself.
    console.error('Starter deck', id, e)
  }
  useWorkspace.getState().newDocument('pptx', bytes)
}

/** The starter gallery. */
export function chooseTemplate() {
  useStarters.getState().setOpen(true)
}

/** Kinds whose New has a gallery to choose from as well. */
export function hasTemplates(kind: DocKind) {
  return kind === 'pptx'
}

/** New document from the UI: a presentation starts from the default deck, anything else blank. */
export function createNew(kind: DocKind) {
  if (kind === 'pptx') void createFromStarter(useStarters.getState().default)
  else useWorkspace.getState().newDocument(kind)
}
