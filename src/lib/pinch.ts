import { useEffect, useRef } from 'react'

/**
 * How much one ctrl+wheel event should zoom. Chromium (WebView2 on Windows, Chrome) reports a
 * trackpad pinch as small deltas where scale = e^(−deltaY/100), so following that keeps the page
 * under the fingers. A mouse wheel notch (±100 or more, or line mode) steps by 20%.
 */
export function wheelZoomFactor(e: WheelEvent): number {
  if (e.deltaMode === 0 && Math.abs(e.deltaY) < 50) return Math.exp(-e.deltaY / 100)
  return e.deltaY < 0 ? 1.2 : 1 / 1.2
}

/**
 * Windows: WebView2's pinch zoom is on so a touchpad pinch reaches the page (see
 * `configure_webview2` in src-tauri/src/lib.rs), and a pinch nobody takes would scale the whole
 * window. Documents take theirs in usePinchZoom; this stops the rest. A touchpad pinch arrives as
 * ctrl+wheel, so cancelling that cancels it; touch-action does the same for a touch screen.
 */
export function stopWindowZoom() {
  window.addEventListener('wheel', (e) => e.ctrlKey && e.preventDefault(), { passive: false })
  document.documentElement.style.touchAction = 'pan-x pan-y'
}

interface PinchOptions {
  getZoom: () => number
  setZoom: (zoom: number) => void
  /** The element to scale while the gesture is in progress (defaults to the container's first child). */
  target?: () => HTMLElement | null
  min?: number
  max?: number
  /** Apply the final zoom yourself, keeping the point under the pointer in place (instead of setZoom and the scroll fix-up). */
  commit?: (zoom: number, anchor: { x: number; y: number }) => void
}

/**
 * Trackpad pinch (Safari/WebKit gesture events) and ctrl-wheel (Chromium, mouse) zoom.
 *
 * Re-laying out a document on every gesture frame is what makes zoom feel laggy, so while
 * the fingers move we only scale the content with a CSS transform (GPU-cheap). The real
 * zoom is applied once when the gesture ends, and the scroll position is adjusted so the
 * point under the pointer stays put.
 */
export function usePinchZoom(containerRef: React.RefObject<HTMLElement | null>, options: PinchOptions) {
  const opts = useRef(options)
  opts.current = options

  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    const clamp = (z: number) => Math.min(opts.current.max ?? 4, Math.max(opts.current.min ?? 0.25, z))

    let startZoom = 1
    let factor = 1
    let el: HTMLElement | null = null
    let anchor = { x: 0, y: 0 }
    let gestureActive = false
    let wheelTimer = 0

    const scroller = () => {
      for (let node: HTMLElement | null = el?.parentElement ?? container; node; node = node.parentElement) {
        const style = getComputedStyle(node)
        if (/(auto|scroll)/.test(style.overflow + style.overflowY + style.overflowX) && (node.scrollHeight > node.clientHeight || node.scrollWidth > node.clientWidth)) return node
        if (node === container.parentElement) break
      }
      return container
    }

    const begin = (clientX: number, clientY: number) => {
      startZoom = opts.current.getZoom() || 1
      factor = 1
      el = opts.current.target?.() ?? (container.firstElementChild as HTMLElement | null)
      anchor = { x: clientX, y: clientY }
      if (el) {
        const r = el.getBoundingClientRect()
        el.style.transformOrigin = `${clientX - r.left}px ${clientY - r.top}px`
        el.style.willChange = 'transform'
      }
    }

    const preview = (f: number) => {
      factor = clamp(startZoom * f) / startZoom
      if (el) el.style.transform = `scale(${factor})`
    }

    const commit = () => {
      const next = clamp(startZoom * factor)
      const sc = scroller()
      const rect = sc.getBoundingClientRect()
      const px = sc.scrollLeft + (anchor.x - rect.left)
      const py = sc.scrollTop + (anchor.y - rect.top)
      if (el) {
        el.style.transform = ''
        el.style.transformOrigin = ''
        el.style.willChange = ''
      }
      el = null
      if (Math.abs(next - startZoom) < 0.005) return
      if (opts.current.commit) {
        opts.current.commit(next, anchor)
        return
      }
      opts.current.setZoom(next)
      const ratio = next / startZoom
      // Wait for the editor to lay out at the new zoom, then keep the anchor point in place.
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          sc.scrollLeft = px * ratio - (anchor.x - rect.left)
          sc.scrollTop = py * ratio - (anchor.y - rect.top)
        }),
      )
    }

    type GestureEvent = Event & { scale: number; clientX: number; clientY: number }
    const onGestureStart = (e: Event) => {
      e.preventDefault()
      e.stopPropagation()
      gestureActive = true
      const g = e as GestureEvent
      begin(g.clientX, g.clientY)
    }
    const onGestureChange = (e: Event) => {
      e.preventDefault()
      e.stopPropagation()
      preview((e as GestureEvent).scale)
    }
    const onGestureEnd = (e: Event) => {
      e.preventDefault()
      gestureActive = false
      commit()
    }

    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return
      // Ours alone: an editor's own ctrl-wheel zoom underneath would zoom twice.
      e.preventDefault()
      e.stopPropagation()
      if (gestureActive) return
      if (!el) begin(e.clientX, e.clientY)
      preview(factor * wheelZoomFactor(e))
      window.clearTimeout(wheelTimer)
      wheelTimer = window.setTimeout(commit, 160)
    }

    // Capture phase, so we see the gesture before any editor inside the container does.
    container.addEventListener('gesturestart', onGestureStart, true)
    container.addEventListener('gesturechange', onGestureChange, true)
    container.addEventListener('gestureend', onGestureEnd, true)
    container.addEventListener('wheel', onWheel, { passive: false, capture: true })
    return () => {
      window.clearTimeout(wheelTimer)
      container.removeEventListener('gesturestart', onGestureStart, true)
      container.removeEventListener('gesturechange', onGestureChange, true)
      container.removeEventListener('gestureend', onGestureEnd, true)
      container.removeEventListener('wheel', onWheel, { capture: true })
    }
  }, [containerRef])
}
