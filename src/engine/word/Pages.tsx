// The Word page view: pages stacked in a scrolling column, drawn from the engine's display lists
// only while they're near the screen, with the caret and selection over them and a hidden text
// field taking the keyboard (and the input method's composition, which shows at the caret).

import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { MenuEntry } from '../../components/ui'
import { PopoverMenu } from '../../components/ui'
import { shrinkPicture } from '../../lib/pictures'
import { isMac, openExternal, shortcut } from '../../lib/platform'
import { spellingMenu } from '../../lib/spelling'
import { dimming, useFocus } from '../../state/focus'
import { PeerCaret, PeerSelection, together } from '../../collab/cursors'
import { usePlaces } from '../../collab/session'
import { itemsHtml } from '../paint'
import type { WordEngine } from './engine'
import { spellingAt, watchSpelling } from './spelling'

/** Space above the first page and between pages, before zoom. */
const TOP = 24
const GAP = 16
const SIDE = 24
/** Pages this far beyond the screen stay drawn; further ones are dropped. */
const KEEP = 3

export const WORD_CLIPBOARD = 'application/x-workspace-word'

/** First page index whose bottom reaches past `y` (O(log P)). */
function firstPageAtOrAfter(tops: readonly number[], sizes: readonly [number, number][], zoom: number, y: number): number {
  let lo = 0
  let hi = sizes.length
  while (lo < hi) {
    const mid = (lo + hi) >> 1
    if (tops[mid] + sizes[mid][1] * zoom <= y) lo = mid + 1
    else hi = mid
  }
  return lo
}

/** The 0-based page with the largest visible vertical slice in `[top, bottom]`. */
export function visiblePageIndex(tops: readonly number[], sizes: readonly [number, number][], zoom: number, top: number, bottom: number): number {
  if (!sizes.length) return 0
  const start = firstPageAtOrAfter(tops, sizes, zoom, top)
  let best = -1
  let bestVisible = 0
  for (let i = start; i < sizes.length; i++) {
    const pt = tops[i]
    if (pt >= bottom) break
    const pb = pt + sizes[i][1] * zoom
    const visible = Math.min(bottom, pb) - Math.max(top, pt)
    if (visible > bestVisible + 1 || (best < 0 && visible > 0)) {
      best = i
      bestVisible = visible
    }
  }
  if (best >= 0) return best
  const mid = (top + bottom) / 2
  const k = firstPageAtOrAfter(tops, sizes, zoom, mid)
  if (k === 0) return 0
  if (k >= sizes.length) return sizes.length - 1
  const prevDist = Math.max(0, mid - (tops[k - 1] + sizes[k - 1][1] * zoom))
  const curDist = Math.max(0, tops[k] - mid)
  return prevDist <= curDist ? k - 1 : k
}

interface Props {
  engine: WordEngine
  zoom: number
  /** The scrolling element (the viewport). */
  scroller: () => HTMLElement | null
}

export function Pages({ engine, zoom, scroller }: Props) {
  const mountRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const caretRef = useRef<HTMLDivElement>(null)
  const painted = useRef(new Map<number, string>())
  const pageContentEls = useRef(new Map<number, HTMLDivElement>())
  const [, setTick] = useState(0)
  const [overlayWin, setOverlayWin] = useState<[number, number]>([0, 16])
  const overlayWinRef = useRef<[number, number]>([0, 16])
  const composing = useRef(false)
  const scrollAfter = useRef(false)
  /** The viewport's scroll position and height, read on scroll and resize (never right after a
   * DOM change, which would make the browser lay out every page at once). */
  const view = useRef({ top: 0, height: 0 })
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null)
  const focusDim = useFocus(dimming)

  // Misspelled words get a wavy underline; right-clicking one offers corrections.
  useEffect(() => watchSpelling(engine), [engine])
  // Where everyone else is, while the document is shared.
  const places = usePlaces(engine)

  const sizes = engine.sizes
  const { tops, height, width } = useMemo(() => {
    const t: number[] = new Array(sizes.length)
    let y = TOP
    let maxW = 0
    for (let i = 0; i < sizes.length; i++) {
      const [w, h] = sizes[i]
      t[i] = y
      y += h * zoom + GAP
      if (w > maxW) maxW = w
    }
    return { tops: t, height: y + 48, width: maxW * zoom + SIDE * 2 }
  }, [sizes, zoom])
  // Pages are centered in the mount's actual width, which can exceed its minimum width.
  // CSS keeps overlays aligned during window and panel resizing without a layout read.
  const pageX = (i: number, x: number) => `calc(50% + ${(x - sizes[i][0] / 2) * zoom}px)`

  // Draw the pages near the screen; drop far ones.
  const draw = () => {
    const mount = mountRef.current
    if (!mount) return
    const { top: scrollTop, height: viewHeight } = view.current
    const top = scrollTop
    const bottom = top + (viewHeight || 1200)
    if (viewHeight > 0 && sizes.length > 0) {
      engine.setVisiblePage(visiblePageIndex(tops, sizes, zoom, top, bottom) + 1)
    }
    const pad = (viewHeight || 1200) * 0.5
    const startIdx = firstPageAtOrAfter(tops, sizes, zoom, top - pad)
    let first = -1
    let last = -1
    for (let i = startIdx; i < sizes.length; i++) {
      const pt = tops[i]
      if (pt > bottom + pad) break
      if (first < 0) first = i
      last = i
    }
    if (first < 0) return
    const sig = `${engine.revision}:${engine.latexVersion}`
    let paintedNow = false
    for (let i = first; i <= last; i++) {
      const el = pageContentEls.current.get(i)
      const was = painted.current.get(i)
      if (!el || was === sig) continue
      // A page already on screen is only sent again when its drawing changed.
      const p = engine.paint(i, was === undefined || !el.firstChild || was.split(':')[1] !== String(engine.latexVersion))
      if (p) {
        el.innerHTML = itemsHtml(p.items, engine.styles, engine.media.url)
        paintedNow = true
      }
      painted.current.set(i, sig)
    }
    if (paintedNow) engine.progressivePainted()
    for (const [i] of painted.current) {
      if (i < first - KEEP || i > last + KEEP) {
        const el = pageContentEls.current.get(i)
        if (el) el.innerHTML = ''
        painted.current.delete(i)
      }
    }
    const [ws, we] = overlayWinRef.current
    if (first - KEEP < ws || last + KEEP > we) {
      const next: [number, number] = [Math.max(0, first - KEEP * 2), last + KEEP * 2]
      overlayWinRef.current = next
      setOverlayWin(next)
    }
  }

  // Follow the engine: repaint what changed, move the caret.
  useEffect(
    () =>
      engine.subscribe(() => {
        setTick((t) => t + 1)
      }),
    [engine],
  )

  useLayoutEffect(() => {
    engine.input = inputRef.current
    engine.pagePoint = pagePoint
    engine.toClient = (page, x, y) => {
      const r = mountRef.current!.getBoundingClientRect()
      return { x: r.left + (r.width - sizes[page][0] * zoom) / 2 + x * zoom, y: r.top + (tops[page] ?? 0) + y * zoom }
    }
  }, [engine, sizes, tops, zoom])

  useLayoutEffect(() => {
    // Pages that no longer exist drop out of the bookkeeping.
    for (const [i] of painted.current) if (i >= sizes.length) painted.current.delete(i)
    draw()
    // overlayWin: frames that just came into the window mount empty and are drawn here.
  }, [engine.revision, engine.latexVersion, sizes, tops, zoom, overlayWin])

  useLayoutEffect(() => {
    placeInput()
    if (caretRef.current && caretRef.current.getAnimations) {
      for (const anim of caretRef.current.getAnimations()) anim.currentTime = 0
    }
    if (scrollAfter.current) {
      scrollAfter.current = false
      revealCaret()
    }
  }, [engine.tick, sizes, tops, zoom])

  // Listeners are added once and call the latest render's functions through these refs, so
  // a keystroke doesn't tear down and re-add them (and ResizeObserver's first call doesn't
  // trigger another draw).
  const drawRef = useRef(draw)
  drawRef.current = draw
  const scrollerRef = useRef(scroller)
  scrollerRef.current = scroller

  useEffect(() => {
    const sc = scrollerRef.current()
    if (!sc) return
    let frame = 0
    const measure = () => {
      view.current = { top: sc.scrollTop, height: sc.clientHeight }
    }
    measure()
    const onScroll = () => {
      measure()
      if (!frame) {
        frame = requestAnimationFrame(() => {
          frame = 0
          drawRef.current()
        })
      }
    }
    sc.addEventListener('scroll', onScroll, { passive: true })
    const ro = new ResizeObserver(onScroll)
    ro.observe(sc)
    return () => {
      cancelAnimationFrame(frame)
      sc.removeEventListener('scroll', onScroll)
      ro.disconnect()
    }
  }, [])

  // Pages drawn at another zoom are drawn again only if their content changed; the CSS scale
  // takes care of the rest.

  /** Where a client point is on a page: the page under it, or the nearest by height. */
  const pagePoint = (clientX: number, clientY: number) => {
    const mount = mountRef.current!
    const r = mount.getBoundingClientRect()
    const my = clientY - r.top
    let lo = 0
    let hi = sizes.length - 1
    while (lo < hi) {
      const mid = (lo + hi) >> 1
      if (my >= tops[mid] + sizes[mid][1] * zoom + GAP / 2) lo = mid + 1
      else hi = mid
    }
    const page = Math.max(0, lo)
    const left = (r.width - sizes[page][0] * zoom) / 2
    return { page, x: (clientX - r.left - left) / zoom, y: (my - tops[page]) / zoom }
  }

  /** The text field sits at the caret, so an input method's window opens there. */
  const placeInput = () => {
    const input = inputRef.current
    if (!input) return
    const c = engine.overlay.caret
    if (!c || !sizes[c[0]]) return
    const [page, x, cy, h] = c
    input.style.left = pageX(page, x)
    input.style.top = `${(tops[page] ?? 0) + cy * zoom}px`
    input.style.height = `${h * zoom}px`
    input.style.fontSize = `${Math.max(10, h * zoom * 0.75)}px`
  }

  /** Scrolls the caret into view. */
  const revealCaret = () => {
    const sc = scroller()
    const c = engine.overlay.caret ?? (engine.overlay.image ? [engine.overlay.image[0], engine.overlay.image[1], engine.overlay.image[2], engine.overlay.image[4]] : null)
    if (!sc || !c) return
    const [page, , cy, h] = c
    const top = (tops[page] ?? 0) + cy * zoom
    const bottom = top + h * zoom
    const { top: scrollTop, height } = view.current
    const typewriter = useFocus.getState().typewriter
    // Only scroll when the caret is out of view: setting the scroll position lays the page out.
    if (typewriter) {
      const want = top + h * zoom / 2 - height * 0.45
      if (Math.abs(want - scrollTop) > 2) sc.scrollTop = want
      return
    }
    if (top < scrollTop + 24) sc.scrollTop = top - 48
    else if (bottom > scrollTop + height - 72) sc.scrollTop = bottom - height + 96
  }

  const run = (cmd: Record<string, unknown> & { cmd: string }, reveal = true) => {
    scrollAfter.current = reveal
    return engine.run(cmd)
  }

  // ── Pointer
  const drag = useRef<{ page: number } | null>(null)
  const resize = useRef<{ corner: string; startX: number; startY: number; w: number; h: number; ratio: number } | null>(null)
  const onMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return
    const target = e.target as HTMLElement
    // ⌘-click (Ctrl-click on Windows) on a link opens it.
    const link = target.closest<HTMLElement>('.ln[data-link]')
    if (link && (isMac ? e.metaKey : e.ctrlKey)) {
      e.preventDefault()
      const href = link.dataset.link!
      if (!href.startsWith('#')) void openExternal(href)
      return
    }
    e.preventDefault()
    inputRef.current?.focus({ preventScroll: true })
    const handle = target.closest<HTMLElement>('.wp-handle')
    if (handle && engine.state.image) {
      const img = engine.state.image
      resize.current = { corner: handle.dataset.corner!, startX: e.clientX, startY: e.clientY, w: img.widthEmu, h: img.heightEmu, ratio: img.heightEmu / Math.max(1, img.widthEmu) }
      return
    }
    const at = pagePoint(e.clientX, e.clientY)
    const area = engine.doc.headerFooterAt(at.page, at.y)
    const note = engine.doc.noteAt(at.page, at.y) ?? null
    const editingNote = engine.state.note
    if (note != null && note !== editingNote) {
      run({ cmd: 'editNote', page: at.page, x: at.x, y: at.y }, false)
      drag.current = { page: at.page }
      return
    }
    if (editingNote != null && note == null) run({ cmd: 'exitHeaderFooter' }, false)
    else if (engine.state.headerFooter && !area) run({ cmd: 'exitHeaderFooter' }, false)
    else if (!engine.state.headerFooter && area && e.detail === 2 && engine.doc.hasHeaderFooter(at.page, area)) {
      run({ cmd: 'editHeaderFooter', page: at.page, which: area }, false)
      return
    }
    run({ cmd: 'click', page: at.page, x: at.x, y: at.y, extend: e.shiftKey, count: e.detail }, false)
    drag.current = { page: at.page }
  }
  const pointer = useRef({ zoom, pagePoint, run })
  pointer.current = { zoom, pagePoint, run }
  useEffect(() => {
    let moveRaf = 0
    let pendingMove: { clientX: number; clientY: number } | null = null
    const flushMove = () => {
      moveRaf = 0
      const pt = pendingMove
      pendingMove = null
      if (!pt) return
      const { zoom, pagePoint, run } = pointer.current
      if (resize.current) {
        const r = resize.current
        const dx = (pt.clientX - r.startX) / zoom
        const grow = r.corner.includes('w') ? -dx : dx
        const w = Math.max(9525 * 8, r.w + grow * 9525)
        // A picture keeps its proportions as a corner is dragged.
        const el = mountRef.current?.querySelector<HTMLElement>('.wp-image-frame')
        if (el) {
          el.style.width = `${(w / 9525) * zoom}px`
          el.style.height = `${((w * r.ratio) / 9525) * zoom}px`
        }
        return
      }
      if (!drag.current) return
      const at = pagePoint(pt.clientX, pt.clientY)
      run({ cmd: 'drag', page: at.page, x: at.x, y: at.y }, false)
      // Keep the pointer's end of the selection in view while dragging past the edge.
      const sc = scrollerRef.current()
      if (sc) {
        const r = sc.getBoundingClientRect()
        if (pt.clientY > r.bottom - 20) sc.scrollTop += 16
        else if (pt.clientY < r.top + 20) sc.scrollTop -= 16
      }
    }
    const move = (e: MouseEvent) => {
      if (!resize.current && (!drag.current || !(e.buttons & 1))) return
      pendingMove = { clientX: e.clientX, clientY: e.clientY }
      if (!moveRaf) moveRaf = requestAnimationFrame(flushMove)
    }
    const up = (e: MouseEvent) => {
      if (moveRaf) {
        cancelAnimationFrame(moveRaf)
        moveRaf = 0
      }
      pendingMove = null
      const { zoom, run } = pointer.current
      if (resize.current) {
        const r = resize.current
        resize.current = null
        const dx = (e.clientX - r.startX) / zoom
        const grow = r.corner.includes('w') ? -dx : dx
        const w = Math.max(9525 * 8, r.w + grow * 9525)
        if (Math.abs(w - r.w) > 100) run({ cmd: 'imageSize', widthEmu: Math.round(w), heightEmu: Math.round(w * r.ratio) }, false)
        else setTick((t) => t + 1)
      }
      drag.current = null
    }
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', up)
    return () => {
      if (moveRaf) cancelAnimationFrame(moveRaf)
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', up)
    }
  }, [])

  const onContextMenu = (e: React.MouseEvent) => {
    const mark = spellingAt(engine, e.clientX, e.clientY)
    if (!mark) return
    e.preventDefault()
    const { clientX: x, clientY: y } = e
    // The word is selected, as other apps do, so the correction replaces it.
    run({ cmd: 'select', anchor: [mark.path, mark.start], focus: [mark.path, mark.end] }, false)
    void spellingMenu(mark.word, (word) => {
      run({ cmd: 'select', anchor: [mark.path, mark.start], focus: [mark.path, mark.end] }, false)
      run({ cmd: 'insertText', text: word })
    }).then((items) => items.length && setMenu({ x, y, items }))
  }

  // Links show where they go, and a hand while ⌘ (Ctrl) is held.
  const onMouseOver = (e: React.MouseEvent) => {
    const link = (e.target as HTMLElement).closest<HTMLElement>('.ln[data-link]')
    if (link && !link.title) link.title = `${link.dataset.link}\n${shortcut('⌘Click')} to open`
  }

  // ── Keyboard and text
  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (composing.current || e.nativeEvent.isComposing) return
    const mod = isMac ? e.metaKey : e.ctrlKey
    const byWord = isMac ? e.altKey : e.ctrlKey
    const extend = e.shiftKey
    const move = (to: string) => run({ cmd: 'move', to, extend })
    switch (e.key) {
      case 'ArrowLeft':
        move(isMac && e.metaKey ? 'lineStart' : byWord ? 'wordLeft' : 'left')
        break
      case 'ArrowRight':
        move(isMac && e.metaKey ? 'lineEnd' : byWord ? 'wordRight' : 'right')
        break
      case 'ArrowUp':
        move(isMac && e.metaKey ? 'docStart' : 'up')
        break
      case 'ArrowDown':
        move(isMac && e.metaKey ? 'docEnd' : 'down')
        break
      case 'Home':
        move(e.ctrlKey || e.metaKey ? 'docStart' : 'lineStart')
        break
      case 'End':
        move(e.ctrlKey || e.metaKey ? 'docEnd' : 'lineEnd')
        break
      case 'PageUp':
        move('pageUp')
        break
      case 'PageDown':
        move('pageDown')
        break
      case 'Backspace':
        if (isMac && e.metaKey) {
          if (!engine.state.hasSelection) engine.run({ cmd: 'move', to: 'lineStart', extend: true })
          run({ cmd: 'backspace', word: false })
        } else run({ cmd: 'backspace', word: byWord })
        break
      case 'Delete':
        run({ cmd: 'delete', word: byWord })
        break
      case 'Enter':
        if (mod) run({ cmd: 'pageBreak' })
        else run({ cmd: e.shiftKey ? 'lineBreak' : 'enter' })
        break
      case 'Tab':
        if (mod || e.altKey) return
        run({ cmd: 'tab', shift: e.shiftKey })
        break
      case 'Escape':
        if (!engine.state.headerFooter && engine.state.note == null) return
        run({ cmd: 'exitHeaderFooter' })
        break
      default: {
        if (!mod || e.altKey) return
        const k = e.key.toLowerCase()
        if (k === 'a') run({ cmd: 'selectAll' }, false)
        else if (k === 'z') run({ cmd: e.shiftKey ? 'redo' : 'undo' })
        else if (k === 'y' && !isMac) run({ cmd: 'redo' })
        else if (k === 'b' && !e.shiftKey) run({ cmd: 'toggle', what: 'bold' }, false)
        else if (k === 'i' && !e.shiftKey) run({ cmd: 'toggle', what: 'italic' }, false)
        else if (k === 'u' && !e.shiftKey) run({ cmd: 'toggle', what: 'underline' }, false)
        else return
      }
    }
    e.preventDefault()
    e.stopPropagation()
  }

  const onBeforeInput = (e: React.FormEvent<HTMLTextAreaElement>) => {
    const ev = e.nativeEvent as InputEvent
    switch (ev.inputType) {
      case 'insertText':
      case 'insertReplacementText': {
        if (composing.current) return
        const text = ev.data ?? ev.dataTransfer?.getData('text/plain') ?? ''
        e.preventDefault()
        if (text) run({ cmd: 'insertText', text })
        break
      }
      case 'historyUndo':
        e.preventDefault()
        run({ cmd: 'undo' })
        break
      case 'historyRedo':
        e.preventDefault()
        run({ cmd: 'redo' })
        break
      case 'insertParagraph':
      case 'insertLineBreak':
        e.preventDefault()
        break
    }
  }

  const onCompositionStart = () => {
    composing.current = true
    inputRef.current?.classList.add('composing')
  }
  const onCompositionEnd = (e: React.CompositionEvent<HTMLTextAreaElement>) => {
    composing.current = false
    const input = inputRef.current!
    input.classList.remove('composing')
    const text = e.data || input.value
    input.value = ''
    if (text) run({ cmd: 'insertText', text })
  }
  const onInput = () => {
    // Anything that got into the field outside a composition was typed: take it as text.
    const input = inputRef.current!
    if (composing.current || !input.value) return
    const text = input.value
    input.value = ''
    run({ cmd: 'insertText', text })
  }

  const onCopy = (e: React.ClipboardEvent, cut: boolean) => {
    if (!engine.state.hasSelection && !engine.state.image) return
    e.preventDefault()
    const out = engine.run<{ text: string; json: string }>({ cmd: cut ? 'cut' : 'copy' })
    e.clipboardData.setData('text/plain', out.text)
    e.clipboardData.setData(WORD_CLIPBOARD, out.json)
  }
  const onPaste = async (e: React.ClipboardEvent) => {
    e.preventDefault()
    const files = Array.from(e.clipboardData.files).filter((f) => f.type.startsWith('image/'))
    if (files.length) {
      for (const f of files) await insertImageFile(engine, f)
      return
    }
    const json = e.clipboardData.getData(WORD_CLIPBOARD) || null
    run({ cmd: 'paste', text: e.clipboardData.getData('text/plain'), json })
  }

  // ── Overlay: caret, selection, marks and the picture frame, in one layer over the pages.
  const overlay = engine.overlay
  const place = (page: number, x: number, y: number, w: number, h: number) =>
    sizes[page] ? { left: pageX(page, x), top: tops[page] + y * zoom, width: w * zoom, height: h * zoom } : { display: 'none' }
  // Focus mode's dimming: every page veiled but for a band across the caret's paragraph (an empty
  // paragraph has no text boxes, so the caret's line). Shown by CSS only while dimming is on.
  const c = overlay.caret
  const [winStart, winEnd] = overlayWin
  const focusBoxes = focusDim
    ? overlay.paragraph?.length
      ? overlay.paragraph
      : c
        ? [[c[0], c[1], c[2], 1, c[3]] as const]
        : []
    : []
  const focusByPage = focusDim && focusBoxes.length ? new Map(focusBoxes.map((b) => [b[0], b])) : null
  // Everyone else's carets and selections; people whose carets are at one place share a marker.
  const remote = places.map((place) => ({ place, o: engine.remoteOverlay(place.at) }))
  const peerCarets = together(remote.flatMap(({ place, o }) => (o.caret && sizes[o.caret[0]] ? [{ place, key: o.caret.join(), item: o.caret }] : [])))

  return (
    <div className="docx-content-mount" ref={mountRef} style={{ height, minWidth: width }} onMouseDown={onMouseDown} onMouseOver={onMouseOver} onContextMenu={onContextMenu}>
      <div className="docx-pages">
        {/* Only the pages near the screen have frames: a 500-page document doesn't keep 500
            page boxes in the DOM. The window follows scrolling (see draw). */}
        {sizes.slice(winStart, winEnd + 1).map(([w, h], k) => {
          const i = winStart + k
          return <PageFrame key={i} index={i} width={w} height={h} top={tops[i]} zoom={zoom} els={pageContentEls.current} />
        })}
      </div>
      <div className="docx-overlay">
        {focusDim &&
          sizes.map(([w, h], i) => {
            if (i < winStart || i > winEnd) return null
            const band = focusByPage?.get(i)
            return (
              <div key={`f${i}`} className={`wp-focus ${band ? '' : 'veiled'}`} style={place(i, 0, 0, w, h)}>
                {band && <div className="wp-focus-band" style={{ top: (band[2] - 3) * zoom, height: (band[4] + 6) * zoom }} />}
              </div>
            )
          })}
        {overlay.marks?.map((m, k) => {
          if (m[0] < winStart || m[0] > winEnd) return null
          const style = engine.markStyles[m[5]]
          if (!style) return null
          if (style.squiggle) return <div key={`m${k}`} className="wp-mark wp-squiggle" style={place(m[0], m[1], m[2], m[3], m[4])} />
          const active = style.activeId !== undefined && style.activeId === m[6]
          return <div key={`m${k}`} className="wp-mark" style={{ ...place(m[0], m[1], m[2], m[3], m[4]), background: active ? (style.activeColor ?? style.color) : style.color }} />
        })}
        {/* A selected picture shows only its frame and handles, as in a presentation. Its line's
            selection band would reach above the picture by the line's spacing. */}
        {!overlay.image &&
          overlay.rects.map((r, k) => (r[0] >= winStart && r[0] <= winEnd ? <div key={k} className="wp-sel" style={place(r[0], r[1], r[2], r[3], r[4])} /> : null))}
        {remote.map(({ place: { peer }, o }) => o.rects.map((r, k) => <PeerSelection key={`${peer.client}:${k}`} peer={peer} style={place(r[0], r[1], r[2], r[3], r[4])} />))}
        {peerCarets.map(({ peers, moves, item: pc }) => (
          <PeerCaret key={peers[0].client} peers={peers} moves={moves} style={{ ...place(pc[0], pc[1], pc[2], 0, pc[3]), width: undefined }} />
        ))}
        {overlay.caret && !engine.state.hasSelection && !overlay.image && (
          <div ref={caretRef} className="wp-caret" style={{ ...place(overlay.caret[0], overlay.caret[1], overlay.caret[2], 0, overlay.caret[3]), width: undefined }} />
        )}
      </div>
      {overlay.image && sizes[overlay.image[0]] && (
        <div className="wp-image-frame" style={place(overlay.image[0], overlay.image[1], overlay.image[2], overlay.image[3], overlay.image[4])}>
          {['nw', 'ne', 'sw', 'se'].map((c) => (
            <div key={c} className={`wp-handle ${c}`} data-corner={c} />
          ))}
        </div>
      )}
      <textarea
        ref={inputRef}
        className="wp-input"
        data-engine-input=""
        aria-label="Document"
        autoCapitalize="off"
        autoComplete="off"
        autoCorrect="off"
        spellCheck={false}
        onKeyDown={onKeyDown}
        onBeforeInput={onBeforeInput}
        onInput={onInput}
        onCompositionStart={onCompositionStart}
        onCompositionEnd={onCompositionEnd}
        onCopy={(e) => onCopy(e, false)}
        onCut={(e) => onCopy(e, true)}
        onPaste={(e) => void onPaste(e)}
      />
      {menu && (
        <PopoverMenu
          anchor={menu}
          items={menu.items}
          onClose={() => {
            setMenu(null)
            engine.focus()
          }}
        />
      )}
    </div>
  )
}

/** One page's frame. Its drawing goes in by hand (see `draw`), so it only re-renders when the
 * page moves or changes size. */
const PageFrame = memo(function PageFrame({ index, width, height, top, zoom, els }: { index: number; width: number; height: number; top: number; zoom: number; els: Map<number, HTMLDivElement> }) {
  return (
    <div
      className="docx-page"
      data-page-index={index}
      style={{ top, width: width * zoom, height: height * zoom, left: '50%', marginLeft: -(width * zoom) / 2 }}
    >
      <div className="docx-page-scale" style={{ width, height, transform: `scale(${zoom})` }}>
        <div
          className="docx-page-content"
          ref={(el) => {
            if (el) els.set(index, el)
            else els.delete(index)
          }}
        />
      </div>
    </div>
  )
})

/**
 * Reads a picture file and puts it in at the caret. One far wider than twice the text (a phone
 * photo) is scaled down to that, as on a slide: it still shows sharp on any screen, and it's a
 * fraction of the bytes to save, or to send to everyone in a shared document.
 */
export async function insertImageFile(engine: WordEngine, file: File) {
  const bytes = new Uint8Array(await file.arrayBuffer())
  const type = file.type || 'image/png'
  const setup = JSON.parse(engine.doc.pageSetup()) as { pageWidthTwips: number; marginsTwips: { left: number; right: number } }
  // 15 twips to a pixel.
  const textWidth = (setup.pageWidthTwips - setup.marginsTwips.left - setup.marginsTwips.right) / 15
  const pic = (await shrinkPicture(bytes, type, 2 * (textWidth > 0 ? textWidth : 624), Infinity)) ?? { bytes, type, width: 400, height: 300 }
  engine.doc.insertImage(pic.bytes, pic.type, pic.width, pic.height)
  engine.refresh()
}
