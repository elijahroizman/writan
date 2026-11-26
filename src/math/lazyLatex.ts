type RenderFn = (tag: string) => { bytes: Uint8Array; type: string } | null

let cached: RenderFn | null = null
let loading: Promise<void> | null = null
const waiters = new Set<() => void>()

/**
 * Resolves a `writan:latex:...` tag lazily without pulling `mathjax-full` into normal Word/Slides
 * bundles. Returns `null` on the first miss while MathJax loads, then invokes `onReady`.
 */
export function renderLatexTagLazy(tag: string, onReady: () => void): { bytes: Uint8Array; type: string } | null {
  if (cached) return cached(tag)
  waiters.add(onReady)
  if (!loading) {
    loading = import('./latexMath')
      .then((m) => {
        cached = m.renderLatexTagToSvgBytes
        const pending = [...waiters]
        waiters.clear()
        for (const cb of pending) cb()
      })
      .catch(() => {
        waiters.clear()
      })
  }
  return null
}
