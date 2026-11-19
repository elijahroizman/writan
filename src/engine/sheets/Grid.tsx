// The spreadsheet grid: cells drawn by the engine a block at a time as the view scrolls, column
// letters and row numbers that stay in view, the selection, and typing into cells.

import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { isMac } from '../../lib/platform'
import { ChartSvg } from './Chart'
import type { ChartView, SheetsEngine } from './engine'
import { columnName } from './engine'
import './grid.css'

const HEAD_W = 46
const HEAD_H = 22
/** Rows and columns drawn beyond the view on each side. */
const MARGIN_ROWS = 20
const MARGIN_COLS = 6
const SHEETS_CLIPBOARD = 'application/x-writan-sheets'

interface Props {
  engine: SheetsEngine
  zoom: number
  active: boolean
  /** Editing a cell: what's typed so far, or null. Shared with the formula bar. */
  editing: string | null
  onEdit: (text: string | null) => void
  onCommit: (text: string, move: 'down' | 'right' | 'up' | 'left' | 'none') => void
  onContextMenu?: (x: number, y: number) => void
  /** Whether the cell being edited is on this sheet (a formula can point at another sheet's cells). */
  editingHere?: boolean
  /** The formula editor: whether a click on a cell now puts its reference into the formula. */
  canPoint?: () => boolean
  /**
   * Puts a reference into the formula: `replace` the one last put in, `extend` it from its first
   * cell (Shift-click), or `add` another after a comma (⌘-click). Returns the range's first cell.
   */
  onPoint?: (from: [number, number], to: [number, number], how?: 'replace' | 'extend' | 'add') => [number, number] | undefined
  /** The ranges the formula being typed refers to, drawn in their colors. */
  refBoxes?: { r0: number; c0: number; r1: number; c1: number; color: string }[]
  /** Hooks for the box typed into on the cell (suggestions, the caret). */
  editorHooks?: EditorHooks
  charts?: ChartView[]
  selectedChart?: number | null
  onSelectChart?: (index: number | null) => void
  onChartMenu?: (index: number, x: number, y: number) => void
}

export interface EditorHooks {
  onFocus: (el: HTMLTextAreaElement | HTMLInputElement) => void
  onCaret: (el: HTMLTextAreaElement | HTMLInputElement) => void
  /** Keys the suggestions want first; true when handled. */
  onKeyDown: (e: React.KeyboardEvent) => boolean
}

type Box = { x: number; y: number; w: number; h: number }
type ClipboardData = { text: string; spreadsheet: string | null }

export function Grid({
  engine,
  zoom,
  active,
  editing,
  onEdit,
  onCommit,
  onContextMenu,
  editingHere = true,
  canPoint,
  onPoint,
  refBoxes = [],
  editorHooks,
  charts = [],
  selectedChart = null,
  onSelectChart,
  onChartMenu,
}: Props) {
  const scroller = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const keyboardCut = useRef<{ data?: ClipboardData } | null>(null)
  const cellsRef = useRef<HTMLDivElement>(null)
  const [view, setView] = useState({ left: 0, top: 0, w: 800, h: 600 })
  const [, setTick] = useState(0)
  const drawn = useRef<{ key: string } | null>(null)
  const drag = useRef<
    | null
    | { kind: 'cells' | 'cols' | 'rows' | 'point'; from: [number, number] }
    | { kind: 'resizeCol' | 'resizeRow'; index: number; start: number; size: number }
    | { kind: 'chart'; index: number; handle: string; x: number; y: number; box: Box }
  >(null)
  // A chart being moved or resized: its box so far, in grid pixels.
  const [chartDrag, setChartDrag] = useState<{ index: number; box: Box } | null>(null)
  const chartBox = useRef<Box | null>(null)
  const chartRefs = useRef<(HTMLDivElement | null)[]>([])
  const [resize, setResize] = useState<{ kind: 'col' | 'row'; index: number; size: number } | null>(null)

  useEffect(() => engine.subscribe(() => setTick((t) => t + 1)), [engine])
  useEffect(() => {
    if (active && editing === null) inputRef.current?.focus({ preventScroll: true })
  }, [active, editing, engine])

  const st = engine.state
  const sheet = st.sheet
  const g = engine.geometryOf(sheet)
  const z = zoom

  useLayoutEffect(() => {
    const el = scroller.current!
    const measure = () => setView({ left: el.scrollLeft, top: el.scrollTop, w: el.clientWidth, h: el.clientHeight })
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    let frame = 0
    const onScroll = () => {
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(measure)
    }
    el.addEventListener('scroll', onScroll, { passive: true })
    return () => {
      ro.disconnect()
      el.removeEventListener('scroll', onScroll)
      cancelAnimationFrame(frame)
    }
  }, [])

  // The block of cells in view, with a margin around it.
  const r0 = Math.max(0, g.rowAt(view.top / z) - MARGIN_ROWS)
  const r1 = Math.min(g.info.rows, g.rowAt((view.top + view.h) / z) + MARGIN_ROWS)
  const c0 = Math.max(0, g.colAt(view.left / z) - MARGIN_COLS)
  const c1 = Math.min(g.info.cols, g.colAt((view.left + view.w) / z) + MARGIN_COLS)
  // Draw again only when the block moves past what's drawn, or the sheet changes.
  useLayoutEffect(() => {
    const el = cellsRef.current
    if (!el) return
    const key = `${sheet}:${engine.revision}`
    const block = drawn.current as (null | { key: string; r0: number; r1: number; c0: number; c1: number })
    const inside = block && block.key === key && r0 >= block.r0 && r1 <= block.r1 && c0 >= block.c0 && c1 <= block.c1
    if (inside) return
    // Draw a little more than needed, so small scrolls reuse it.
    const br0 = Math.max(0, r0 - MARGIN_ROWS)
    const br1 = Math.min(g.info.rows, r1 + MARGIN_ROWS)
    const bc0 = Math.max(0, c0 - MARGIN_COLS)
    const bc1 = Math.min(g.info.cols, c1 + MARGIN_COLS)
    el.innerHTML = engine.paint(sheet, br0, br1, bc0, bc1)
    drawn.current = { key, r0: br0, r1: br1, c0: bc0, c1: bc1 } as never
  })

  const range = engine.range
  const selX = g.x(range.c0) * z
  const selY = g.y(range.r0) * z
  const selW = (g.x(range.c1 + 1) - g.x(range.c0)) * z
  const selH = (g.y(range.r1 + 1) - g.y(range.r0)) * z
  const [fr, fc] = st.focus
  const curX = g.x(fc) * z
  const curY = g.y(fr) * z
  const curW = (g.x(fc + 1) - g.x(fc)) * z
  const curH = (g.y(fr + 1) - g.y(fr)) * z

  /** Keeps the focused cell in view. */
  const reveal = (r: number, c: number) => {
    const el = scroller.current!
    const x = g.x(c) * z
    const y = g.y(r) * z
    const w = (g.x(c + 1) - g.x(c)) * z
    const h = (g.y(r + 1) - g.y(r)) * z
    if (x < el.scrollLeft) el.scrollLeft = x
    else if (x + w > el.scrollLeft + el.clientWidth - HEAD_W) el.scrollLeft = x + w - el.clientWidth + HEAD_W
    if (y < el.scrollTop) el.scrollTop = y
    else if (y + h > el.scrollTop + el.clientHeight - HEAD_H) el.scrollTop = y + h - el.clientHeight + HEAD_H
  }

  const select = (anchor: [number, number], focus: [number, number]) => {
    if (editing !== null) onCommit(editing, 'none')
    const clamp = (p: [number, number]): [number, number] => [Math.max(0, Math.min(g.info.rows - 1, p[0])), Math.max(0, Math.min(g.info.cols - 1, p[1]))]
    engine.run({ cmd: 'select', sheet, anchor: clamp(anchor), focus: clamp(focus) })
    reveal(...clamp(focus))
  }

  // ── Pointer
  const cellAt = (e: { clientX: number; clientY: number }): [number, number] => {
    const r = cellsRef.current!.getBoundingClientRect()
    return [g.rowAt((e.clientY - r.top) / z), g.colAt((e.clientX - r.left) / z)]
  }
  const onCellsDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return
    e.preventDefault()
    // Typing a formula: the click puts the cell's reference into it, and the editor keeps focus.
    // Shift-click stretches the reference to the cell, and ⌘-click (Ctrl on Windows) adds another, as in Excel.
    if (editing !== null && canPoint?.()) {
      const at = cellAt(e)
      const from = onPoint?.(at, at, e.shiftKey ? 'extend' : (isMac ? e.metaKey : e.ctrlKey) ? 'add' : 'replace') ?? at
      drag.current = { kind: 'point', from }
      return
    }
    onSelectChart?.(null)
    inputRef.current?.focus({ preventScroll: true })
    const at = cellAt(e)
    if (e.detail >= 2) {
      select(at, at)
      onEdit(engine.cell?.input ?? '')
      return
    }
    if (e.shiftKey) select(st.anchor, at)
    else select(at, at)
    drag.current = { kind: 'cells', from: e.shiftKey ? st.anchor : at }
  }
  const onColHeadDown = (e: React.MouseEvent, c: number) => {
    e.preventDefault()
    inputRef.current?.focus({ preventScroll: true })
    const edge = (e.target as HTMLElement).dataset.edge
    if (edge) {
      const col = Number(edge)
      drag.current = { kind: 'resizeCol', index: col, start: e.clientX, size: g.x(col + 1) - g.x(col) }
      setResize({ kind: 'col', index: col, size: g.x(col + 1) - g.x(col) })
      return
    }
    select([0, e.shiftKey ? st.anchor[1] : c], [g.info.rows - 1, c])
    drag.current = { kind: 'cols', from: [0, c] }
  }
  const onRowHeadDown = (e: React.MouseEvent, r: number) => {
    e.preventDefault()
    inputRef.current?.focus({ preventScroll: true })
    const edge = (e.target as HTMLElement).dataset.edge
    if (edge) {
      const row = Number(edge)
      drag.current = { kind: 'resizeRow', index: row, start: e.clientY, size: g.y(row + 1) - g.y(row) }
      setResize({ kind: 'row', index: row, size: g.y(row + 1) - g.y(row) })
      return
    }
    select([e.shiftKey ? st.anchor[0] : r, 0], [r, g.info.cols - 1])
    drag.current = { kind: 'rows', from: [r, 0] }
  }
  useEffect(() => {
    const move = (e: MouseEvent) => {
      const d = drag.current
      if (!d) return
      if (d.kind === 'resizeCol' || d.kind === 'resizeRow') {
        const delta = ((d.kind === 'resizeCol' ? e.clientX : e.clientY) - d.start) / z
        setResize({ kind: d.kind === 'resizeCol' ? 'col' : 'row', index: d.index, size: Math.max(4, d.size + delta) })
        return
      }
      if (d.kind === 'chart') {
        const dx = (e.clientX - d.x) / z
        const dy = (e.clientY - d.y) / z
        const b = { ...d.box }
        if (d.handle === 'move') {
          b.x = Math.max(0, b.x + dx)
          b.y = Math.max(0, b.y + dy)
        } else {
          if (d.handle.includes('e')) b.w = Math.max(40, b.w + dx)
          if (d.handle.includes('s')) b.h = Math.max(40, b.h + dy)
          if (d.handle.includes('w')) {
            const w = Math.max(40, b.w - dx)
            b.x = Math.max(0, b.x + b.w - w)
            b.w = w
          }
          if (d.handle.includes('n')) {
            const h = Math.max(40, b.h - dy)
            b.y = Math.max(0, b.y + b.h - h)
            b.h = h
          }
        }
        chartBox.current = b
        setChartDrag({ index: d.index, box: b })
        return
      }
      if (d.kind === 'point') {
        onPoint?.(d.from, cellAt(e))
        return
      }
      if (d.kind !== 'cells' && d.kind !== 'cols' && d.kind !== 'rows') return
      const at = cellAt(e)
      if (d.kind === 'cells') select(d.from, at)
      else if (d.kind === 'cols') select([0, d.from[1]], [g.info.rows - 1, at[1]])
      else select([d.from[0], 0], [at[0], g.info.cols - 1])
    }
    const up = () => {
      const d = drag.current
      drag.current = null
      if (d?.kind === 'chart') {
        const b = chartBox.current
        chartBox.current = null
        if (b && (b.x !== d.box.x || b.y !== d.box.y || b.w !== d.box.w || b.h !== d.box.h)) {
          engine.run({ cmd: 'moveChart', sheet, index: d.index, x: Math.round(b.x), y: Math.round(b.y), w: Math.round(b.w), h: Math.round(b.h) })
        }
        setChartDrag(null)
        return
      }
      if (d && (d.kind === 'resizeCol' || d.kind === 'resizeRow')) {
        setResize((r) => {
          if (r) engine.run(r.kind === 'col' ? { cmd: 'columnWidth', sheet, col: r.index, px: Math.round(r.size) } : { cmd: 'rowHeight', sheet, row: r.index, px: Math.round(r.size) })
          return null
        })
      }
    }
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', up)
    return () => {
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', up)
    }
  })

  // ── Keyboard (while not editing a cell)
  const endCutGesture = () => {
    keyboardCut.current = null
  }
  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing) return
    // Keys typed before the cell editor took focus still belong to it.
    if (editing !== null) {
      const move = e.key === 'Enter' ? (e.shiftKey ? 'up' : 'down') : e.key === 'Tab' ? (e.shiftKey ? 'left' : 'right') : null
      if (move) {
        e.preventDefault()
        onCommit(editing, move)
      } else if (e.key === 'Escape') {
        e.preventDefault()
        onEdit(null)
      }
      return
    }
    const mod = isMac ? e.metaKey : e.ctrlKey
    if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'x') {
      if (!e.repeat || !keyboardCut.current) keyboardCut.current = {}
    } else endCutGesture()
    const [r, c] = st.focus
    const step = (dr: number, dc: number) => {
      // ⌘-arrow goes to the edge of the sheet's used cells, roughly as Excel does.
      if (mod) {
        const nr = dr ? (dr > 0 ? g.info.content[0] - 1 : 0) : r
        const nc = dc ? (dc > 0 ? g.info.content[1] - 1 : 0) : c
        return e.shiftKey ? select(st.anchor, [Math.max(nr, 0), Math.max(nc, 0)]) : select([Math.max(nr, 0), Math.max(nc, 0)], [Math.max(nr, 0), Math.max(nc, 0)])
      }
      const next: [number, number] = [r + dr, c + dc]
      if (e.shiftKey) select(st.anchor, next)
      else select(next, next)
    }
    switch (e.key) {
      case 'ArrowUp':
        step(-1, 0)
        break
      case 'ArrowDown':
        step(1, 0)
        break
      case 'ArrowLeft':
        step(0, -1)
        break
      case 'ArrowRight':
        step(0, 1)
        break
      case 'Enter':
        step(e.shiftKey ? -1 : 1, 0)
        break
      case 'Tab':
        step(0, e.shiftKey ? -1 : 1)
        break
      case 'PageDown':
        step(Math.max(1, Math.floor(view.h / z / g.info.defaultHeight) - 1), 0)
        break
      case 'PageUp':
        step(-Math.max(1, Math.floor(view.h / z / g.info.defaultHeight) - 1), 0)
        break
      case 'Home':
        select([mod ? 0 : r, 0], [mod ? 0 : r, 0])
        break
      case 'Delete':
      case 'Backspace':
        engine.run({ cmd: 'clear', sheet, range: engine.range })
        break
      case 'F2':
        onEdit(engine.cell?.input ?? '')
        break
      default: {
        if (!mod || e.altKey) return
        const k = e.key.toLowerCase()
        if (k === 'z') engine.run({ cmd: e.shiftKey ? 'redo' : 'undo' })
        else if (k === 'y' && !isMac) engine.run({ cmd: 'redo' })
        else if (k === 'a') select([0, 0], [g.info.rows - 1, g.info.cols - 1])
        else if (k === 'b') engine.run({ cmd: 'format', sheet, range: engine.range, patch: { bold: !engine.cell?.bold } })
        else if (k === 'i') engine.run({ cmd: 'format', sheet, range: engine.range, patch: { italic: !engine.cell?.italic } })
        else if (k === 'u') engine.run({ cmd: 'format', sheet, range: engine.range, patch: { underline: !engine.cell?.underline } })
        else return
      }
    }
    e.preventDefault()
    e.stopPropagation()
  }

  // Typing on a cell starts editing it with what was typed.
  // (A native listener: React's onBeforeInput comes from textInput, without an inputType.)
  const onBeforeInput = (ev: InputEvent) => {
    if (ev.inputType === 'historyUndo' || ev.inputType === 'historyRedo') {
      ev.preventDefault()
      engine.run({ cmd: ev.inputType === 'historyUndo' ? 'undo' : 'redo' })
      return
    }
    if (ev.inputType === 'insertText' && ev.data && !ev.isComposing) {
      ev.preventDefault()
      onEdit((editing ?? '') + ev.data)
    }
  }
  const beforeInput = useRef(onBeforeInput)
  beforeInput.current = onBeforeInput
  useEffect(() => {
    const el = inputRef.current!
    const f = (e: InputEvent) => beforeInput.current(e)
    el.addEventListener('beforeinput', f)
    return () => el.removeEventListener('beforeinput', f)
  }, [])

  const onCopy = (e: React.ClipboardEvent, cut: boolean) => {
    const rr = engine.range
    const gesture = cut ? keyboardCut.current : null
    const previous = gesture?.data
    const moveId = cut && !previous ? Array.from(crypto.getRandomValues(new Uint32Array(4))).join('-') : undefined
    let data = previous
    if (!data) {
      const { text, clipboard } = JSON.parse(engine.doc.copy(sheet, rr.r0, rr.c0, rr.r1, rr.c1, moveId)) as { text: string; clipboard: unknown }
      data = { text, spreadsheet: clipboard ? JSON.stringify(clipboard) : null }
    }
    e.preventDefault()
    e.clipboardData.setData('text/plain', data.text)
    if (data.spreadsheet) e.clipboardData.setData(SHEETS_CLIPBOARD, data.spreadsheet)
    if (moveId) {
      // WebKit can send two cut events for one keypress. Both must write the pre-cut data.
      if (gesture) gesture.data = data
      engine.run({ cmd: 'cut', sheet, range: rr, moveId })
    }
  }
  const onPaste = (e: React.ClipboardEvent) => {
    const text = e.clipboardData.getData('text/plain')
    const clipboard = e.clipboardData.getData(SHEETS_CLIPBOARD)
    if (!text && !clipboard) return
    e.preventDefault()
    engine.run({ cmd: 'paste', sheet, range: engine.range, text, clipboard: clipboard || null })
  }

  // ── Charts
  const onChartDown = (e: React.MouseEvent, index: number, handle: string) => {
    if (e.button !== 0) return
    e.stopPropagation()
    e.preventDefault()
    if (editing !== null) onCommit(editing, 'none')
    onSelectChart?.(index)
    chartRefs.current[index]?.focus({ preventScroll: true })
    const c = charts[index]
    drag.current = { kind: 'chart', index, handle, x: e.clientX, y: e.clientY, box: { x: c.x, y: c.y, w: c.w, h: c.h } }
  }
  const onChartKey = (e: React.KeyboardEvent, index: number) => {
    const c = charts[index]
    const nudge = e.shiftKey ? 10 : 1
    const move = (dx: number, dy: number) => engine.run({ cmd: 'moveChart', sheet, index, x: Math.max(0, c.x + dx), y: Math.max(0, c.y + dy), w: c.w, h: c.h })
    const mod = isMac ? e.metaKey : e.ctrlKey
    if (e.key === 'Delete' || e.key === 'Backspace') {
      engine.run({ cmd: 'deleteChart', sheet, index })
      onSelectChart?.(null)
      inputRef.current?.focus({ preventScroll: true })
    } else if (e.key === 'Escape') {
      onSelectChart?.(null)
      inputRef.current?.focus({ preventScroll: true })
    } else if (e.key === 'ArrowLeft') move(-nudge, 0)
    else if (e.key === 'ArrowRight') move(nudge, 0)
    else if (e.key === 'ArrowUp') move(0, -nudge)
    else if (e.key === 'ArrowDown') move(0, nudge)
    else if (mod && !e.altKey && e.key.toLowerCase() === 'z') engine.run({ cmd: e.shiftKey ? 'redo' : 'undo' })
    else return
    e.preventDefault()
    e.stopPropagation()
  }
  // Keep focus on the selected chart (after undo, or when picked from elsewhere).
  useEffect(() => {
    if (selectedChart !== null && active) chartRefs.current[selectedChart]?.focus({ preventScroll: true })
  }, [selectedChart, active])

  // ── Headers in view
  const cols: number[] = []
  for (let c = c0; c < c1; c++) cols.push(c)
  const rows: number[] = []
  for (let r = r0; r < r1; r++) rows.push(r)
  const inSel = (k: number, lo: number, hi: number) => k >= lo && k <= hi

  return (
    <div className="sg-scroller" ref={scroller} onContextMenu={(e) => {
      e.preventDefault()
      const at = cellAt(e)
      if (!(inSel(at[0], range.r0, range.r1) && inSel(at[1], range.c0, range.c1))) select(at, at)
      onContextMenu?.(e.clientX, e.clientY)
    }}>
      <div className="sg-space" style={{ width: g.width * z + HEAD_W, height: g.height * z + HEAD_H }}>
        <div className="sg-corner" onMouseDown={(e) => {
          e.preventDefault()
          select([0, 0], [g.info.rows - 1, g.info.cols - 1])
        }} />
        <div className="sg-colheads" style={{ width: g.width * z }}>
          {cols.map((c) => (
            <div key={c} className={`sg-colhead ${inSel(c, range.c0, range.c1) ? 'on' : ''}`} style={{ left: g.x(c) * z, width: (g.x(c + 1) - g.x(c)) * z }} onMouseDown={(e) => onColHeadDown(e, c)}>
              {columnName(c)}
              <span className="sg-edge" data-edge={c} />
            </div>
          ))}
        </div>
        <div className="sg-rowheads" style={{ height: g.height * z }}>
          {rows.map((r) => (
            <div key={r} className={`sg-rowhead ${inSel(r, range.r0, range.r1) ? 'on' : ''}`} style={{ top: g.y(r) * z, height: (g.y(r + 1) - g.y(r)) * z }} onMouseDown={(e) => onRowHeadDown(e, r)}>
              {r + 1}
              <span className="sg-edge" data-edge={r} />
            </div>
          ))}
        </div>
        <div className="sg-body" style={{ width: g.width * z, height: g.height * z }} onMouseDown={onCellsDown}>
          <div className="sg-cells" ref={cellsRef} style={{ width: g.width, height: g.height, transform: `scale(${z})` }} />
          <div className="sg-sel" style={{ left: selX, top: selY, width: selW, height: selH }} />
          <div className="sg-cur" style={{ left: curX, top: curY, width: curW, height: curH }} />
          {refBoxes.map((b, i) => (
            <div
              key={`ref${i}`}
              className="sg-ref"
              style={{
                left: g.x(b.c0) * z,
                top: g.y(b.r0) * z,
                width: (g.x(b.c1 + 1) - g.x(b.c0)) * z,
                height: (g.y(b.r1 + 1) - g.y(b.r0)) * z,
                '--ref': b.color,
              } as React.CSSProperties}
            />
          ))}
          {engine.picturesOf(sheet).map((p, i) => {
            const src = engine.media.url(p.media)
            const [l, t, r, b] = p.crop ?? [0, 0, 0, 0]
            const iw = p.w / Math.max(0.01, 1 - l - r)
            const ih = p.h / Math.max(0.01, 1 - t - b)
            return (
              <div key={`pic${i}`} className="sg-picture" role="img" aria-label={p.name ?? 'Picture'} style={{ left: p.x * z, top: p.y * z, width: p.w * z, height: p.h * z }}>
                {src && <img src={src} alt="" draggable={false} style={{ left: -l * iw * z, top: -t * ih * z, width: iw * z, height: ih * z }} />}
              </div>
            )
          })}
          {editing === null && engine.cell?.validation?.prompt && (
            <div className="sg-prompt" role="tooltip" style={{ left: curX, top: curY + curH + 4 }}>
              {engine.cell.validation.promptTitle && <strong>{engine.cell.validation.promptTitle}</strong>}
              <span>{engine.cell.validation.prompt}</span>
            </div>
          )}
          {editing === null && engine.cell?.validation?.list && (
            <ValidationList
              items={engine.cell.validation.list}
              style={{ left: curX + curW + 1, top: curY }}
              height={curH}
              onPick={(v) => onCommit(v, 'none')}
            />
          )}
          {charts.map((c, i) => {
            const box = chartDrag?.index === i ? chartDrag.box : c
            const on = selectedChart === i
            return (
              <div
                key={i}
                ref={(el) => {
                  chartRefs.current[i] = el
                }}
                className={`sg-chart ${on ? 'on' : ''}`}
                tabIndex={-1}
                role="img"
                aria-label={c.title ? `Chart: ${c.title}` : 'Chart'}
                style={{ left: box.x * z, top: box.y * z, width: box.w * z, height: box.h * z }}
                onMouseDown={(e) => onChartDown(e, i, 'move')}
                onKeyDown={(e) => onChartKey(e, i)}
                onContextMenu={(e) => {
                  e.preventDefault()
                  e.stopPropagation()
                  onSelectChart?.(i)
                  onChartMenu?.(i, e.clientX, e.clientY)
                }}
              >
                <div className="sg-chart-scale" style={{ width: box.w, height: box.h, transform: `scale(${z})` }}>
                  <ChartSvg chart={c} width={box.w} height={box.h} />
                </div>
                {on &&
                  ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'].map((h) => (
                    <span key={h} className={`sg-handle ${h}`} onMouseDown={(e) => onChartDown(e, i, h)} />
                  ))}
              </div>
            )
          })}
          {resize && (
            <div
              className="sg-resize-guide"
              style={resize.kind === 'col' ? { left: (g.x(resize.index) + resize.size) * z, top: 0, width: 1, height: '100%' } : { top: (g.y(resize.index) + resize.size) * z, left: 0, height: 1, width: '100%' }}
            />
          )}
          {editing !== null && editingHere && (
            <CellEditor
              hooks={editorHooks}
              value={editing}
              style={{ left: curX, top: curY, minWidth: curW, minHeight: curH, fontSize: Math.max(11, (engine.cell?.size ?? 11) * (96 / 72) * z) }}
              onChange={onEdit}
              onCommit={(move) => onCommit(editing, move)}
              onCancel={() => onEdit(null)}
            />
          )}
        </div>
        <textarea
          ref={inputRef}
          className="sg-input"
          data-engine-input=""
          aria-label="Cells"
          style={{ left: curX + HEAD_W, top: curY + HEAD_H }}
          onKeyDown={onKeyDown}
          onKeyUp={endCutGesture}
          onBlur={endCutGesture}
          onCompositionEnd={(e) => {
            const v = e.data
            ;(e.target as HTMLTextAreaElement).value = ''
            if (v) onEdit(v)
          }}
          onCopy={(e) => onCopy(e, false)}
          onCut={(e) => onCopy(e, true)}
          onPaste={onPaste}
        />
      </div>
    </div>
  )
}

/** The box typed into on the cell itself. Enter and Tab commit and move; Escape cancels. */
function CellEditor({
  value,
  style,
  hooks,
  onChange,
  onCommit,
  onCancel,
}: {
  value: string
  style: React.CSSProperties
  hooks?: EditorHooks
  onChange: (v: string) => void
  onCommit: (move: 'down' | 'right' | 'up' | 'left' | 'none') => void
  onCancel: () => void
}) {
  const ref = useRef<HTMLTextAreaElement>(null)
  // Focused before the next key arrives, so a fast typist's second letter lands here.
  useLayoutEffect(() => {
    const el = ref.current!
    el.focus({ preventScroll: true })
    el.setSelectionRange(el.value.length, el.value.length)
    hooks?.onFocus(el)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  return (
    <textarea
      ref={ref}
      className="sg-editor"
      data-engine-input=""
      value={value}
      style={style}
      spellCheck={false}
      onMouseDown={(e) => e.stopPropagation()}
      onFocus={(e) => hooks?.onFocus(e.currentTarget)}
      onSelect={(e) => hooks?.onCaret(e.currentTarget)}
      onChange={(e) => onChange(e.target.value)}
      onKeyDown={(e) => {
        if (hooks?.onKeyDown(e)) {
          e.stopPropagation()
          return
        }
        if (e.key === 'Enter' && (e.altKey || (isMac ? false : e.ctrlKey))) return
        if (e.key === 'Enter') {
          e.preventDefault()
          onCommit(e.shiftKey ? 'up' : 'down')
        } else if (e.key === 'Tab') {
          e.preventDefault()
          onCommit(e.shiftKey ? 'left' : 'right')
        } else if (e.key === 'Escape') {
          e.preventDefault()
          onCancel()
        }
        e.stopPropagation()
      }}
    />
  )
}

/** The drop-down arrow beside a cell whose data validation is a list, and the list it opens. */
function ValidationList({ items, style, height, onPick }: { items: string[]; style: React.CSSProperties; height: number; onPick: (v: string) => void }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="sg-dropdown" style={style} onMouseDown={(e) => e.stopPropagation()}>
      <button type="button" className="sg-dropdown-button" aria-label="Choose a value" aria-expanded={open} style={{ height }} onClick={() => setOpen(!open)}>
        ▾
      </button>
      {open && (
        <ul className="sg-dropdown-list" role="listbox">
          {items.map((v, i) => (
            <li
              key={i}
              role="option"
              aria-selected={false}
              onClick={() => {
                setOpen(false)
                onPick(v)
              }}
            >
              {v}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
