// The slide being edited: drawn from the engine, with the selected frames' boxes and handles
// over it. Click picks a frame (shift or ⌘ adds), dragging moves it, handles resize and rotate
// it, dragging on empty slide picks what's inside, and a double-click (or typing on a picked
// text frame) edits its text, with the caret and selection drawn inside the frame.

import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { MenuEntry } from '../../components/ui'
import { PopoverMenu } from '../../components/ui'
import { isMac } from '../../lib/platform'
import { spellingMenu } from '../../lib/spelling'
import type { Box, SlidesEngine } from './engine'
import type { SlideSpelling } from './spelling'
import { slideWordAt, watchSlideSpelling } from './spelling'
import './slides.css'

export const SLIDES_CLIPBOARD = 'application/x-workspace-slides'

const HANDLES = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'] as const
type HandleName = (typeof HANDLES)[number]

interface Props {
  engine: SlidesEngine
  /** Zoom relative to fitting the slide in the view (1 fits). */
  scale: number
  /** Space around the slide when it fits. */
  padding?: number
  /** Whether keyboard and pointer go to the slide (the tab is active). */
  active: boolean
  onContextMenu?: (x: number, y: number) => void
  /** The scrolling view and the slide's wrapper, for zooming around a point and pinching. */
  viewportRef?: React.MutableRefObject<HTMLDivElement | null>
  wrapperRef?: React.MutableRefObject<HTMLDivElement | null>
  /** The scale in pixels per slide pixel, reported as it changes. */
  onPixelScale?: (s: number) => void
}

type Drag =
  | { kind: 'move'; ids: string[]; x0: number; y0: number; dx: number; dy: number; moved: boolean }
  | { kind: 'resize'; box: Box; handle: HandleName; x0: number; y0: number; next: Box; keep: boolean }
  | { kind: 'rotate'; box: Box; rot: number }
  | { kind: 'marquee'; x0: number; y0: number; x1: number; y1: number; additive: boolean }
  | { kind: 'text'; frame: string }

export function SlideCanvas({ engine, scale, padding = 40, active, onContextMenu, viewportRef, wrapperRef, onPixelScale }: Props) {
  const viewport = useRef<HTMLDivElement | null>(null)
  const slideRef = useRef<HTMLDivElement | null>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const [view, setView] = useState({ w: 800, h: 600 })
  const [, setTick] = useState(0)
  const [drag, setDrag] = useState<Drag | null>(null)
  const dragRef = useRef<Drag | null>(null)
  dragRef.current = drag
  const composing = useRef(false)
  const [spelling, setSpelling] = useState<SlideSpelling | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null)

  useEffect(() => engine.subscribe(() => setTick((t) => t + 1)), [engine])
  // Misspelled words get a wavy underline; right-clicking one offers corrections.
  useEffect(() => watchSlideSpelling(engine, setSpelling), [engine])

  useLayoutEffect(() => {
    const el = viewport.current
    if (!el) return
    const measure = () => setView({ w: el.clientWidth, h: el.clientHeight })
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  const fit = Math.max(0.05, Math.min((view.w - padding * 2) / engine.width, (view.h - padding * 2) / engine.height))
  const px = fit * scale
  const sw = engine.width * px
  const sh = engine.height * px
  const stageW = Math.max(view.w, sw + padding * 2)
  const stageH = Math.max(view.h, sh + padding * 2)
  const left = (stageW - sw) / 2
  const top = (stageH - sh) / 2
  useEffect(() => {
    onPixelScale?.(px)
  }, [px, onPixelScale])

  useEffect(() => {
    if (active) inputRef.current?.focus({ preventScroll: true })
  }, [active, engine])

  const st = engine.state
  const html = engine.html(st.active)

  /** A client point in slide pixels. */
  const toSlide = (cx: number, cy: number) => {
    const r = slideRef.current!.getBoundingClientRect()
    return { x: (cx - r.left) / px, y: (cy - r.top) / px }
  }

  // Live preview of a move: the moving frames' drawings shift with the pointer.
  useLayoutEffect(() => {
    const surface = slideRef.current?.querySelector('.slide-surface')
    if (!surface) return
    const els = Array.from(surface.querySelectorAll<HTMLElement>(':scope > .gr[data-id]'))
    for (const el of els) {
      const moving = drag?.kind === 'move' && drag.moved && drag.ids.includes(el.dataset.id!)
      el.style.translate = moving ? `${drag.dx}px ${drag.dy}px` : ''
    }
  })

  const run = <T,>(cmd: Record<string, unknown> & { cmd: string }) => engine.run<T>(cmd)

  // ── Pointer
  const onMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return
    e.preventDefault()
    inputRef.current?.focus({ preventScroll: true })
    const p = toSlide(e.clientX, e.clientY)
    const target = e.target as HTMLElement
    const handle = target.closest<HTMLElement>('[data-handle]')?.dataset.handle as HandleName | 'rot' | undefined
    if (handle && st.boxes.length === 1) {
      const box = st.boxes[0]
      if (handle === 'rot') setDrag({ kind: 'rotate', box, rot: box.rot })
      else setDrag({ kind: 'resize', box, handle, x0: p.x, y0: p.y, next: box, keep: false })
      return
    }
    const additive = e.shiftKey || (isMac ? e.metaKey : e.ctrlKey)
    if (st.editing) {
      const inEditing = engine.doc.frameAt(p.x, p.y) === st.editing
      if (inEditing) {
        run({ cmd: 'textClick', x: p.x, y: p.y, extend: e.shiftKey, count: e.detail })
        setDrag({ kind: 'text', frame: st.editing })
        return
      }
    }
    const hit = run<string | null>({ cmd: 'press', x: p.x, y: p.y, additive })
    if (hit && e.detail >= 2) {
      const el = engine.elements.find((x) => x.id === hit)
      if (el && (el.type === 'text' || el.type === 'shape' || el.type === 'table')) {
        run({ cmd: 'beginEdit', id: Number(hit), x: p.x, y: p.y })
        return
      }
    }
    if (hit && !additive) setDrag({ kind: 'move', ids: engine.state.frames, x0: p.x, y0: p.y, dx: 0, dy: 0, moved: false })
    else if (!hit) setDrag({ kind: 'marquee', x0: p.x, y0: p.y, x1: p.x, y1: p.y, additive })
  }

  useEffect(() => {
    if (!drag) return
    const move = (e: MouseEvent) => {
      const d = dragRef.current
      if (!d) return
      const p = toSlide(e.clientX, e.clientY)
      switch (d.kind) {
        case 'move': {
          const dx = p.x - d.x0
          const dy = p.y - d.y0
          setDrag({ ...d, dx, dy, moved: d.moved || Math.hypot(dx, dy) * px > 3 })
          break
        }
        case 'resize':
          setDrag({ ...d, next: resized(d.box, d.handle, p.x - d.x0, p.y - d.y0, e.shiftKey || isPicture(engine, d.box.id)) })
          break
        case 'rotate': {
          const cx = d.box.x + d.box.w / 2
          const cy = d.box.y + d.box.h / 2
          let rot = (Math.atan2(p.y - cy, p.x - cx) * 180) / Math.PI + 90
          if (e.shiftKey) rot = Math.round(rot / 15) * 15
          setDrag({ ...d, rot: ((rot % 360) + 360) % 360 })
          break
        }
        case 'marquee':
          setDrag({ ...d, x1: p.x, y1: p.y })
          break
        case 'text':
          run({ cmd: 'textClick', x: p.x, y: p.y, extend: true, count: 1 })
          break
      }
    }
    const up = () => {
      const d = dragRef.current
      setDrag(null)
      if (!d) return
      if (d.kind === 'move' && d.moved) run({ cmd: 'moveFrames', ids: d.ids.map(Number), dx: d.dx, dy: d.dy })
      else if (d.kind === 'resize' && d.next !== d.box) run({ cmd: 'setBox', id: Number(d.box.id), x: d.next.x, y: d.next.y, w: d.next.w, h: d.next.h, rot: d.box.rot })
      else if (d.kind === 'rotate') run({ cmd: 'setBox', id: Number(d.box.id), x: d.box.x, y: d.box.y, w: d.box.w, h: d.box.h, rot: d.rot })
      else if (d.kind === 'marquee' && (Math.abs(d.x1 - d.x0) > 2 || Math.abs(d.y1 - d.y0) > 2)) run({ cmd: 'selectIn', x0: d.x0, y0: d.y0, x1: d.x1, y1: d.y1, additive: d.additive })
    }
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', up)
    return () => {
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', up)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drag !== null, px])

  const onContext = (e: React.MouseEvent) => {
    e.preventDefault()
    const at = toSlide(e.clientX, e.clientY)
    const word = spelling?.slide === engine.slides[st.active]?.id ? slideWordAt(spelling, at.x, at.y) : null
    if (word) {
      const { clientX: x, clientY: y } = e
      const select = () => run({ cmd: 'selectText', id: Number(word.frame), anchor: [word.path, word.start], focus: [word.path, word.end] })
      // The word is selected, as other apps do, so the correction replaces it.
      select()
      void spellingMenu(word.word, (w) => {
        select()
        run({ cmd: 'insertText', text: w })
      }).then((items) => items.length && setMenu({ x, y, items }))
      return
    }
    if (!st.editing) {
      const p = toSlide(e.clientX, e.clientY)
      const hit = engine.doc.frameAt(p.x, p.y)
      if (hit && !st.frames.includes(hit)) run({ cmd: 'press', x: p.x, y: p.y, additive: false })
      else if (!hit) run({ cmd: 'selectFrames', ids: [] })
    }
    onContextMenu?.(e.clientX, e.clientY)
  }

  // ── Keyboard
  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (composing.current || e.nativeEvent.isComposing) return
    const mod = isMac ? e.metaKey : e.ctrlKey
    const byWord = isMac ? e.altKey : e.ctrlKey
    if (st.editing) {
      const moveTo = (to: string) => run({ cmd: 'moveCaret', to, extend: e.shiftKey })
      switch (e.key) {
        case 'ArrowLeft':
          moveTo(isMac && e.metaKey ? 'lineStart' : byWord ? 'wordLeft' : 'left')
          break
        case 'ArrowRight':
          moveTo(isMac && e.metaKey ? 'lineEnd' : byWord ? 'wordRight' : 'right')
          break
        case 'ArrowUp':
          moveTo(isMac && e.metaKey ? 'start' : 'up')
          break
        case 'ArrowDown':
          moveTo(isMac && e.metaKey ? 'end' : 'down')
          break
        case 'Home':
          moveTo(mod ? 'start' : 'lineStart')
          break
        case 'End':
          moveTo(mod ? 'end' : 'lineEnd')
          break
        case 'Backspace':
          run({ cmd: 'backspace', word: byWord })
          break
        case 'Delete':
          run({ cmd: 'deleteText', word: byWord })
          break
        case 'Enter':
          run({ cmd: e.shiftKey ? 'lineBreak' : 'enter' })
          break
        case 'Tab':
          run({ cmd: 'tab', shift: e.shiftKey })
          break
        case 'Escape':
          run({ cmd: 'endEdit' })
          break
        default: {
          if (!mod || e.altKey) return
          const k = e.key.toLowerCase()
          if (k === 'a') run({ cmd: 'selectAllText' })
          else if (k === 'z') run({ cmd: e.shiftKey ? 'redo' : 'undo' })
          else if (k === 'y' && !isMac) run({ cmd: 'redo' })
          else if (k === 'b' || k === 'i' || k === 'u') toggleText(engine, k)
          else return
        }
      }
      e.preventDefault()
      e.stopPropagation()
      return
    }
    // Objects.
    const nudge = e.shiftKey ? 10 : 1
    switch (e.key) {
      case 'Backspace':
      case 'Delete':
        if (!st.frames.length) return
        run({ cmd: 'delete' })
        break
      case 'ArrowLeft':
      case 'ArrowRight':
      case 'ArrowUp':
      case 'ArrowDown': {
        if (!st.frames.length) return
        const dx = e.key === 'ArrowLeft' ? -nudge : e.key === 'ArrowRight' ? nudge : 0
        const dy = e.key === 'ArrowUp' ? -nudge : e.key === 'ArrowDown' ? nudge : 0
        run({ cmd: 'moveFrames', ids: st.frames.map(Number), dx, dy })
        break
      }
      case 'Escape':
        if (!st.frames.length) return
        run({ cmd: 'selectFrames', ids: [] })
        break
      case 'Enter':
      case 'F2': {
        const id = st.frames.length === 1 ? st.frames[0] : null
        if (!id) return
        run({ cmd: 'beginEdit', id: Number(id), x: null, y: null })
        break
      }
      default: {
        if (!mod || e.altKey) return
        const k = e.key.toLowerCase()
        if (k === 'a') run({ cmd: 'selectFrames', ids: engine.elements.map((x) => Number(x.id)) })
        else if (k === 'z') run({ cmd: e.shiftKey ? 'redo' : 'undo' })
        else if (k === 'y' && !isMac) run({ cmd: 'redo' })
        else return
      }
    }
    e.preventDefault()
    e.stopPropagation()
  }

  const onBeforeInput = (e: React.FormEvent<HTMLTextAreaElement>) => {
    const ev = e.nativeEvent as InputEvent
    if (ev.inputType === 'historyUndo' || ev.inputType === 'historyRedo') {
      e.preventDefault()
      run({ cmd: ev.inputType === 'historyUndo' ? 'undo' : 'redo' })
      return
    }
    if (composing.current || (ev.inputType !== 'insertText' && ev.inputType !== 'insertReplacementText')) return
    const text = ev.data ?? ev.dataTransfer?.getData('text/plain') ?? ''
    e.preventDefault()
    if (!text) return
    typeText(text)
  }

  /** Typing goes into the text being edited, or starts editing the one picked text frame. */
  const typeText = (text: string) => {
    if (!engine.state.editing) {
      const id = engine.state.frames.length === 1 ? engine.state.frames[0] : null
      const el = id ? engine.elements.find((x) => x.id === id) : null
      if (!el || !(el.type === 'text' || el.type === 'shape')) return
      engine.run({ cmd: 'beginEdit', id: Number(id), x: null, y: null })
    }
    run({ cmd: 'insertText', text })
  }

  const onCopy = (e: React.ClipboardEvent, cut: boolean) => {
    if (st.editing) {
      const text = engine.doc.copyText()
      if (!text) return
      e.preventDefault()
      e.clipboardData.setData('text/plain', text)
      if (cut) run({ cmd: 'backspace', word: false })
      return
    }
    if (!st.frames.length) return
    e.preventDefault()
    e.clipboardData.setData(SLIDES_CLIPBOARD, engine.doc.copyFrames())
    e.clipboardData.setData('text/plain', engine.elements.filter((x) => st.frames.includes(x.id)).map((x) => x.text).join('\n'))
    if (cut) run({ cmd: 'delete' })
  }

  const onPaste = (e: React.ClipboardEvent) => {
    const frames = e.clipboardData.getData(SLIDES_CLIPBOARD)
    if (frames && !st.editing) {
      e.preventDefault()
      run({ cmd: 'pasteFrames', json: frames })
      return
    }
    const text = e.clipboardData.getData('text/plain')
    if (text && st.editing) {
      e.preventDefault()
      run({ cmd: 'insertText', text: text.replace(/\r\n?/g, '\n') })
    }
  }

  // ── Overlay
  const boxes = drag?.kind === 'resize' ? st.boxes.map((b) => (b.id === drag.box.id ? drag.next : b)) : drag?.kind === 'rotate' ? st.boxes.map((b) => (b.id === drag.box.id ? { ...b, rot: drag.rot } : b)) : st.boxes
  const moveOffset = drag?.kind === 'move' && drag.moved ? { dx: drag.dx, dy: drag.dy } : { dx: 0, dy: 0 }
  const text = st.text
  const caret = text?.caret

  // The input sits at the caret, so an input method's window opens there.
  const inputStyle: React.CSSProperties = caret && text ? { left: left + (text.x + caret[0]) * px, top: top + (text.y + caret[1]) * px, height: caret[2] * px, fontSize: Math.max(10, caret[2] * px * 0.7) } : {}

  return (
    <div
      className="sc-viewport"
      ref={(el) => {
        viewport.current = el
        if (viewportRef) viewportRef.current = el
      }}
    >
      <div className="sc-stage" style={{ width: stageW, height: stageH }} onMouseDown={onMouseDown} onContextMenu={onContext}>
        <div
          className="sc-slide"
          ref={(el) => {
            slideRef.current = el
            if (wrapperRef) wrapperRef.current = el
          }}
          style={{ left, top, width: sw, height: sh }}
        >
          <div className="slide-surface" style={{ width: engine.width, height: engine.height, transform: `scale(${px})` }} dangerouslySetInnerHTML={{ __html: html }} />
        </div>
        <div className="sc-overlay" style={{ left, top, width: engine.width, height: engine.height, transform: `scale(${px})` }}>
          {boxes.map((b) => (
            <div
              key={b.id}
              className={`sc-box ${st.editing === b.id ? 'editing' : ''}`}
              style={{ left: b.x + moveOffset.dx, top: b.y + moveOffset.dy, width: b.w, height: b.h, rotate: `${b.rot}deg`, borderWidth: 1.5 / px }}
            >
              {boxes.length === 1 && !st.editing && (
                <>
                  {HANDLES.map((h) => (
                    <div key={h} className="sc-handle" data-handle={h} style={{ ...handlePlace(h, b.w, b.h), width: 9 / px, height: 9 / px, margin: `${-5 / px}px 0 0 ${-5 / px}px`, borderWidth: 1.5 / px, cursor: `${cursorFor(h, b.rot)}` }} />
                  ))}
                  <div className="sc-rotate" data-handle="rot" style={{ top: -26 / px, width: 11 / px, height: 11 / px, marginLeft: -6 / px, borderWidth: 1.5 / px }} />
                </>
              )}
            </div>
          ))}
          {spelling?.slide === engine.slides[st.active]?.id && (!drag || drag.kind === 'text' || drag.kind === 'marquee') && (
            <div className="sc-spelling" style={{ ['--sc-px' as string]: 1 / px }}>
              {spelling.boxes.map((b) => (
                <div key={b.frame} style={{ position: 'absolute', left: b.x, top: b.y, width: b.w, height: b.h, rotate: `${b.rot}deg`, transformOrigin: '50% 50%' }}>
                  {b.rects.map((r, i) => (
                    <div key={i} className="sc-squiggle" style={{ left: r[1], top: r[2], width: r[3], height: r[4] }} />
                  ))}
                </div>
              ))}
            </div>
          )}
          {text && (
            <div style={{ position: 'absolute', left: text.x, top: text.y, width: text.w, height: text.h, rotate: `${text.rot}deg`, transformOrigin: '50% 50%' }}>
              {text.rects.map((r, i) => (
                <div key={i} className="sc-text-sel" style={{ left: r[0], top: r[1], width: r[2], height: r[3] }} />
              ))}
              {caret && <div key={engine.tick} className="sc-caret" style={{ left: caret[0], top: caret[1], height: caret[2], width: 1.5 / px }} />}
            </div>
          )}
          {drag?.kind === 'marquee' && (
            <div className="sc-marquee" style={{ left: Math.min(drag.x0, drag.x1), top: Math.min(drag.y0, drag.y1), width: Math.abs(drag.x1 - drag.x0), height: Math.abs(drag.y1 - drag.y0) }} />
          )}
        </div>
        <textarea
          ref={inputRef}
          className="sc-input"
          data-engine-input=""
          aria-label="Slide"
          autoCapitalize="off"
          autoComplete="off"
          autoCorrect="off"
          spellCheck={false}
          style={inputStyle}
          onKeyDown={onKeyDown}
          onBeforeInput={onBeforeInput}
          onInput={() => {
            const input = inputRef.current!
            if (composing.current || !input.value) return
            const v = input.value
            input.value = ''
            typeText(v)
          }}
          onCompositionStart={() => {
            composing.current = true
            inputRef.current?.classList.add('composing')
          }}
          onCompositionEnd={(e) => {
            composing.current = false
            const input = inputRef.current!
            input.classList.remove('composing')
            const v = e.data || input.value
            input.value = ''
            if (v) typeText(v)
          }}
          onCopy={(e) => onCopy(e, false)}
          onCut={(e) => onCopy(e, true)}
          onPaste={onPaste}
        />
      </div>
      {menu && (
        <PopoverMenu
          anchor={menu}
          items={menu.items}
          onClose={() => {
            setMenu(null)
            inputRef.current?.focus({ preventScroll: true })
          }}
        />
      )}
    </div>
  )
}

/** ⌘B, ⌘I and ⌘U: turn the style on unless the frame's text already has it. */
export function toggleText(engine: SlidesEngine, key: 'b' | 'i' | 'u') {
  const id = engine.state.editing ?? engine.state.frames[0]
  const el = engine.elements.find((x) => x.id === id)
  const s = el?.textStyle
  const field = key === 'b' ? 'bold' : key === 'i' ? 'italic' : 'underline'
  engine.run({ cmd: 'textStyle', patch: { [field]: !s?.[field] } })
}

function isPicture(engine: SlidesEngine, id: string) {
  return engine.elements.find((x) => x.id === id)?.type === 'image'
}

/** A handle's place on a box `w` by `h`. */
function handlePlace(h: HandleName, w: number, hh: number): React.CSSProperties {
  const x = h.includes('w') ? 0 : h.includes('e') ? w : w / 2
  const y = h.includes('n') ? 0 : h.includes('s') ? hh : hh / 2
  return { left: x, top: y }
}

function cursorFor(h: HandleName, rot: number) {
  const order: HandleName[] = ['n', 'ne', 'e', 'se', 's', 'sw', 'w', 'nw']
  const cursors = ['ns-resize', 'nesw-resize', 'ew-resize', 'nwse-resize']
  const i = (order.indexOf(h) + Math.round(rot / 45) + 8) % 8
  return cursors[i % 4]
}

/**
 * A box resized by dragging handle `h` by (`dx`, `dy`) slide pixels, measured along the box's
 * own (possibly rotated) axes; the opposite edge or corner stays put. `keep` keeps proportions.
 */
function resized(b: Box, h: HandleName, dx: number, dy: number, keep: boolean): Box {
  const a = (-b.rot * Math.PI) / 180
  const lx = dx * Math.cos(a) - dy * Math.sin(a)
  const ly = dx * Math.sin(a) + dy * Math.cos(a)
  let w = b.w
  let hh = b.h
  if (h.includes('e')) w = b.w + lx
  if (h.includes('w')) w = b.w - lx
  if (h.includes('s')) hh = b.h + ly
  if (h.includes('n')) hh = b.h - ly
  w = Math.max(4, w)
  hh = Math.max(4, hh)
  if (keep && (h.length === 2 || keep)) {
    const k = Math.max(w / b.w, hh / b.h)
    if (h.length === 2) {
      w = b.w * k
      hh = b.h * k
    }
  }
  // Keep the opposite corner fixed, in slide coordinates.
  const fx = h.includes('w') ? 1 : h.includes('e') ? 0 : 0.5
  const fy = h.includes('n') ? 1 : h.includes('s') ? 0 : 0.5
  const rot = (b.rot * Math.PI) / 180
  const anchor = (bw: number, bh: number, x: number, y: number) => {
    const cx = x + bw / 2
    const cy = y + bh / 2
    const ox = fx * bw - bw / 2
    const oy = fy * bh - bh / 2
    return { x: cx + ox * Math.cos(rot) - oy * Math.sin(rot), y: cy + ox * Math.sin(rot) + oy * Math.cos(rot) }
  }
  const fixed = anchor(b.w, b.h, b.x, b.y)
  const moved = anchor(w, hh, b.x, b.y)
  return { ...b, x: b.x + fixed.x - moved.x, y: b.y + fixed.y - moved.y, w, h: hh }
}
