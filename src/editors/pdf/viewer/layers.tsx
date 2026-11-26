// Layers drawn over a page: invisible selectable text, links and form fields, the selected
// annotation's handles, live drawing, the new-text box and search highlights. Positions are in
// points and scale with the page through the `--u` (CSS pixels per point) variable.

import type { PdfAnnotationObject, PdfInkAnnoObject } from '@embedpdf/models'
import { PdfAnnotationSubtype } from '@embedpdf/models'
import type { CSSProperties } from 'react'
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { Annot, Box, PageInfo, TextRun } from '../engine/protocol'
import type { DocStore } from '../engine/store'
import { wordPart } from '../ocr'
import { isWebLink } from '../../../lib/platform'

export const pos = (b: Box): CSSProperties => ({
  left: `calc(var(--u) * ${b.x}px)`,
  top: `calc(var(--u) * ${b.y}px)`,
  width: `calc(var(--u) * ${b.w}px)`,
  height: `calc(var(--u) * ${b.h}px)`,
})

// ── Text

/** Font size (points) of the invisible text; each line is scaled from it to its real size. */
const TEXT_SIZE = 10

let measureCtx: CanvasRenderingContext2D | null = null
function textWidth(text: string) {
  measureCtx ??= document.createElement('canvas').getContext('2d')!
  measureCtx.font = '100px sans-serif'
  return measureCtx.measureText(text).width / 100
}

/**
 * Invisible text laid over the drawing so it can be selected, copied and highlighted. `recognized`
 * is set once text has been recognized on a scanned page, which adds its words.
 */
export const TextLayer = memo(function TextLayer({ store, index, page, recognized }: { store: DocStore; index: number; page: PageInfo; recognized?: boolean }) {
  const [runs, setRuns] = useState<TextRun[] | null>(null)
  const indexRef = useRef(index)
  indexRef.current = index
  useEffect(() => {
    let live = true
    store
      .getText(indexRef.current)
      .then((r) => live && setRuns(r))
      .catch(() => {})
    return () => {
      live = false
    }
  }, [store, page.key, page.rotation, recognized])

  const children = useMemo(() => {
    if (!runs) return null
    const out: React.ReactNode[] = []
    let prev: TextRun | null = null
    runs.forEach((run, i) => {
      if (prev) {
        const sameLine = !run.vertical && !prev.vertical && Math.abs(run.box.y + run.box.h / 2 - (prev.box.y + prev.box.h / 2)) < prev.box.h / 2
        out.push(sameLine ? ' ' : <br key={`b${i}`} />)
      }
      prev = run
      const { x, y, w, h } = run.box
      // Every span uses the layer's one font size and is scaled to fit its box. A font size per
      // line (OCR text has a slightly different one on each) makes WebKit build a font for each
      // and crawl while scrolling.
      const size = (run.vertical ? w : h) / TEXT_SIZE
      const natural = textWidth(run.text) * TEXT_SIZE
      const stretch = natural > 0 ? (run.vertical ? h : w) / natural : 1
      const style: CSSProperties = run.vertical
        ? { left: `calc(var(--u) * ${x + w}px)`, top: `calc(var(--u) * ${y}px)`, transform: `rotate(90deg) scale(${stretch}, ${size})` }
        : { left: `calc(var(--u) * ${x}px)`, top: `calc(var(--u) * ${y}px)`, transform: `scale(${stretch}, ${size})` }
      out.push(
        <span key={i} data-run={i} style={style}>
          {run.text}
        </span>,
      )
    })
    return out
  }, [runs])

  return (
    <div className="text-layer" data-key={page.key} onMouseDown={dragSelect}>
      {children}
    </div>
  )
})

// ── Selecting text with the mouse

type Caret = { node: Node; offset: number }

/**
 * Selects text by pressing and dragging. The browser's own selection over this layer jumps
 * whenever the pointer is between lines or beside them: it places the caret by the layer's
 * invisible line breaks, which are stacked at the top of the page, not by where the text is. Here
 * each end goes to the text nearest the pointer instead. Double and triple clicks stay the
 * browser's (a word, a line).
 */
function dragSelect(e: React.MouseEvent<HTMLElement>) {
  if (e.button !== 0 || e.detail > 1 || e.metaKey || e.ctrlKey || e.altKey) return
  const pages = e.currentTarget.closest('.pdf-pages')
  const sel = window.getSelection()
  if (!pages || !sel) return
  const scroller = pages.closest<HTMLElement>('.pdf-container')
  const at = (x: number, y: number) => {
    const layer = layerAt(pages, x, y)
    return layer ? caretIn(layer, x, y) : null
  }
  // Shift-click extends the selection already on the pages.
  const start: Caret | null = e.shiftKey && sel.anchorNode && pages.contains(sel.anchorNode) ? { node: sel.anchorNode, offset: sel.anchorOffset } : at(e.clientX, e.clientY)
  if (!start) return
  e.preventDefault()
  scroller?.focus({ preventScroll: true })
  let last = { x: e.clientX, y: e.clientY }
  let end = start
  const extend = () => {
    end = at(last.x, last.y) ?? end
    sel.setBaseAndExtent(start.node, start.offset, end.node, end.offset)
  }
  extend()
  // Near or past the edge of the view, the pages scroll and the selection follows.
  let frame = 0
  const scroll = () => {
    frame = 0
    if (!scroller) return
    const r = scroller.getBoundingClientRect()
    const edge = 16
    const speed = (p: number, lo: number, hi: number) => (p < lo + edge ? Math.max(-40, p - lo - edge) : p > hi - edge ? Math.min(40, p - hi + edge) : 0)
    const dx = speed(last.x, r.left, r.right)
    const dy = speed(last.y, r.top, r.bottom)
    if (!dx && !dy) return
    scroller.scrollBy(dx / 2, dy / 2)
    extend()
    frame = requestAnimationFrame(scroll)
  }
  const move = (ev: MouseEvent) => {
    last = { x: ev.clientX, y: ev.clientY }
    extend()
    frame ||= requestAnimationFrame(scroll)
  }
  const up = () => {
    cancelAnimationFrame(frame)
    window.removeEventListener('mousemove', move)
    window.removeEventListener('mouseup', up)
    window.removeEventListener('blur', up)
  }
  window.addEventListener('mousemove', move)
  window.addEventListener('mouseup', up)
  window.addEventListener('blur', up)
}

/** The text layer of the page under (or else nearest) a point, if its text has loaded. */
function layerAt(pages: Element, x: number, y: number): HTMLElement | null {
  let best: HTMLElement | null = null
  let bestDist = Infinity
  for (const page of pages.querySelectorAll<HTMLElement>('.page')) {
    const r = page.getBoundingClientRect()
    const d = Math.max(r.top - y, 0, y - r.bottom) * 4 + Math.max(r.left - x, 0, x - r.right)
    if (d < bestDist) {
      bestDist = d
      best = page
    }
  }
  const layer = best?.querySelector<HTMLElement>('.text-layer')
  return layer?.querySelector('span[data-run]') ? layer : null
}

/**
 * The place in `layer`'s text nearest a point: in the run under it; else in the nearest run on
 * the same line, unless the point is over another column's text; else in the nearest run.
 */
function caretIn(layer: HTMLElement, x: number, y: number): Caret | null {
  let inside: { span: HTMLElement; area: number } | null = null
  let row: { span: HTMLElement; r: DOMRect; d: number } | null = null
  let near: { span: HTMLElement; r: DOMRect; d: number; over: boolean } | null = null
  const all: DOMRect[] = []
  for (const span of layer.querySelectorAll<HTMLElement>('span[data-run]')) {
    const r = span.getBoundingClientRect()
    all.push(r)
    const dx = Math.max(r.left - x, 0, x - r.right)
    const dy = Math.max(r.top - y, 0, y - r.bottom)
    // Overlapping runs (letters drawn twice, a ligature split in two): the smaller one.
    if (!dx && !dy) {
      if (!inside || r.width * r.height < inside.area) inside = { span, area: r.width * r.height }
    } else if (!dy) {
      if (!row || dx < row.d) row = { span, r, d: dx }
    }
    const d = Math.hypot(dx, dy)
    if (!near || d < near.d) near = { span, r, d, over: !dx }
  }
  // The stretch of the line a run is on: runs beside it, closer than the line is tall (the words of
  // a scanned page's line, not the next column).
  const line = (r: DOMRect) => {
    let left = r.left
    let right = r.right
    for (let grew = true; grew; ) {
      grew = false
      for (const o of all) {
        const mid = (o.top + o.bottom) / 2
        if (mid < r.top || mid > r.bottom) continue
        if (o.left < left && o.right >= left - r.height * 0.75) [left, grew] = [o.left, true]
        if (o.right > right && o.left <= right + r.height * 0.75) [right, grew] = [o.right, true]
      }
    }
    return { left, right }
  }
  const differentColumn = (a: DOMRect, b: DOMRect) => {
    const [p, q] = [line(a), line(b)]
    return p.right < q.left || q.right < p.left
  }
  const span = inside?.span ?? (row && !(near?.over && differentColumn(row.r, near.r)) ? row.span : near?.span)
  const text = span?.firstChild
  if (!span || !text) return null
  // The browser's own hit test within the run handles right-to-left and mixed text; it can land in
  // an overlapping run, so it's only taken when it stays in this one.
  const r = span.getBoundingClientRect()
  const px = Math.min(Math.max(x, r.left + 0.5), r.right - 0.5)
  const py = Math.min(Math.max(y, r.top + 0.5), r.bottom - 0.5)
  const hit = caretFromPoint(px, py)
  return { node: text, offset: hit?.node === text ? hit.offset : offsetIn(text, x, y) }
}

function caretFromPoint(x: number, y: number): Caret | null {
  const doc = document as Document & { caretPositionFromPoint?(x: number, y: number): { offsetNode: Node; offset: number } | null }
  if (doc.caretPositionFromPoint) {
    const p = doc.caretPositionFromPoint(x, y)
    return p && { node: p.offsetNode, offset: p.offset }
  }
  const range = doc.caretRangeFromPoint?.(x, y)
  return range ? { node: range.startContainer, offset: range.startOffset } : null
}

/** How many of a run's characters come before a point, along the direction its text runs. */
function offsetIn(text: Node, x: number, y: number) {
  const length = text.textContent?.length ?? 0
  const range = document.createRange()
  const centers: number[] = []
  const units: number[] = []
  const rects: DOMRect[] = []
  for (let i = 0; i < length; ) {
    const code = text.textContent!.charCodeAt(i)
    const n = code >= 0xd800 && code < 0xdc00 && i + 1 < length ? 2 : 1
    range.setStart(text, i)
    range.setEnd(text, i + n)
    rects.push(range.getBoundingClientRect())
    units.push(n)
    i += n
  }
  if (!rects.length) return 0
  const first = rects[0]
  const lastRect = rects[rects.length - 1]
  const vertical = rects.length > 1 && Math.abs(lastRect.top - first.top) > Math.abs(lastRect.left - first.left)
  const sign = (vertical ? lastRect.top - first.top : lastRect.left - first.left) < 0 ? -1 : 1
  for (const r of rects) centers.push(sign * (vertical ? r.top + r.height / 2 : r.left + r.width / 2))
  const p = sign * (vertical ? y : x)
  let offset = 0
  for (let i = 0; i < centers.length && centers[i] < p; i++) offset += units[i]
  return offset
}

/**
 * Drops a text selection that reaches into the pages (or `within`). Clicks that a tool handles
 * itself, and pages leaving the DOM, would otherwise leave it behind or let it grow.
 */
export function clearTextSelection(within?: Node) {
  const sel = window.getSelection()
  if (!sel?.rangeCount) return
  for (const root of within ? [within] : document.querySelectorAll('.pdf-pages')) {
    for (let i = 0; i < sel.rangeCount; i++) {
      if (sel.getRangeAt(i).intersectsNode(root)) return sel.removeAllRanges()
    }
  }
}

/**
 * Text picked by the current selection, per page: [first, last] PDFium character indices (-1 when
 * none), and boxes around picked words that were recognized on a scanned page.
 */
export async function selectedCharacters(store: DocStore): Promise<{ index: number; key: number; start: number; end: number; boxes: Box[] }[]> {
  const sel = window.getSelection()
  if (!sel || sel.isCollapsed || !sel.rangeCount) return []
  const range = sel.getRangeAt(0)
  const byKey = new Map<number, { start: number; end: number; boxes: Box[] }>()
  for (const layer of document.querySelectorAll<HTMLElement>('.text-layer')) {
    if (!range.intersectsNode(layer)) continue
    const key = Number(layer.dataset.key)
    const index = store.pages.findIndex((p) => p.key === key)
    if (index === -1) continue
    const runs = await store.getText(index)
    for (const span of layer.querySelectorAll<HTMLElement>('span[data-run]')) {
      if (!range.intersectsNode(span)) continue
      const run = runs[Number(span.dataset.run)]
      if (!run) continue
      const textNode = span.firstChild
      let from = 0
      let to = run.text.length
      if (textNode && range.startContainer === textNode) from = range.startOffset
      if (textNode && range.endContainer === textNode) to = range.endOffset
      if (to <= from) continue
      const have = byKey.get(key) ?? { start: -1, end: -1, boxes: [] }
      byKey.set(key, have)
      if (run.ocr) {
        have.boxes.push(wordPart(run, from, Math.min(to, run.text.length)))
        continue
      }
      const start = run.map[from]
      const end = run.map[Math.min(to, run.map.length) - 1]
      have.start = have.start === -1 ? start : Math.min(have.start, start)
      have.end = Math.max(have.end, end)
    }
  }
  return [...byKey].map(([key, r]) => ({ key, index: store.pages.findIndex((p) => p.key === key), ...r }))
}

// ── Annotations, links and form fields

/** Annotation types that can be picked with the arrow tool. */
const MOVABLE = new Set<number>([PdfAnnotationSubtype.INK, PdfAnnotationSubtype.STAMP, PdfAnnotationSubtype.FREETEXT, PdfAnnotationSubtype.SQUARE, PdfAnnotationSubtype.CIRCLE, PdfAnnotationSubtype.TEXT])
const MARKUP = new Set<number>([PdfAnnotationSubtype.HIGHLIGHT, PdfAnnotationSubtype.UNDERLINE, PdfAnnotationSubtype.STRIKEOUT, PdfAnnotationSubtype.SQUIGGLY])
const SELECTABLE = new Set<number>([...MOVABLE, ...MARKUP, PdfAnnotationSubtype.LINE, PdfAnnotationSubtype.POLYGON, PdfAnnotationSubtype.POLYLINE, PdfAnnotationSubtype.CARET])
const RESIZABLE = new Set<number>([PdfAnnotationSubtype.INK, PdfAnnotationSubtype.STAMP, PdfAnnotationSubtype.FREETEXT, PdfAnnotationSubtype.SQUARE, PdfAnnotationSubtype.CIRCLE])

export interface AnnotationActions {
  select(sel: { key: number; id: string } | null): void
  update(index: number, before: PdfAnnotationObject, after: PdfAnnotationObject): void
  remove(index: number, annot: Annot): void
  setField(index: number, annot: Annot, value: { text: string } | { checked: boolean } | { option: number }): void
  /** A signature field was clicked. */
  signField(index: number, annot: Annot): void
  openLink(link: NonNullable<Annot['link']>): void
}

export const AnnotationLayer = memo(function AnnotationLayer({
  store,
  index,
  page,
  rev,
  unit,
  interactive,
  selectable,
  selectedId,
  actions,
}: {
  store: DocStore
  index: number
  page: PageInfo
  rev: number
  unit: number
  /** Links and form fields respond (off while drawing or adding text). */
  interactive: boolean
  /** Annotations can be picked (arrow tool). */
  selectable: boolean
  selectedId: string | null
  actions: AnnotationActions
}) {
  const [annots, setAnnots] = useState<Annot[] | null>(null)
  const layerRef = useRef<HTMLDivElement>(null)
  const indexRef = useRef(index)
  indexRef.current = index

  useEffect(() => {
    let live = true
    store
      .getAnnotations(indexRef.current)
      .then((a) => live && setAnnots(a))
      .catch(() => {})
    return () => {
      live = false
    }
  }, [store, page.key, page.rotation, rev])

  // Highlights sit on text, so a plain click (not a drag that selects text) picks them.
  useEffect(() => {
    const pageEl = layerRef.current?.parentElement
    if (!pageEl || !selectable || !annots) return
    let down: { x: number; y: number } | null = null
    const onDown = (e: PointerEvent) => {
      down = { x: e.clientX, y: e.clientY }
    }
    const onClick = (e: MouseEvent) => {
      if (!down || Math.hypot(e.clientX - down.x, e.clientY - down.y) > 4) return
      if ((e.target as HTMLElement).closest('.annot-hit, .annot-selection, a, input, select, textarea, button')) return
      // Shift-click extended the text selection.
      if (e.shiftKey && !window.getSelection()?.isCollapsed) return
      const r = pageEl.getBoundingClientRect()
      const x = (e.clientX - r.left) / unit
      const y = (e.clientY - r.top) / unit
      const hit = annots.find((a) => MARKUP.has(a.type) && markupBoxes(a).some((b) => x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h))
      if (hit) {
        window.getSelection()?.removeAllRanges()
        actions.select({ key: page.key, id: hit.id })
      } else if (selectedId) actions.select(null)
    }
    pageEl.addEventListener('pointerdown', onDown)
    pageEl.addEventListener('click', onClick)
    return () => {
      pageEl.removeEventListener('pointerdown', onDown)
      pageEl.removeEventListener('click', onClick)
    }
  }, [annots, selectable, unit, page.key, selectedId, actions])

  const selected = selectedId ? annots?.find((a) => a.id === selectedId) : undefined

  return (
    <div ref={layerRef} className={`annot-layer ${interactive ? '' : 'inert'}`}>
      {annots?.map((a) => {
        if (a.link) {
          return (
            <a
              key={a.id}
              className="annot-link"
              style={pos(a.box)}
              // Only web links get a real href: a middle-click or drag mustn't reach file: or javascript:.
              href={a.link.url === undefined ? `#page=${(a.link.page ?? 0) + 1}` : isWebLink(a.link.url) ? a.link.url : '#'}
              title={a.link.url}
              onClick={(e) => {
                e.preventDefault()
                actions.openLink(a.link!)
              }}
            />
          )
        }
        if (a.field) {
          // A signature field with a signature already on it lets clicks through to that picture.
          if (a.field.kind === 'signature' && annots.some((o) => o.type === PdfAnnotationSubtype.STAMP && covers(o.box, a.box))) return null
          return <FormField key={a.id} annot={a} unit={unit} onChange={(value) => actions.setField(indexRef.current, a, value)} onSign={() => actions.signField(indexRef.current, a)} />
        }
        if (selectable && MOVABLE.has(a.type) && a.id !== selectedId) {
          return (
            <div
              key={a.id}
              className="annot-hit"
              style={pos(a.box)}
              onPointerDown={(e) => {
                e.preventDefault()
                e.stopPropagation()
                window.getSelection()?.removeAllRanges()
                actions.select({ key: page.key, id: a.id })
              }}
            />
          )
        }
        return null
      })}
      {selectable && selected && SELECTABLE.has(selected.type) && (
        <SelectionBox
          // A new box (after a move) starts a fresh drag.
          key={`${selected.id}:${selected.box.x}:${selected.box.y}:${selected.box.w}:${selected.box.h}`}
          annot={selected}
          unit={unit}
          page={page}
          onChange={(after) => actions.update(indexRef.current, selected.raw, after)}
          onDelete={() => actions.remove(indexRef.current, selected)}
        />
      )}
    </div>
  )
})

/** Whether picture `a` signs field `b`: mostly inside it, and not a small mark (a check). */
function covers(a: Box, b: Box) {
  const w = Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x)
  const h = Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y)
  return w > 0 && h > 0 && w * h > 0.5 * a.w * a.h && a.w * a.h > 0.2 * b.w * b.h
}

function markupBoxes(a: Annot): Box[] {
  const rects = (a.raw as { segmentRects?: { origin: { x: number; y: number }; size: { width: number; height: number } }[] }).segmentRects
  return rects?.length ? rects.map((r) => ({ x: r.origin.x, y: r.origin.y, w: r.size.width, h: r.size.height })) : [a.box]
}

/** Outline with a resize handle; drag to move, Delete to remove. */
function SelectionBox({ annot, unit, page, onChange, onDelete }: { annot: Annot; unit: number; page: PageInfo; onChange(after: PdfAnnotationObject): void; onDelete(): void }) {
  const [drag, setDrag] = useState<{ dx: number; dy: number; sw: number; sh: number } | null>(null)
  const ref = useRef<HTMLDivElement>(null)
  const movable = MOVABLE.has(annot.type)
  const resizable = RESIZABLE.has(annot.type)
  const b = annot.box
  const box = drag ? { x: b.x + drag.dx, y: b.y + drag.dy, w: Math.max(4, b.w * drag.sw), h: Math.max(4, b.h * drag.sh) } : b

  useEffect(() => {
    ref.current?.focus({ preventScroll: true })
  }, [])

  const start = (e: React.PointerEvent, mode: 'move' | 'resize') => {
    if (e.button !== 0) return
    e.preventDefault()
    e.stopPropagation()
    const el = e.currentTarget as HTMLElement
    el.setPointerCapture(e.pointerId)
    const sx = e.clientX
    const sy = e.clientY
    let last = { dx: 0, dy: 0, sw: 1, sh: 1 }
    const keepRatio = annot.type === PdfAnnotationSubtype.STAMP || annot.type === PdfAnnotationSubtype.INK
    const move = (ev: PointerEvent) => {
      const dx = (ev.clientX - sx) / unit
      const dy = (ev.clientY - sy) / unit
      if (mode === 'move') {
        last = { dx: clamp(dx, -b.x, page.width - b.x - b.w), dy: clamp(dy, -b.y, page.height - b.y - b.h), sw: 1, sh: 1 }
      } else {
        let sw = Math.max(8 / b.w, (b.w + dx) / b.w)
        let sh = Math.max(8 / b.h, (b.h + dy) / b.h)
        if (keepRatio) sw = sh = Math.max(sw, sh)
        sw = Math.min(sw, (page.width - b.x) / b.w)
        sh = Math.min(sh, (page.height - b.y) / b.h)
        if (keepRatio) sw = sh = Math.min(sw, sh)
        last = { dx: 0, dy: 0, sw, sh }
      }
      setDrag(last)
    }
    const up = () => {
      el.removeEventListener('pointermove', move)
      el.removeEventListener('pointerup', up)
      el.removeEventListener('pointercancel', up)
      if (last.dx || last.dy || last.sw !== 1 || last.sh !== 1) onChange(transformAnnot(annot.raw, b, last))
      else setDrag(null)
    }
    el.addEventListener('pointermove', move)
    el.addEventListener('pointerup', up)
    el.addEventListener('pointercancel', up)
  }

  return (
    <div
      ref={ref}
      className={`annot-selection ${movable ? 'movable' : ''}`}
      style={pos(box)}
      tabIndex={-1}
      onPointerDown={movable ? (e) => start(e, 'move') : undefined}
      onKeyDown={(e) => {
        if (e.key === 'Backspace' || e.key === 'Delete') {
          e.preventDefault()
          onDelete()
        }
      }}
    >
      {resizable && <div className="annot-handle" onPointerDown={(e) => start(e, 'resize')} />}
    </div>
  )
}

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), Math.max(lo, hi))

/** Move/scale an annotation (and an ink drawing's points) by a drag. */
function transformAnnot(raw: PdfAnnotationObject, b: Box, t: { dx: number; dy: number; sw: number; sh: number }): PdfAnnotationObject {
  const map = (p: { x: number; y: number }) => ({ x: b.x + t.dx + (p.x - b.x) * t.sw, y: b.y + t.dy + (p.y - b.y) * t.sh })
  const rect = { origin: map({ x: b.x, y: b.y }), size: { width: b.w * t.sw, height: b.h * t.sh } }
  if (raw.type === PdfAnnotationSubtype.INK) {
    const ink = raw as PdfInkAnnoObject
    return { ...ink, rect, inkList: ink.inkList.map((s) => ({ points: s.points.map(map) })) }
  }
  return { ...raw, rect }
}

function FormField({
  annot,
  unit,
  onChange,
  onSign,
}: {
  annot: Annot
  unit: number
  onChange(value: { text: string } | { checked: boolean } | { option: number }): void
  onSign(): void
}) {
  const f = annot.field!
  const [focused, setFocused] = useState(false)
  if (f.readOnly || f.kind === 'button' || f.kind === 'other') return null
  const style = pos(annot.box)
  if (f.kind === 'signature') {
    return (
      <button type="button" className="form-field sign" style={style} data-annot={annot.id} aria-label={`Sign ${f.name}`} title="Sign here" onClick={onSign}>
        <span style={{ fontSize: Math.max(9, Math.min(13, annot.box.h * 0.4)) * unit }}>Sign here</span>
      </button>
    )
  }
  if (f.kind === 'checkbox' || f.kind === 'radio') {
    return (
      <button
        type="button"
        className="form-field toggle"
        style={style}
        data-annot={annot.id}
        aria-label={f.name}
        aria-pressed={f.checked}
        onClick={() => onChange({ checked: f.kind === 'radio' ? true : !f.checked })}
      />
    )
  }
  if (f.kind === 'combo' || f.kind === 'list') {
    const selected = f.options.findIndex((o) => o.selected)
    return (
      <select className="form-field choice" style={style} data-annot={annot.id} aria-label={f.name} value={selected} onChange={(e) => onChange({ option: Number(e.currentTarget.value) })}>
        {selected === -1 && <option value={-1} />}
        {f.options.map((o, i) => (
          <option key={i} value={i}>
            {o.label}
          </option>
        ))}
      </select>
    )
  }
  const fontSize = (f.fontSize > 0 ? f.fontSize : Math.min(12, annot.box.h * 0.7)) * unit
  const props = {
    className: `form-field text ${focused ? 'editing' : ''}`,
    'data-annot': annot.id,
    style: { ...style, fontSize },
    'aria-label': f.name,
    defaultValue: f.value,
    spellCheck: false,
    onFocus: () => setFocused(true),
    onBlur: (e: React.FocusEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      setFocused(false)
      if (e.currentTarget.value !== f.value) onChange({ text: e.currentTarget.value })
    },
    onKeyDown: (e: React.KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      if (e.key === 'Enter' && !f.multiline) e.currentTarget.blur()
      if (e.key === 'Escape') {
        e.currentTarget.value = f.value
        e.currentTarget.blur()
      }
    },
  }
  return f.multiline ? <textarea {...props} /> : <input type={f.password ? 'password' : 'text'} {...props} />
}

// ── Drawing

export type Stroke = { x: number; y: number }[]

/** Captures freehand strokes on a page (draw tool). */
export function DrawLayer({ unit, color, width, strokes, onStroke }: { unit: number; color: string; width: number; strokes: Stroke[]; onStroke(stroke: Stroke): void }) {
  const [live, setLive] = useState<Stroke | null>(null)
  const ref = useRef<HTMLDivElement>(null)
  return (
    <div
      ref={ref}
      className="tool-layer draw"
      onPointerDown={(e) => {
        if (e.button !== 0) return
        e.preventDefault()
        clearTextSelection()
        const el = e.currentTarget
        el.setPointerCapture(e.pointerId)
        const r = el.getBoundingClientRect()
        const at = (ev: { clientX: number; clientY: number }) => ({ x: (ev.clientX - r.left) / unit, y: (ev.clientY - r.top) / unit })
        const stroke: Stroke = [at(e)]
        setLive([...stroke])
        const move = (ev: PointerEvent) => {
          const events = ev.getCoalescedEvents?.() ?? [ev]
          for (const c of events.length ? events : [ev]) stroke.push(at(c))
          setLive([...stroke])
        }
        const up = () => {
          el.removeEventListener('pointermove', move)
          el.removeEventListener('pointerup', up)
          el.removeEventListener('pointercancel', up)
          setLive(null)
          if (stroke.length === 1) stroke.push({ x: stroke[0].x + 0.01, y: stroke[0].y })
          onStroke(stroke)
        }
        el.addEventListener('pointermove', move)
        el.addEventListener('pointerup', up)
        el.addEventListener('pointercancel', up)
      }}
    >
      <InkPreview strokes={live ? [...strokes, live] : strokes} color={color} width={width} />
    </div>
  )
}

/** Strokes not yet written into the PDF, drawn in SVG (in points) so they track zoom for free. */
export function InkPreview({ strokes, color, width }: { strokes: Stroke[]; color: string; width: number }) {
  if (!strokes.length) return null
  return (
    <svg className="ink-preview" style={{ width: '100%', height: '100%' }}>
      <g style={{ transform: 'scale(var(--u))' }}>
        {strokes.map((s, i) => (
          <polyline key={i} points={s.map((p) => `${p.x},${p.y}`).join(' ')} fill="none" stroke={color} strokeWidth={width} strokeLinecap="round" strokeLinejoin="round" />
        ))}
      </g>
    </svg>
  )
}

/** Click to start a text box (text tool). */
export function TextToolLayer({ unit, onPlace }: { unit: number; onPlace(x: number, y: number): void }) {
  return (
    <div
      className="tool-layer text"
      onPointerDown={(e) => {
        if (e.button !== 0) return
        e.preventDefault()
        clearTextSelection()
        const r = e.currentTarget.getBoundingClientRect()
        onPlace((e.clientX - r.left) / unit, (e.clientY - r.top) / unit)
      }}
    />
  )
}

/** The text box being typed; becomes a FreeText annotation when it loses focus. */
export function TextDraft({ x, y, unit, fontSize, color, onDone }: { x: number; y: number; unit: number; fontSize: number; color: string; onDone(text: string, box: Box): void }) {
  const ref = useRef<HTMLDivElement>(null)
  const done = useRef(false)
  useLayoutEffect(() => {
    const el = ref.current!
    // Focus after the click that placed it has finished.
    requestAnimationFrame(() => el.focus())
  }, [])
  const finish = () => {
    if (done.current) return
    done.current = true
    const el = ref.current!
    const text = el.innerText.replace(/\n$/, '')
    const pad = 2
    onDone(text, { x, y: y - (fontSize * 1.2) / 2, w: el.offsetWidth / unit + pad, h: el.offsetHeight / unit + pad })
  }
  return (
    <div
      ref={ref}
      className="text-draft"
      contentEditable
      suppressContentEditableWarning
      spellCheck={false}
      style={{
        left: `calc(var(--u) * ${x}px)`,
        top: `calc(var(--u) * ${y - (fontSize * 1.2) / 2}px)`,
        fontSize: `calc(var(--u) * ${fontSize}px)`,
        color,
      }}
      onPointerDown={(e) => e.stopPropagation()}
      onBlur={finish}
      onKeyDown={(e) => {
        if (e.key === 'Escape' || (e.key === 'Enter' && (e.metaKey || e.ctrlKey))) {
          e.preventDefault()
          ref.current!.blur()
        }
      }}
    />
  )
}

// ── Search

export function SearchLayer({ boxes, current }: { boxes: Box[][]; current: number }) {
  return (
    <div className="search-layer">
      {boxes.map((match, i) =>
        match.map((b, j) => <div key={`${i}-${j}`} className={`search-hit ${i === current ? 'current' : ''}`} style={pos({ x: b.x - 1, y: b.y - 1, w: b.w + 2, h: b.h + 2 })} />),
      )}
    </div>
  )
}
