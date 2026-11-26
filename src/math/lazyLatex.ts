import type { PaintItem } from '../engine/paint'
import { LATEX_MATH_PREFIX } from './latexTag'
import type { renderLatexTagToSvgBytes as renderTag } from './latexMath'

type MathModule = typeof import('./latexMath')

let loaded: MathModule | null = null
let loading: Promise<MathModule> | null = null

/** Loads MathJax the first time a document or export actually contains editable LaTeX. */
export function loadLatexMath(): Promise<MathModule> {
  loading ??= import('./latexMath').then((m) => {
    loaded = m
    return m
  })
  return loading
}

/**
 * Renders a tag when MathJax is already loaded; otherwise starts loading it and answers null.
 * `ready` runs once the renderer is available, so the caller can invalidate the affected view.
 */
export function renderLazyLatexTag(tag: string, ready: () => void): ReturnType<typeof renderTag> {
  if (loaded) return loaded.renderLatexTagToSvgBytes(tag)
  void loadLatexMath().then(ready)
  return null
}

/** Whether a display list contains editable LaTeX, including inside groups. */
export function itemsContainLatex(items: readonly PaintItem[]): boolean {
  return items.some((item) => (item.t === 'image' && item.media.startsWith(LATEX_MATH_PREFIX)) || (item.t === 'group' && itemsContainLatex(item.items)))
}
