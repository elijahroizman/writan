import type { ReactNode } from 'react'
import { forwardRef, memo, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { wheelZoomFactor } from '../../../lib/pinch'
import { Priority } from '../engine/client'
import type { Box, PageInfo } from '../engine/protocol'
import type { DocStore } from '../engine/store'
import type { Layout, ZoomMode } from './layout'
import { clampZoom, computeLayout, fitZoom, mostVisible, pageAt, pageLeft, pagesIn, stepZoom } from './layout'
import { clearTextSelection } from './layers'
import { PageRenderer } from './PageRenderer'

export interface ViewerHandle {
  /** Scroll to a page; `y` is points from the page's top. */
  goTo(index: number, y?: number): void
  /** Scroll just enough to show `box` (points) on page `index`. */
  reveal(index: number, box: Box): void
  setZoom(zoom: number | ZoomMode): void
  step(dir: 1 | -1): void
  zoomBy(factor: number): void
  /** The page and spot (points) under a point on screen, if any. */
  hitTest(clientX: number, clientY: number): { index: number; x: number; y: number } | null
  /** Hold a page in place, and keep it the current page, across an edit that reshapes it. */
  hold(index: number): void
}

export interface PageSlot {
  index: number
  page: PageInfo
  /** CSS pixels per point. */
  unit: number
}

interface Props {
  store: DocStore
  pages: PageInfo[]
  /** False while the tab is in the background and its drawings have been let go. */
  awake: boolean
  className?: string
  onPageChange(index: number): void
  onZoomChange(zoom: number): void
  /** Layers drawn over each page (text, links, annotations, tools). */
  renderPage(slot: PageSlot): ReactNode
  /**
   * Remembers where the view was under this key, and goes back there when a viewer with the
   * same key mounts again (a PDF reloaded in place, like a LaTeX build opened in its own tab).
   */
  positionKey?: string
}

type Anchor = { index: number; u: number; v: number; px: number; py: number }

/** The page at the top of the view, how far down it, and the zoom, by `positionKey`. */
const positions = new Map<string, { index: number; v: number; zoom: number }>()

/** Scrolling, virtualized list of pages with pinch zoom. Only pages near the window exist in the DOM. */
export const Viewer = forwardRef<ViewerHandle, Props>(function Viewer({ store, pages, awake, className, onPageChange, onZoomChange, renderPage, positionKey }, ref) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  const [viewport, setViewport] = useState({ width: 0, height: 0 })
  /** A fixed zoom, or a fit mode measured against one page (the current one when it was chosen). */
  const [zoomState, setZoomState] = useState<{ zoom: number; mode: ZoomMode | null; fitIndex: number }>({ zoom: 1, mode: 'auto', fitIndex: 0 })
  const [range, setRange] = useState<[number, number]>([0, -1])
  const [current, setCurrent] = useState(0)
  const fitPage = pages[Math.min(zoomState.fitIndex, pages.length - 1)]
  const zoom = zoomState.mode && viewport.width ? fitZoom(zoomState.mode, fitPage, viewport) : zoomState.zoom
  const layout = useMemo(() => computeLayout(pages, zoom, viewport.width), [pages, zoom, viewport.width])
  const layoutRef = useRef(layout)
  layoutRef.current = layout
  /** Where to put the view after the next layout (zoom origin, or the spot at the top before an edit). */
  const pendingAnchor = useRef<Anchor | null>(null)
  const topAnchor = useRef<Anchor | null>(null)
  const listeners = useRef(new Set<() => void>())
  const currentRef = useRef(0)
  /** A page the user just edited stays current until they scroll or click. */
  const held = useRef(false)
  /** A position from an earlier viewer to go back to, once there is a layout to put it in. */
  const restore = useRef(positionKey ? positions.get(positionKey) : undefined)

  useEffect(() => onZoomChange(zoom), [zoom, onZoomChange])

  // ── Track what's on screen.
  const update = useCallback(() => {
    const el = scrollRef.current
    const l = layoutRef.current
    if (!el || !l.tops.length) return
    const top = el.scrollTop
    const bottom = top + el.clientHeight
    const overscan = el.clientHeight
    const [first, last] = pagesIn(l, Math.max(0, top - overscan), bottom + overscan)
    setRange((r) => (r[0] === first && r[1] === last ? r : [first, last]))
    const now = held.current ? currentRef.current : mostVisible(l, top, bottom)
    if (now !== currentRef.current) {
      currentRef.current = now
      setCurrent(now)
      onPageChange(now)
    }
    const i = pageAt(l, top + 1)
    topAnchor.current = { index: i, u: 0.5, v: (top - l.tops[i]) / l.heights[i], px: el.clientWidth / 2, py: 0 }
    // Until the old position is back, scrolling hasn't started from it, so don't overwrite it.
    if (positionKey && !restore.current) positions.set(positionKey, { index: i, v: topAnchor.current.v, zoom: l.zoom })
    for (const fn of listeners.current) fn()
  }, [onPageChange, positionKey])

  useEffect(() => {
    const el = scrollRef.current!
    let frame = 0
    const onScroll = () => {
      frame ||= requestAnimationFrame(() => {
        frame = 0
        update()
      })
    }
    el.addEventListener('scroll', onScroll, { passive: true })
    const release = () => {
      held.current = false
    }
    // WebView2 and Chromium keep the selection when the click lands between pages.
    const onDown = (e: PointerEvent) => {
      if (e.button === 0 && !(e.target as Element).closest('.page')) clearTextSelection(el)
    }
    el.addEventListener('pointerdown', onDown)
    for (const type of ['wheel', 'pointerdown', 'keydown', 'touchstart']) el.addEventListener(type, release, { passive: true })
    const resize = new ResizeObserver(() => {
      if (!el.clientWidth) return // hidden
      pendingAnchor.current ??= topAnchor.current
      setViewport((v) => (v.width === el.clientWidth && v.height === el.clientHeight ? v : { width: el.clientWidth, height: el.clientHeight }))
    })
    resize.observe(el)
    return () => {
      cancelAnimationFrame(frame)
      el.removeEventListener('scroll', onScroll)
      el.removeEventListener('pointerdown', onDown)
      for (const type of ['wheel', 'pointerdown', 'keydown', 'touchstart']) el.removeEventListener(type, release)
      resize.disconnect()
    }
  }, [update])

  // After a layout change, put the anchored spot back where it was, then refresh what's visible.
  useLayoutEffect(() => {
    const el = scrollRef.current!
    const content = contentRef.current!
    content.style.transform = ''
    content.style.transformOrigin = ''
    let a = pendingAnchor.current
    pendingAnchor.current = null
    const back = restore.current
    if (back && layout.tops.length && viewport.width) {
      restore.current = undefined
      const i = Math.min(back.index, layout.tops.length - 1)
      const anchor = { index: i, u: 0.5, v: back.v, px: el.clientWidth / 2, py: 0 }
      if (Math.abs(back.zoom - layout.zoom) < 1e-6) a = anchor
      else {
        // The zoom comes first; the next layout puts the spot back.
        pendingAnchor.current = anchor
        setZoomState({ zoom: clampZoom(back.zoom), mode: null, fitIndex: i })
      }
    }
    if (a && layout.tops.length) {
      const i = Math.min(a.index, layout.tops.length - 1)
      el.scrollLeft = pageLeft(layout, i) + a.u * layout.widths[i] - a.px
      el.scrollTop = layout.tops[i] + a.v * layout.heights[i] - a.py
    }
    update()
  }, [layout, update])

  // A zoom that left the layout as it was (already at 800%, the same fit) has nothing to restore.
  // Runs after the effect above, so a real change has used its anchor by now.
  useLayoutEffect(() => {
    pendingAnchor.current = null
  }, [zoomState])

  // When pages change shape (rotate, delete…), keep the page at the top of the window steady.
  const prevPages = useRef(pages)
  if (prevPages.current !== pages) {
    prevPages.current = pages
    pendingAnchor.current ??= topAnchor.current
  }

  // ── Zoom
  const anchorAt = useCallback((clientX: number, clientY: number): Anchor | null => {
    const el = scrollRef.current
    const l = layoutRef.current
    if (!el || !l.tops.length) return null
    const r = el.getBoundingClientRect()
    const px = clientX - r.left
    const py = clientY - r.top
    const y = el.scrollTop + py
    const i = pageAt(l, y)
    return { index: i, u: (el.scrollLeft + px - pageLeft(l, i)) / l.widths[i], v: (y - l.tops[i]) / l.heights[i], px, py }
  }, [])

  const zoomTo = useCallback(
    (zoom: number, mode: ZoomMode | null, clientX?: number, clientY?: number) => {
      const el = scrollRef.current
      if (!el) return
      const r = el.getBoundingClientRect()
      pendingAnchor.current = anchorAt(clientX ?? r.left + r.width / 2, clientY ?? r.top + r.height / 2)
      setZoomState({ zoom: clampZoom(zoom), mode, fitIndex: currentRef.current })
    },
    [anchorAt],
  )

  // Pinch (gesture events in WebKit, ctrl+wheel in Chromium) scales the drawn pages with a CSS
  // transform while the fingers move. The real zoom, which re-lays out and redraws, happens once
  // when they stop.
  useEffect(() => {
    const el = scrollRef.current!
    const content = contentRef.current!
    let gesture: { start: number; factor: number; x: number; y: number } | null = null
    let gestureActive = false
    let wheelTimer = 0
    const begin = (clientX: number, clientY: number) => {
      const r = el.getBoundingClientRect()
      gesture = { start: layoutRef.current.zoom, factor: 1, x: clientX, y: clientY }
      content.style.transformOrigin = `${el.scrollLeft + clientX - r.left}px ${el.scrollTop + clientY - r.top}px`
    }
    const preview = (factor: number) => {
      if (!gesture) return
      gesture.factor = clampZoom(gesture.start * factor) / gesture.start
      content.style.transform = `scale(${gesture.factor})`
    }
    const commit = () => {
      if (!gesture) return
      const g = gesture
      gesture = null
      if (Math.abs(g.factor - 1) < 0.005) {
        content.style.transform = ''
        return
      }
      zoomTo(g.start * g.factor, null, g.x, g.y)
    }
    type GestureEvent = Event & { scale: number; clientX: number; clientY: number }
    const onGestureStart = (e: Event) => {
      e.preventDefault()
      gestureActive = true
      const g = e as GestureEvent
      begin(g.clientX, g.clientY)
    }
    const onGestureChange = (e: Event) => {
      e.preventDefault()
      preview((e as GestureEvent).scale)
    }
    const onGestureEnd = (e: Event) => {
      e.preventDefault()
      gestureActive = false
      commit()
    }
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return
      e.preventDefault()
      if (gestureActive) return
      if (!gesture) begin(e.clientX, e.clientY)
      preview(gesture!.factor * wheelZoomFactor(e))
      window.clearTimeout(wheelTimer)
      wheelTimer = window.setTimeout(commit, 150)
    }
    el.addEventListener('gesturestart', onGestureStart)
    el.addEventListener('gesturechange', onGestureChange)
    el.addEventListener('gestureend', onGestureEnd)
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => {
      window.clearTimeout(wheelTimer)
      el.removeEventListener('gesturestart', onGestureStart)
      el.removeEventListener('gesturechange', onGestureChange)
      el.removeEventListener('gestureend', onGestureEnd)
      el.removeEventListener('wheel', onWheel)
    }
  }, [zoomTo])

  useImperativeHandle(
    ref,
    () => ({
      goTo(index, y) {
        const el = scrollRef.current
        const l = layoutRef.current
        if (!el || index < 0 || index >= l.tops.length) return
        el.scrollTop = l.tops[index] + (y !== undefined ? y * l.unit - 8 : -8)
      },
      reveal(index, box) {
        const el = scrollRef.current
        const l = layoutRef.current
        if (!el || index >= l.tops.length) return
        const top = l.tops[index] + box.y * l.unit
        const bottom = top + box.h * l.unit
        if (top < el.scrollTop + 40 || bottom > el.scrollTop + el.clientHeight - 100) el.scrollTop = top - el.clientHeight / 3
        const left = pageLeft(l, index) + box.x * l.unit
        if (left < el.scrollLeft || left + box.w * l.unit > el.scrollLeft + el.clientWidth) el.scrollLeft = left - el.clientWidth / 3
      },
      setZoom(zoom) {
        if (typeof zoom === 'number') zoomTo(zoom, null)
        else {
          zoomTo(layoutRef.current.zoom, zoom)
          // Fitting a whole page also brings its top into view.
          if (zoom === 'page-fit') pendingAnchor.current = { index: currentRef.current, u: 0.5, v: 0, px: viewport.width / 2, py: 16 }
        }
      },
      step(dir) {
        zoomTo(stepZoom(layoutRef.current.zoom, dir), null)
      },
      zoomBy(factor) {
        zoomTo(layoutRef.current.zoom * factor, null)
      },
      hold(index) {
        const el = scrollRef.current
        const l = layoutRef.current
        if (!el || index >= l.tops.length) return
        held.current = true
        currentRef.current = index
        setCurrent(index)
        onPageChange(index)
        pendingAnchor.current = { index, u: 0.5, v: 0, px: el.clientWidth / 2, py: Math.max(8, l.tops[index] - el.scrollTop) }
      },
      hitTest(clientX, clientY) {
        const el = scrollRef.current
        const l = layoutRef.current
        if (!el || !l.tops.length) return null
        const r = el.getBoundingClientRect()
        const y = el.scrollTop + clientY - r.top
        const i = pageAt(l, y)
        const x = el.scrollLeft + clientX - r.left - pageLeft(l, i)
        const py = y - l.tops[i]
        if (x < 0 || py < 0 || x > l.widths[i] || py > l.heights[i]) return null
        return { index: i, x: x / l.unit, y: py / l.unit }
      },
    }),
    [zoomTo, viewport, onPageChange],
  )

  const slots: number[] = []
  if (awake && viewport.width) for (let i = range[0]; i <= range[1] && i < pages.length; i++) slots.push(i)

  return (
    <div ref={scrollRef} className={`pdf-container ${className ?? ''}`} tabIndex={-1}>
      <div ref={contentRef} className="pdf-pages" style={{ width: layout.width, height: layout.height }}>
        {slots.map((i) => (
          <PageView
            key={pages[i].key}
            store={store}
            index={i}
            page={pages[i]}
            rev={store.rev(pages[i].key)}
            layout={layout}
            scrollRef={scrollRef}
            listeners={listeners.current}
            priority={pagePriority(i, current, layout, scrollRef.current)}
          >
            {renderPage({ index: i, page: pages[i], unit: layout.unit })}
          </PageView>
        ))}
      </div>
    </div>
  )
})

function pagePriority(i: number, current: number, l: Layout, el: HTMLElement | null) {
  if (!el) return Priority.visible + Math.abs(i - current)
  const onScreen = l.tops[i] < el.scrollTop + el.clientHeight && l.tops[i] + l.heights[i] > el.scrollTop
  return (onScreen ? Priority.visible : Priority.nearby) + Math.min(9, Math.abs(i - current))
}

const PageView = memo(function PageView({
  store,
  index,
  page,
  rev,
  layout,
  scrollRef,
  listeners,
  priority,
  children,
}: {
  store: DocStore
  index: number
  page: PageInfo
  rev: number
  layout: Layout
  scrollRef: React.RefObject<HTMLDivElement | null>
  listeners: Set<() => void>
  priority: number
  children: ReactNode
}) {
  const pageRef = useRef<HTMLDivElement>(null)
  const hostRef = useRef<HTMLDivElement>(null)
  const renderer = useRef<PageRenderer | null>(null)
  const top = layout.tops[index]
  const left = pageLeft(layout, index)
  const width = layout.widths[index]
  const height = layout.heights[index]
  const geometry = useRef({ top, left })
  geometry.current = { top, left }

  // A selection that ends on this page would snap to the page list once the page is gone, and
  // swallow whatever pages are put back before it.
  useLayoutEffect(() => {
    const el = pageRef.current!
    return () => clearTextSelection(el)
  }, [])

  useEffect(() => {
    const r = new PageRenderer(hostRef.current!, pageRef.current!, store)
    renderer.current = r
    const onViewport = () => {
      const el = scrollRef.current
      if (!el) return
      const g = geometry.current
      r.setVisibleRect({ x: el.scrollLeft - g.left, y: el.scrollTop - g.top, w: el.clientWidth, h: el.clientHeight })
    }
    listeners.add(onViewport)
    onViewport()
    return () => {
      listeners.delete(onViewport)
      r.destroy()
      renderer.current = null
    }
  }, [store, scrollRef, listeners])

  useEffect(() => {
    renderer.current!.update({ page, rev, unit: layout.unit, priority })
    const el = scrollRef.current
    if (el) renderer.current!.setVisibleRect({ x: el.scrollLeft - left, y: el.scrollTop - top, w: el.clientWidth, h: el.clientHeight })
  }, [page, rev, layout.unit, priority, scrollRef, left, top])

  return (
    <div
      ref={pageRef}
      className="page"
      data-page-number={index + 1}
      style={{ top, left, width, height, ['--u' as string]: layout.unit }}
    >
      <div ref={hostRef} className="page-canvas" />
      {children}
    </div>
  )
})
