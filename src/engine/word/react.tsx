// React bindings for the Word engine: a root that opens the document, the scrolling page view,
// and hooks for the toolbar and inspector (formatting, styles, zoom, find, links, headers and
// footers, page setup, the outline).

import type { ReactNode } from 'react'
import { createContext, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { CenterState } from '../../components/ui'
import { pickImages } from '../../lib/platform'
import type { WordEngine, WordState } from './engine'
import { WordEngine as Engine } from './engine'
import { insertImageFile, Pages } from './Pages'
import './pages.css'

// ── Zoom

const ZOOM_STEPS = [0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4]

class ZoomStore {
  zoom = 1
  auto = true
  /** The width of the view, for fitting the page. */
  viewWidth = 0
  pageWidth = 794
  private listeners = new Set<() => void>()
  subscribe = (f: () => void) => {
    this.listeners.add(f)
    return () => void this.listeners.delete(f)
  }
  get = () => this.zoom
  private emit() {
    this.listeners.forEach((f) => f())
  }
  /** The zoom that fits the page across the view, never above 100%. */
  fit() {
    if (!this.viewWidth) return 1
    return Math.max(0.25, Math.min(1, (this.viewWidth - 48) / this.pageWidth))
  }
  set(z: number) {
    this.auto = false
    this.zoom = Math.min(4, Math.max(0.25, z))
    this.emit()
  }
  setAuto() {
    this.auto = true
    this.zoom = this.fit()
    this.emit()
  }
  resize(viewWidth: number, pageWidth: number) {
    this.viewWidth = viewWidth
    this.pageWidth = pageWidth || 794
    if (this.auto) {
      const z = this.fit()
      if (Math.abs(z - this.zoom) > 0.001) {
        this.zoom = z
        this.emit()
      }
    }
  }
}

// ── Find

class SearchStore {
  query = ''
  count = 0
  activeIndex = -1
  matchCase = false
  wholeWord = false
  private listeners = new Set<() => void>()
  private version = 0
  constructor(private engine: () => WordEngine | null, private viewport: () => HTMLElement | null) {}
  subscribe = (f: () => void) => {
    this.listeners.add(f)
    return () => void this.listeners.delete(f)
  }
  getVersion = () => this.version
  private emit() {
    this.version++
    this.listeners.forEach((f) => f())
  }
  setQuery(q: string) {
    this.query = q
    this.find(q, { matchCase: this.matchCase, wholeWord: this.wholeWord })
    if (this.count) this.goTo(this.activeIndex < 0 ? 0 : this.activeIndex)
  }
  /** Searches again; answers the matches (an array as long as the count, for callers that index it). */
  find(q: string, opts: { matchCase?: boolean; wholeWord?: boolean } = {}) {
    const e = this.engine()
    if (!e || !q) {
      this.count = 0
      this.activeIndex = -1
      e?.run({ cmd: 'clearFind' })
      this.emit()
      return []
    }
    const r = e.run<{ count: number; first: number }>({ cmd: 'find', query: q, matchCase: !!opts.matchCase, wholeWord: !!opts.wholeWord })
    this.count = r.count
    this.activeIndex = r.count ? Math.min(r.first, r.count - 1) : -1
    this.emit()
    return Array.from({ length: r.count }, (_, i) => i)
  }
  goTo(i: number) {
    const e = this.engine()
    if (!e || !this.count) return
    this.activeIndex = ((i % this.count) + this.count) % this.count
    e.run({ cmd: 'selectMatch', index: this.activeIndex })
    // Use layout coordinates: an offscreen page may not have any painted text yet.
    const sc = this.viewport()
    const rect = e.overlay.rects[0]
    if (sc && rect && e.toClient) {
      const [page, x, y, width, height] = rect
      const start = e.toClient(page, x, y)
      const end = e.toClient(page, x + width, y + height)
      const view = sc.getBoundingClientRect()
      const top = view.top + sc.clientTop + 24
      const bottom = view.top + sc.clientTop + sc.clientHeight - 24
      const left = view.left + sc.clientLeft + 24
      const right = view.left + sc.clientLeft + sc.clientWidth - 24
      if (start.y < top) sc.scrollTop += start.y - top
      else if (end.y > bottom) sc.scrollTop += Math.min(end.y - bottom, start.y - top)
      if (start.x < left) sc.scrollLeft += start.x - left
      else if (end.x > right) sc.scrollLeft += Math.min(end.x - right, start.x - left)
    }
    this.emit()
  }
  next() {
    this.goTo(this.activeIndex + 1)
  }
  previous() {
    this.goTo(this.activeIndex - 1)
  }
  clear() {
    this.query = ''
    this.count = 0
    this.activeIndex = -1
    this.engine()?.run({ cmd: 'clearFind' })
    this.emit()
  }
  replaceCurrent(text: string) {
    const e = this.engine()
    if (!e || !this.count) return
    const index = Math.max(0, this.activeIndex)
    e.run({ cmd: 'replaceMatch', index, text })
    // Search again from the same place: the replaced match is gone, the next one takes its index.
    this.find(this.query, { matchCase: this.matchCase, wholeWord: this.wholeWord })
    if (this.count) this.goTo(Math.min(index, this.count - 1))
  }
  replaceAll(text: string) {
    const e = this.engine()
    if (!e || !this.count) return
    e.run({ cmd: 'replaceAll', text })
    this.find(this.query, { matchCase: this.matchCase, wholeWord: this.wholeWord })
  }
}

// ── The link popover's state

export interface LinkPopupState {
  mode: 'closed' | 'editing' | 'preview'
  url: string
}

class LinkStore {
  state: LinkPopupState = { mode: 'closed', url: '' }
  private listeners = new Set<() => void>()
  constructor(private engine: () => WordEngine | null) {}
  subscribe = (f: () => void) => {
    this.listeners.add(f)
    return () => void this.listeners.delete(f)
  }
  get = () => this.state
  private set(s: LinkPopupState) {
    this.state = s
    this.listeners.forEach((f) => f())
  }
  openAtCaret = () => {
    const e = this.engine()
    if (!e) return
    const url = e.state.link
    this.set(url ? { mode: 'preview', url } : { mode: 'editing', url: '' })
  }
  edit = () => this.set({ ...this.state, mode: 'editing' })
  setUrl = (url: string) => this.set({ ...this.state, url })
  close = () => this.set({ mode: 'closed', url: '' })
  /** Applies the address being edited to the selected text; answers whether it did. */
  commitEdit = (url = this.state.url) => {
    const e = this.engine()
    const u = url.trim()
    if (!e || !u) return false
    if (!e.state.hasSelection && !e.state.link) return false
    e.run({ cmd: 'setLink', url: u })
    this.close()
    return true
  }
  remove = () => {
    this.engine()?.run({ cmd: 'removeLink' })
    this.close()
  }
}

// ── Root and context

interface Ctx {
  engine: WordEngine | null
  error: string | null
  zoom: ZoomStore
  search: SearchStore
  link: LinkStore
  popups: { hyperlink?: () => ReactNode }
  viewport: React.MutableRefObject<HTMLDivElement | null>
}

const WordCtx = createContext<Ctx | null>(null)

function useCtx() {
  const c = useContext(WordCtx)
  if (!c) throw new Error('Word hooks need a DocxEditor.Root')
  return c
}

export interface ChangeEvent {
  source: 'load' | 'edit'
  revision: number
}

function Root({
  document,
  popups,
  onReady,
  onChange,
  onError,
  children,
}: {
  document: Uint8Array | null
  popups?: { hyperlink?: () => ReactNode }
  onReady?: (engine: WordEngine) => void
  onChange?: (change: ChangeEvent) => void
  onError?: (error: unknown) => void
  children: ReactNode
}) {
  const [engine, setEngine] = useState<WordEngine | null>(null)
  const [error, setError] = useState<string | null>(null)
  const engineRef = useRef<WordEngine | null>(null)
  const viewport = useRef<HTMLDivElement | null>(null)
  const stores = useMemo(() => {
    const get = () => engineRef.current
    return { zoom: new ZoomStore(), search: new SearchStore(get, () => viewport.current), link: new LinkStore(get) }
  }, [])
  const ready = useRef(onReady)
  ready.current = onReady
  const changed = useRef(onChange)
  changed.current = onChange
  const failed = useRef(onError)
  failed.current = onError

  useEffect(() => {
    let live = true
    let opened: WordEngine | null = null
    setEngine(null)
    setError(null)
    Engine.open(document)
      .then((e) => {
        if (!live) return e.dispose()
        opened = e
        engineRef.current = e
        setEngine(e)
        ready.current?.(e)
        changed.current?.({ source: 'load', revision: e.state.dirty ? -1 : 0 })
      })
      .catch((error) => {
        if (!live) return
        setError(error instanceof Error ? error.message : String(error))
        failed.current?.(error)
      })
    return () => {
      live = false
      engineRef.current = null
      opened?.dispose()
    }
  }, [document])

  // Unsaved changes follow the engine's history.
  useEffect(() => {
    if (!engine) return
    let dirty = engine.state.dirty
    return engine.subscribe(() => {
      if (engine.state.dirty !== dirty) {
        dirty = engine.state.dirty
        changed.current?.({ source: 'edit', revision: dirty ? 1 : 0 })
      }
    })
  }, [engine])

  const ctx = useMemo<Ctx>(() => ({ engine, error, ...stores, popups: popups ?? {}, viewport }), [engine, error, stores, popups])
  return <WordCtx.Provider value={ctx}>{children}</WordCtx.Provider>
}

function Viewport({ className, children }: { className?: string; children: ReactNode }) {
  const { viewport, zoom, engine } = useCtx()
  useLayoutEffect(() => {
    const el = viewport.current
    if (!el) return
    const update = () => zoom.resize(el.clientWidth, Math.max(...(engine?.sizes.map((s) => s[0]) ?? [794])))
    update()
    const ro = new ResizeObserver(update)
    ro.observe(el)
    return () => ro.disconnect()
  }, [viewport, zoom, engine])
  return (
    <div className={className} ref={viewport} style={{ overflow: 'auto', position: 'relative' }}>
      {children}
    </div>
  )
}

function Content() {
  const { engine, error, zoom, viewport, popups } = useCtx()
  const z = useSyncExternalStore(zoom.subscribe, zoom.get)
  if (error !== null) {
    return (
      <div role="alert" style={{ position: 'absolute', inset: 0 }}>
        <CenterState title="Couldn't open this document">
          <p className="muted">{error || 'The file could not be read. Try opening another DOCX file.'}</p>
        </CenterState>
      </div>
    )
  }
  if (!engine) return <CenterState spinner />
  return (
    <>
      <Pages engine={engine} zoom={z} scroller={() => viewport.current} />
      {popups.hyperlink?.()}
    </>
  )
}

/** The link popover: an address field while editing, the address with edit and remove buttons for a link. */
function HyperLink() {
  const { link } = useCtx()
  const state = useSyncExternalStore(link.subscribe, link.get)
  const input = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (state.mode === 'editing') {
      input.current?.focus({ preventScroll: true })
      input.current?.select()
    }
  }, [state.mode])
  if (state.mode === 'closed') return null
  return (
    <div className="docx-hyperlink-popup" data-mode={state.mode} role="dialog" aria-label="Link">
      {state.mode === 'editing' ? (
        <>
          <div className="docx-hyperlink-popup__fields">
            <input
              ref={input}
              className="docx-hyperlink-popup__input"
              value={state.url}
              placeholder="Paste or type a link"
              onChange={(e) => link.setUrl(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') {
                  e.preventDefault()
                  link.close()
                }
              }}
            />
          </div>
          <div className="docx-hyperlink-popup__actions">
            <button type="button" className="docx-hyperlink-popup__cancel" onClick={link.close}>
              Cancel
            </button>
            <button type="button" className="docx-hyperlink-popup__apply" disabled={!state.url.trim()}>
              Apply
            </button>
          </div>
        </>
      ) : (
        <>
          <a className="docx-hyperlink-popup__url" href={state.url} onClick={(e) => e.preventDefault()}>
            {state.url}
          </a>
          <div className="docx-hyperlink-popup__actions">
            <button type="button" className="docx-hyperlink-popup__action" title="Edit link" aria-label="Edit link" onClick={link.edit}>
              ✎
            </button>
            <button type="button" className="docx-hyperlink-popup__action" title="Remove link" aria-label="Remove link" onClick={link.remove}>
              ✕
            </button>
          </div>
        </>
      )}
    </div>
  )
}

export const DocxEditor = { Root, Viewport, Content, HyperLink }

// ── Hooks

export function useDocxEditor(): WordEngine | null {
  return useCtx().engine
}

const NO_SUB = () => () => {}

/** Equal, or objects whose own values are all equal (one level deep). */
function shallowEqual(a: unknown, b: unknown) {
  if (Object.is(a, b)) return true
  if (!a || !b || typeof a !== 'object' || typeof b !== 'object') return false
  const ka = Object.keys(a)
  if (ka.length !== Object.keys(b).length) return false
  return ka.every((k) => Object.is((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]))
}

/**
 * Part of the engine's state, kept current: the component re-renders only when what `pick`
 * returns changes (compared one level deep), not on every keystroke.
 */
export function useWordSelector<T>(pick: (engine: WordEngine) => T, fallback: T): T {
  const { engine } = useCtx()
  const last = useRef<{ tick: number; engine: WordEngine | null; value: T } | null>(null)
  const pickRef = useRef(pick)
  pickRef.current = pick
  const snapshot = () => {
    if (!engine) return fallback
    const prev = last.current
    if (prev && prev.engine === engine && prev.tick === engine.tick) return prev.value
    const value = pickRef.current(engine)
    if (prev && prev.engine === engine && shallowEqual(prev.value, value)) {
      last.current = { tick: engine.tick, engine, value: prev.value }
      return prev.value
    }
    last.current = { tick: engine.tick, engine, value }
    return value
  }
  return useSyncExternalStore(engine?.subscribe ?? NO_SUB, snapshot)
}

/** The engine's whole state, re-rendering on every change. */
export function useWordState(): WordState | null {
  return useWordSelector((e) => ({ ...e.state, tick: e.tick }), null)
}

export function useEditorState<T>(pick: (s: { page: WordState['page'] | null; selection: number; image: WordState['image']; table: WordState['table'] }) => T): T {
  return useWordSelector((e) => pick({ page: e.state.page, selection: e.tick, image: e.state.image, table: e.state.table }), pick({ page: null, selection: 0, image: null, table: null }))
}

/** Runs `f` after each change to the document (not caret moves). */
export function useEditorEvent(_: 'change', f: () => void) {
  const { engine } = useCtx()
  const fn = useRef(f)
  fn.current = f
  useEffect(() => {
    if (!engine) return
    let revision = engine.revision
    return engine.subscribe(() => {
      if (engine.revision !== revision) {
        revision = engine.revision
        fn.current()
      }
    })
  }, [engine])
}

export function useZoom() {
  const { zoom } = useCtx()
  const z = useSyncExternalStore(zoom.subscribe, zoom.get)
  return {
    zoom: z,
    setZoom: (v: number) => zoom.set(v),
    zoomIn: () => zoom.set(ZOOM_STEPS.find((s) => s > z + 0.001) ?? z),
    zoomOut: () => zoom.set([...ZOOM_STEPS].reverse().find((s) => s < z - 0.001) ?? z),
    auto: () => zoom.setAuto(),
    canZoomIn: z < 4 - 0.001,
    canZoomOut: z > 0.25 + 0.001,
  }
}

export function useDocumentSearch() {
  const { search } = useCtx()
  useSyncExternalStore(search.subscribe, search.getVersion)
  return {
    query: search.query,
    matches: Array.from({ length: search.count }, (_, i) => i),
    activeIndex: search.activeIndex,
    matchCase: search.matchCase,
    wholeWord: search.wholeWord,
    setQuery: (q: string) => search.setQuery(q),
    next: () => search.next(),
    previous: () => search.previous(),
    goTo: (i: number) => search.goTo(i),
    clear: () => search.clear(),
    find: (q: string, opts?: { matchCase?: boolean; wholeWord?: boolean }) => search.find(q, opts),
    replaceCurrent: (text: string) => search.replaceCurrent(text),
    replaceAll: (text: string) => search.replaceAll(text),
  }
}

export function useHyperlinkPopup() {
  const { link } = useCtx()
  const state = useSyncExternalStore(link.subscribe, link.get)
  return { state, openAtCaret: link.openAtCaret, setUrl: link.setUrl, commitEdit: link.commitEdit, close: link.close }
}

type Slot =
  | 'history.undo'
  | 'history.redo'
  | 'text.bold'
  | 'text.italic'
  | 'text.underline'
  | 'text.strike'
  | 'script.super'
  | 'script.sub'
  | 'format.clear'
  | 'list.bullet'
  | 'list.numbered'
  | 'list.indent'
  | 'list.outdent'
  | 'alignment.left'
  | 'alignment.center'
  | 'alignment.right'
  | 'alignment.justify'
  | 'direction.ltr'
  | 'direction.rtl'
  | 'insert.pageBreak'

/** What a toolbar button for one of the editor's commands needs. */
export function slotCommand(engine: WordEngine | null, slot: Slot) {
  const s = engine?.state
  const run = (cmd: Record<string, unknown> & { cmd: string }) => () => {
    engine?.run(cmd)
  }
  const inBody = !s?.headerFooter
  switch (slot) {
    case 'history.undo':
      return { isActive: false, isEnabled: !!s?.canUndo, execute: run({ cmd: 'undo' }) }
    case 'history.redo':
      return { isActive: false, isEnabled: !!s?.canRedo, execute: run({ cmd: 'redo' }) }
    case 'text.bold':
      return { isActive: !!s?.bold, isEnabled: !!s, execute: run({ cmd: 'toggle', what: 'bold' }) }
    case 'text.italic':
      return { isActive: !!s?.italic, isEnabled: !!s, execute: run({ cmd: 'toggle', what: 'italic' }) }
    case 'text.underline':
      return { isActive: !!s?.underline, isEnabled: !!s, execute: run({ cmd: 'toggle', what: 'underline' }) }
    case 'text.strike':
      return { isActive: !!s?.strike, isEnabled: !!s, execute: run({ cmd: 'toggle', what: 'strike' }) }
    case 'script.super':
      return { isActive: !!s?.superscript, isEnabled: !!s, execute: run({ cmd: 'toggle', what: 'superscript' }) }
    case 'script.sub':
      return { isActive: !!s?.subscript, isEnabled: !!s, execute: run({ cmd: 'toggle', what: 'subscript' }) }
    case 'format.clear':
      return { isActive: false, isEnabled: !!s, execute: run({ cmd: 'clearFormatting' }) }
    case 'list.bullet':
      return { isActive: s?.list === 'bullet', isEnabled: !!s && inBody, execute: run({ cmd: 'list', kind: 'bullet' }) }
    case 'list.numbered':
      return { isActive: s?.list === 'numbered', isEnabled: !!s && inBody, execute: run({ cmd: 'list', kind: 'numbered' }) }
    case 'list.indent':
      return { isActive: false, isEnabled: !!s, execute: run({ cmd: 'indent', delta: 1 }) }
    case 'list.outdent':
      return { isActive: false, isEnabled: !!s, execute: run({ cmd: 'indent', delta: -1 }) }
    case 'alignment.left':
    case 'alignment.center':
    case 'alignment.right':
    case 'alignment.justify': {
      const to = slot.split('.')[1]
      return { isActive: s?.align === to, isEnabled: !!s, execute: run({ cmd: 'align', to }) }
    }
    case 'direction.ltr':
    case 'direction.rtl': {
      const rtl = slot === 'direction.rtl'
      return { isActive: !!s && s.rtl === rtl, isEnabled: !!s, execute: run({ cmd: 'direction', rtl }) }
    }
    case 'insert.pageBreak':
      return { isActive: false, isEnabled: !!s && inBody, execute: run({ cmd: 'pageBreak' }) }
  }
}

export function useEditorCommand(slot: Slot) {
  const engine = useDocxEditor()
  useWordSelector((e) => {
    const c = slotCommand(e, slot)
    return `${c.isActive}|${c.isEnabled}`
  }, '')
  return slotCommand(engine, slot)
}

const LINE_SPACINGS = [1, 1.15, 1.5, 2]

interface ValueCommand<T> {
  value: T | null
  isEnabled: boolean
  options: number[]
  execute: (v: T) => void
}

export function useEditorValueCommand(slot: 'font.size' | 'list.lineSpacing'): ValueCommand<number>
export function useEditorValueCommand(slot: 'text.color' | 'text.highlight'): ValueCommand<string>
export function useEditorValueCommand(slot: 'font.size' | 'text.color' | 'text.highlight' | 'list.lineSpacing'): ValueCommand<number> | ValueCommand<string> {
  const engine = useDocxEditor()
  const s = useWordSelector((e) => ({ size: e.state.size, lineSpacing: e.state.lineSpacing, color: e.state.color, highlight: e.state.highlight }), null)
  const isEnabled = !!s
  switch (slot) {
    case 'font.size':
      return { value: s?.size ?? null, isEnabled, options: [], execute: (v: number) => void engine?.run({ cmd: 'size', halfPoints: v }) }
    case 'list.lineSpacing':
      return { value: s?.lineSpacing ?? null, isEnabled, options: LINE_SPACINGS, execute: (v: number) => void engine?.run({ cmd: 'lineSpacing', value: v }) }
    case 'text.color':
      return { value: s?.color ? `#${s.color}` : null, isEnabled, options: [], execute: (hex: string) => void engine?.run({ cmd: 'color', hex: hex.replace(/^#/, '') }) }
    case 'text.highlight':
      return { value: s?.highlight ?? null, isEnabled, options: [], execute: (name: string) => void engine?.run({ cmd: 'highlight', name }) }
  }
}

export function useParagraphStyle() {
  const engine = useDocxEditor()
  const style = useWordSelector((e) => e.state.style, null)
  const options = useMemo(() => (engine ? (JSON.parse(engine.doc.styleOptions()) as { styleId: string; name: string }[]) : []), [engine])
  return { value: style, options, isEnabled: !!engine, setValue: (id: string) => engine?.run({ cmd: 'style', id }) }
}

/** Fonts every Mac and Windows computer has, or the app brings an equivalent of. */
const COMMON_FONTS = ['Calibri', 'Cambria', 'Arial', 'Times New Roman', 'Courier New', 'Georgia', 'Helvetica', 'Verdana', 'Trebuchet MS', 'Garamond', 'Aptos']

export function useFontFamily(extras: string[] = []) {
  const engine = useDocxEditor()
  const font = useWordSelector((e) => e.state.font, null)
  const options = useMemo(() => {
    const doc = engine ? (JSON.parse(engine.doc.documentFonts()) as string[]) : []
    return [...new Set([...COMMON_FONTS, ...doc, ...extras])]
  }, [engine, extras])
  return { value: font, options, isEnabled: !!engine, setValue: (family: string) => engine?.run({ cmd: 'font', family }) }
}

export function useHeaderFooterState() {
  const editing = useWordSelector((e) => e.state.headerFooter, null)
  const engine = useDocxEditor()
  return engine ? { editing } : null
}

export interface PageSetup {
  pageWidthTwips: number
  pageHeightTwips: number
  orientation: 'portrait' | 'landscape'
  marginsTwips: { top: number; bottom: number; left: number; right: number }
}

export function usePageSetup() {
  const engine = useDocxEditor()
  // Page setup only changes with the document (or the caret moving to another section).
  const pageSetup = useWordSelector((e) => e.doc.pageSetup(), null)
  const s = useWordSelector((e) => ({ headerFooter: e.state.headerFooter }), null)
  const apply = (p: { pageWidthTwips?: number; pageHeightTwips?: number; orientation?: 'portrait' | 'landscape'; marginTopTwips?: number; marginBottomTwips?: number; marginLeftTwips?: number; marginRightTwips?: number; scope?: string }) =>
    engine?.run({
      cmd: 'pageSetup',
      widthTwips: p.pageWidthTwips ?? null,
      heightTwips: p.pageHeightTwips ?? null,
      landscape: p.orientation ? p.orientation === 'landscape' : null,
      top: p.marginTopTwips ?? null,
      bottom: p.marginBottomTwips ?? null,
      left: p.marginLeftTwips ?? null,
      right: p.marginRightTwips ?? null,
    })
  return { pageSetup: pageSetup ? (JSON.parse(pageSetup) as PageSetup) : null, apply, isEnabled: !!s && !s.headerFooter }
}

export function useDocumentOutline() {
  const engine = useDocxEditor()
  const [items, setItems] = useState<{ heading: { blockId: string; text: string }; depth: number }[]>([])
  const read = () => {
    if (!engine) return
    const out = JSON.parse(engine.doc.outline()) as { text: string; depth: number; path: number[] }[]
    setItems(out.map((o) => ({ heading: { blockId: o.path.join('.'), text: o.text }, depth: o.depth })))
  }
  useEffect(read, [engine])
  const timer = useRef(0)
  useEditorEvent('change', () => {
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(read, 250)
  })
  useEffect(() => () => window.clearTimeout(timer.current), [])
  // The heading the caret is under.
  const selectedBlockId = useWordSelector((e) => {
    if (!items.length) return null
    const sel = JSON.parse(e.doc.selection()) as { focus: [number[], number]; body: boolean }
    if (!sel.body) return null
    const top = sel.focus[0][0]
    let best: string | null = null
    for (const it of items) if (Number(it.heading.blockId.split('.')[0]) <= top) best = it.heading.blockId
    return best
  }, null)
  return {
    items,
    isEmpty: !items.length,
    selectedBlockId,
    goTo: (blockId: string) => {
      engine?.run({ cmd: 'goTo', path: blockId.split('.').map(Number) })
      engine?.focus?.()
    },
  }
}

/** A button that asks for pictures and puts them in at the caret. */
export function ImageInsertTrigger({ className, children }: { className?: string; children: ReactNode }) {
  const engine = useDocxEditor()
  return (
    <button
      type="button"
      className={`${className ?? ''} docx-toolbar__button`}
      disabled={!engine}
      onMouseDown={(e) => e.preventDefault()}
      onClick={async () => {
        if (!engine) return
        const files = await pickImages({ multiple: true })
        for (const f of files) await insertImageFile(engine, f)
        engine.focus?.()
      }}
    >
      {children}
    </button>
  )
}
