import { Sparkles } from 'lucide-react'
import { memo, useEffect, useRef, useState } from 'react'
import { PeerDots } from '../../collab/cursors'
import { usePlaces } from '../../collab/session'
import { isMac } from '../../lib/platform'
import { transitionLabel } from './transitions'
import type { SlideInfo, SlidesEngine } from '../../engine/slides/engine'
import { SlideImage } from '../../engine/slides/SlideImage'

/** How a click picks slides: on its own, added to or taken from the picked ones, or a run from the last pick. */
export type PickMode = 'only' | 'toggle' | 'range'

/**
 * Left sidebar: every slide, click to go, drag to reorder, right-click for more.
 * ⌘-click (Ctrl on Windows) and shift-click pick several; Delete removes the picked slides.
 */
export const SlideStrip = memo(function SlideStrip({
  engine,
  slides,
  activeIndex,
  selected,
  onSelect,
  onMove,
  onMenu,
  onDelete,
}: {
  engine: SlidesEngine
  slides: readonly SlideInfo[]
  activeIndex: number
  selected: readonly number[]
  onSelect: (index: number, mode: PickMode) => void
  onMove: (from: number, to: number) => void
  onMenu: (index: number, x: number, y: number) => void
  onDelete: () => void
}) {
  const listRef = useRef<HTMLDivElement>(null)
  const [drag, setDrag] = useState<{ from: number; target: number } | null>(null)
  const [thumbWidth, setThumbWidth] = useState(196)
  // Who's on which slide, while the presentation is shared.
  const places = usePlaces(engine)
  const peersOn = (id: string) => places.filter((p) => (p.at as { slide?: string } | null)?.slide === id).map((p) => p.peer)

  // Thumbnails fill the sidebar, which the user can resize.
  useEffect(() => {
    const list = listRef.current!
    const observer = new ResizeObserver(() => setThumbWidth(Math.max(80, Math.floor(list.clientWidth - 40))))
    observer.observe(list)
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    listRef.current?.querySelector(`[data-index="${activeIndex}"]`)?.scrollIntoView({ block: 'nearest' })
  }, [activeIndex])

  const startDrag = (e: React.PointerEvent, index: number) => {
    if (e.button !== 0) return
    listRef.current?.focus({ preventScroll: true })
    const mode: PickMode = e.shiftKey ? 'range' : (isMac ? e.metaKey : e.ctrlKey) ? 'toggle' : 'only'
    if (mode !== 'only') {
      onSelect(index, mode)
      return
    }
    const startY = e.clientY
    const el = e.currentTarget as HTMLElement
    let dragging = false
    let target = index
    const move = (ev: PointerEvent) => {
      if (!dragging && Math.abs(ev.clientY - startY) < 6) return
      if (!dragging) el.setPointerCapture(e.pointerId)
      dragging = true
      const items = Array.from(listRef.current!.querySelectorAll<HTMLElement>('.slide-item'))
      target = items.length
      for (let i = 0; i < items.length; i++) {
        const r = items[i].getBoundingClientRect()
        if (ev.clientY < r.top + r.height / 2) {
          target = i
          break
        }
      }
      setDrag({ from: index, target })
      const scroller = listRef.current!.closest('.sidebar-scroll')
      if (scroller) {
        const r = scroller.getBoundingClientRect()
        if (ev.clientY < r.top + 30) scroller.scrollTop -= 12
        else if (ev.clientY > r.bottom - 30) scroller.scrollTop += 12
      }
    }
    const up = () => {
      el.removeEventListener('pointermove', move)
      el.removeEventListener('pointerup', up)
      el.removeEventListener('pointercancel', up)
      setDrag(null)
      if (!dragging) {
        onSelect(index, 'only')
        return
      }
      const to = target > index ? target - 1 : target
      if (to !== index) onMove(index, to)
    }
    el.addEventListener('pointermove', move)
    el.addEventListener('pointerup', up)
    el.addEventListener('pointercancel', up)
  }

  return (
    <div
      className="slide-list"
      ref={listRef}
      role="listbox"
      aria-label="Slides"
      aria-multiselectable
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Delete' || e.key === 'Backspace') onDelete()
        else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          const next = activeIndex + (e.key === 'ArrowDown' ? 1 : -1)
          if (next >= 0 && next < slides.length) onSelect(next, e.shiftKey ? 'range' : 'only')
        } else return
        // The slide editor listens on the window; these keys are the sidebar's.
        e.preventDefault()
        e.stopPropagation()
      }}
    >
      {slides.map((slide, i) => (
        <div
          key={slide.id}
          data-index={i}
          role="option"
          aria-selected={selected.includes(i)}
          className={`slide-item ${i === activeIndex ? 'active' : ''} ${selected.length > 1 && selected.includes(i) ? 'picked' : ''} ${slide.hidden ? 'hidden-slide' : ''} ${drag?.target === i ? 'drop-before' : ''} ${
            drag && drag.target === slides.length && i === slides.length - 1 ? 'drop-after' : ''
          } ${drag?.from === i ? 'dragging' : ''}`}
          onPointerDown={(e) => startDrag(e, i)}
          onContextMenu={(e) => {
            e.preventDefault()
            // Right-clicking one of several picked slides acts on all of them.
            if (!selected.includes(i)) onSelect(i, 'only')
            onMenu(i, e.clientX, e.clientY)
          }}
        >
          <span className="slide-number">
            {i + 1}
            {transitionLabel(slide.transition) && (
              <span className="slide-transition-mark" title={`Transition: ${transitionLabel(slide.transition)}`}>
                <Sparkles size={10} />
              </span>
            )}
            <PeerDots peers={peersOn(slide.id)} />
          </span>
          <div className="slide-thumb">
            <SlideImage engine={engine} index={i} revision={slide.revision} width={thumbWidth} />
          </div>
        </div>
      ))}
    </div>
  )
})
