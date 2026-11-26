import { ChevronDown, ChevronRight } from 'lucide-react'
import { memo, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { Priority } from './engine/client'
import type { OutlineItem, PageInfo } from './engine/protocol'
import type { DocStore } from './engine/store'
import { THUMB_WIDTH } from './engine/store'

/** Space around each thumbnail: padding, the page number, and the gap to the next one. */
const ITEM_EXTRA = 8 + 6 + 14 + 6
const ITEM_GAP = 4
const LIST_PAD = 8
const OVERSCAN = 600

function useStoreVersion(store: DocStore) {
  const version = useRef(0)
  return useSyncExternalStore(
    (fn) =>
      store.subscribe(() => {
        version.current++
        fn()
      }),
    () => version.current,
  )
}

/**
 * Page thumbnails. Only the ones near the visible part of the sidebar exist, and only those are
 * drawn (the rest are cancelled when you scroll past), so a long document scrolls straight to the
 * spot you want. Drag to reorder.
 */
export function Thumbnails({
  store,
  pages,
  current,
  onSelect,
  onMenu,
  onMove,
}: {
  store: DocStore
  pages: PageInfo[]
  current: number
  onSelect: (index: number) => void
  onMenu: (index: number, x: number, y: number) => void
  onMove: (from: number, to: number) => void
}) {
  const listRef = useRef<HTMLDivElement>(null)
  const [view, setView] = useState({ top: 0, height: 0 })
  const [drag, setDrag] = useState<{ from: number; to: number } | null>(null)
  useStoreVersion(store)

  const { offsets, heights, total } = useMemo(() => {
    const offsets = new Float64Array(pages.length)
    const heights = new Float64Array(pages.length)
    let y = LIST_PAD
    pages.forEach((p, i) => {
      offsets[i] = y
      heights[i] = Math.round((THUMB_WIDTH * p.height) / p.width) + ITEM_EXTRA
      y += heights[i] + ITEM_GAP
    })
    return { offsets, heights, total: y + 16 }
  }, [pages])

  const scroller = () => listRef.current?.closest<HTMLElement>('.sidebar-scroll') ?? null

  useEffect(() => {
    const sc = scroller()
    const list = listRef.current
    if (!sc || !list) return
    let frame = 0
    const measure = () => {
      frame = 0
      setView((v) => {
        const top = sc.scrollTop - list.offsetTop
        return v.top === top && v.height === sc.clientHeight ? v : { top, height: sc.clientHeight }
      })
    }
    const onScroll = () => {
      frame ||= requestAnimationFrame(measure)
    }
    sc.addEventListener('scroll', onScroll, { passive: true })
    const resize = new ResizeObserver(onScroll)
    resize.observe(sc)
    measure()
    return () => {
      cancelAnimationFrame(frame)
      sc.removeEventListener('scroll', onScroll)
      resize.disconnect()
    }
  }, [])

  // Keep the current page's thumbnail in view while reading.
  useEffect(() => {
    const sc = scroller()
    const list = listRef.current
    if (!sc || !list || current >= pages.length || drag) return
    const top = list.offsetTop + offsets[current]
    const bottom = top + heights[current]
    if (top < sc.scrollTop) sc.scrollTop = top - 8
    else if (bottom > sc.scrollTop + sc.clientHeight) sc.scrollTop = bottom - sc.clientHeight + 8
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [current])

  const indexAt = (y: number) => {
    let lo = 0
    let hi = pages.length - 1
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1
      if (offsets[mid] <= y) lo = mid
      else hi = mid - 1
    }
    return lo
  }

  const first = pages.length ? indexAt(Math.max(0, view.top - OVERSCAN)) : 0
  const last = pages.length ? indexAt(view.top + view.height + OVERSCAN) : -1
  const middle = indexAt(view.top + view.height / 2)

  const startDrag = (e: React.PointerEvent, index: number) => {
    if (e.button !== 0) return
    const startY = e.clientY
    const el = e.currentTarget as HTMLElement
    const sc = scroller()
    let dragging = false
    let target = index
    let auto = 0
    let lastY = startY
    el.setPointerCapture(e.pointerId)
    const place = () => {
      const list = listRef.current!
      const y = lastY - list.getBoundingClientRect().top
      const i = indexAt(y)
      target = y > offsets[i] + heights[i] / 2 ? i + 1 : i
      setDrag({ from: index, to: target })
    }
    const scrollStep = () => {
      if (!sc) return
      const r = sc.getBoundingClientRect()
      const step = lastY < r.top + 40 ? -14 : lastY > r.bottom - 40 ? 14 : 0
      if (step) {
        sc.scrollTop += step
        place()
      }
      auto = requestAnimationFrame(scrollStep)
    }
    const move = (ev: PointerEvent) => {
      lastY = ev.clientY
      if (!dragging && Math.abs(ev.clientY - startY) < 6) return
      if (!dragging) {
        dragging = true
        auto = requestAnimationFrame(scrollStep)
      }
      place()
    }
    const up = () => {
      cancelAnimationFrame(auto)
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', up)
      setDrag(null)
      if (dragging) {
        const to = target > index ? target - 1 : target
        if (to !== index) onMove(index, to)
      }
    }
    // On window: auto-scroll can take the dragged thumbnail out of the list, and its own
    // listeners would go with it.
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', up)
  }

  const items = []
  for (let i = first; i <= last; i++) {
    const page = pages[i]
    const dropBefore = drag && drag.to === i
    const dropAfter = drag && drag.to === pages.length && i === pages.length - 1
    items.push(
      <div
        key={page.key}
        className={`thumb ${current === i ? 'active' : ''} ${dropBefore ? 'drop-before' : ''} ${dropAfter ? 'drop-after' : ''}`}
        style={{ top: offsets[i], height: heights[i] }}
        data-page={i + 1}
        role="button"
        tabIndex={0}
        aria-label={`Page ${i + 1}`}
        onClick={() => onSelect(i)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') onSelect(i)
        }}
        onContextMenu={(e) => {
          e.preventDefault()
          onSelect(i)
          onMenu(i, e.clientX, e.clientY)
        }}
        onPointerDown={(e) => startDrag(e, i)}
      >
        <Thumb store={store} index={i} page={page} priority={Priority.thumbnails + Math.abs(i - middle) / 100} />
        <span className="thumb-label">{i + 1}</span>
      </div>,
    )
  }

  return (
    <div className="thumbs" ref={listRef} style={{ height: total }}>
      {items}
    </div>
  )
}

const Thumb = memo(function Thumb({ store, index, page, priority }: { store: DocStore; index: number; page: PageInfo; priority: number }) {
  const indexRef = useRef(index)
  indexRef.current = index
  const priorityRef = useRef(priority)
  priorityRef.current = priority
  const rev = useSyncExternalStore(
    (cb) => store.subscribeKey(page.key, cb),
    () => store.rev(page.key),
  )
  const thumb = useSyncExternalStore(
    (cb) => store.subscribeThumb(page.key, cb),
    () => store.thumb(indexRef.current),
  )

  useEffect(() => {
    const job = new AbortController()
    void store.loadThumb(indexRef.current, job.signal, priorityRef.current)
    return () => job.abort()
  }, [store, page.key, page.rotation, rev])

  const height = (THUMB_WIDTH * page.height) / page.width
  // An older thumbnail (from before a rotation) stands in, turned, until the new one is drawn.
  const turn = thumb ? (((page.rotation - thumb.rotation) % 4) + 4) % 4 : 0
  const style: React.CSSProperties | undefined =
    turn % 2 === 1
      ? { position: 'absolute', left: '50%', top: '50%', width: height, height: THUMB_WIDTH, transform: `translate(-50%, -50%) rotate(${turn * 90}deg)` }
      : turn === 2
        ? { transform: 'rotate(180deg)' }
        : undefined
  return (
    <div className="thumb-page" style={{ height }} data-ready={thumb && !turn && thumb.rev === rev ? '' : undefined}>
      {thumb && <img src={thumb.url} alt="" draggable={false} style={style} decoding="async" />}
    </div>
  )
})

export function Outline({ store, onNavigate }: { store: DocStore; onNavigate: (item: OutlineItem) => void }) {
  const [items, setItems] = useState<OutlineItem[] | null>(null)
  useEffect(() => {
    let live = true
    store.engine
      .call('outline', [], { priority: Priority.text })
      .then((o) => live && setItems(o))
      .catch(() => live && setItems([]))
    return () => {
      live = false
    }
  }, [store])

  if (items === null) return null
  if (items.length === 0) return <div className="empty-hint">This PDF doesn’t have an outline.</div>
  return (
    <nav className="list" aria-label="Outline">
      {items.map((item, i) => (
        <OutlineRow key={i} item={item} depth={0} onNavigate={onNavigate} />
      ))}
    </nav>
  )
}

function OutlineRow({ item, depth, onNavigate }: { item: OutlineItem; depth: number; onNavigate: (item: OutlineItem) => void }) {
  const [open, setOpen] = useState(depth === 0 && item.children.length < 12)
  const hasChildren = item.children.length > 0
  return (
    <>
      <div className="list-item outline-item" style={{ paddingLeft: 4 + depth * 14 }} onClick={() => onNavigate(item)}>
        <span
          className="outline-twisty"
          onClick={(e) => {
            e.stopPropagation()
            setOpen(!open)
          }}
          style={{ visibility: hasChildren ? 'visible' : 'hidden' }}
        >
          {open ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
        </span>
        <span className="label">{item.title}</span>
      </div>
      {open && item.children.map((child, i) => <OutlineRow key={i} item={child} depth={depth + 1} onNavigate={onNavigate} />)}
    </>
  )
}
