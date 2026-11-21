// A saved version, shown as the document itself and read only: Word pages, slides, a workbook's
// sheets, a PDF, Markdown as it reads, text as it is, a picture. Each is drawn by the same engine
// as its editor, from the version's bytes, and nothing it shows is saved anywhere.

import { Fragment, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react'
import { renderMarkdown } from '../editors/markdown/render'
import { openExternal } from '../lib/platform'
import { decodeText } from '../lib/textcodec'
import '../editors/markdown/markdown.css'
import type { PdfView } from '../plugins/api'
import { itemsHtml } from '../engine/paint'
import { Grid } from '../engine/sheets/Grid'
import { SheetsEngine } from '../engine/sheets/engine'
import { SlideImage } from '../engine/slides/SlideImage'
import { SlidesEngine } from '../engine/slides/engine'
import { WordEngine } from '../engine/word/engine'
import '../engine/word/pages.css'
import { KindIcon } from './ui'
import { KINDS } from '../lib/kinds'

interface Props {
  kind: string
  bytes: Uint8Array
  /** The file's name, for its extension (a CSV file opens as one). */
  name: string
  /** Called with the bytes once they're on screen, or couldn't be shown. */
  onShown: (bytes: Uint8Array) => void
}

const TEXT = new Set(['txt', 'latex'])
const PICTURE_TYPES: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg' }

export default function VersionPreview({ kind, bytes, name, onShown }: Props) {
  if (kind === 'docx') return <Opened bytes={bytes} open={() => WordEngine.open(bytes)} onShown={onShown} kind={kind} view={(e) => <WordPages engine={e} />} />
  if (kind === 'pptx') return <Opened bytes={bytes} open={() => SlidesEngine.open(bytes)} onShown={onShown} kind={kind} view={(e) => <Slides engine={e} />} />
  if (kind === 'xlsx' || kind === 'csv') {
    const open = async () => {
      const e = await SheetsEngine.open(bytes, name)
      e.readOnly = true
      return e
    }
    return <Opened bytes={bytes} open={open} onShown={onShown} kind={kind} view={(e) => <Sheets engine={e} />} />
  }
  if (kind === 'pdf') return <Pdf bytes={bytes} onShown={onShown} />
  if (kind === 'md') return <Markdown bytes={bytes} onShown={onShown} />
  if (TEXT.has(kind)) return <Text bytes={bytes} mono={kind === 'latex'} onShown={onShown} />
  const type = PICTURE_TYPES[name.split('.').pop()?.toLowerCase() ?? '']
  if (kind === 'image' && type) return <Picture bytes={bytes} type={type} onShown={onShown} />
  return <CantShow kind={kind} bytes={bytes} onShown={onShown} />
}

/** For files Writan can't draw here (HEIC pictures, a damaged file). */
function CantShow({ kind, bytes, onShown }: { kind: string; bytes: Uint8Array; onShown: (bytes: Uint8Array) => void }) {
  useEffect(() => onShown(bytes), [bytes, onShown])
  return (
    <div className="vh-center">
      <div className="vh-note">
        <span className="vh-note-icon">
          <KindIcon kind={kind} size={22} />
        </span>
        <p>{KINDS[kind]?.label ?? 'This file'} versions can’t be shown here. Open one as a copy to look at it, or restore it.</p>
      </div>
    </div>
  )
}

const ids = new WeakMap<object, number>()
let lastId = 0
const idOf = (o: object) => {
  let id = ids.get(o)
  if (id === undefined) ids.set(o, (id = ++lastId))
  return id
}

/**
 * An engine opened on a version's bytes. While the next version opens, the last one stays up, so
 * scrubbing the timeline doesn't flash; each engine is freed once its version is off screen.
 */
function Opened<T extends { dispose(): void }>({ bytes, open, view, onShown, kind }: { bytes: Uint8Array; open: () => Promise<T>; view: (engine: T) => React.ReactNode; onShown: (bytes: Uint8Array) => void; kind: string }) {
  // The engine on screen and the bytes it opened, or the bytes that couldn't be opened.
  const [state, setState] = useState<{ engine: T | null; bytes: Uint8Array | null; failed: boolean }>({ engine: null, bytes: null, failed: false })
  const current = useRef<T | null>(null)
  const retired = useRef(new Set<T>())
  const shown = useRef(onShown)
  shown.current = onShown

  useEffect(() => {
    let live = true
    let mine: T | null = null
    open().then(
      (engine) => {
        if (!live) return engine.dispose()
        mine = engine
        current.current = engine
        setState({ engine, bytes, failed: false })
      },
      (err) => {
        console.warn('version preview', err)
        if (!live) return
        current.current = null
        setState({ engine: null, bytes, failed: true })
      },
    )
    return () => {
      live = false
      if (mine) retired.current.add(mine)
    }
    // `open` is made fresh each render; the bytes are what it opens.
  }, [bytes])

  // Once the next version is on screen, free the ones before it.
  useEffect(() => {
    for (const e of retired.current) {
      if (e === state.engine) continue
      e.dispose()
      retired.current.delete(e)
    }
    if (state.bytes) shown.current(state.bytes)
  }, [state])

  useEffect(
    () => () => {
      // The one on screen is retired too by now: free each once.
      const all = new Set(retired.current)
      if (current.current) all.add(current.current)
      for (const e of all) e.dispose()
      retired.current.clear()
      current.current = null
    },
    [],
  )

  if (state.failed) return <CantShow kind={kind} bytes={state.bytes!} onShown={onShown} />
  // A view of its own for each version: the grid and pages cache what they drew by revision,
  // which starts over in every engine.
  return state.engine ? <Fragment key={idOf(state.engine)}>{view(state.engine)}</Fragment> : null
}

/** The width an element has to fill, kept up to date. */
function useWidth() {
  const ref = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(0)
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const measure = () => setWidth(el.clientWidth)
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])
  return [ref, width] as const
}

/** Room left and right of pages and slides. */
const SIDE = 32

function WordPages({ engine }: { engine: WordEngine }) {
  const [ref, width] = useWidth()
  const widest = Math.max(1, ...engine.sizes.map(([w]) => w))
  const scale = Math.min(1, Math.max(0.2, (width - SIDE * 2) / widest))
  return (
    <div ref={ref} className="vp-stack" role="document" aria-label="This version">
      {width > 0 && engine.sizes.map(([w, h], i) => <WordPage key={`${engine.revision}:${i}`} engine={engine} index={i} w={w} h={h} scale={scale} />)}
    </div>
  )
}

/** One page, drawn when it comes near the screen. */
function WordPage({ engine, index, w, h, scale }: { engine: WordEngine; index: number; w: number; h: number; scale: number }) {
  const box = useRef<HTMLDivElement>(null)
  const content = useRef<HTMLDivElement>(null)
  const [near, setNear] = useState(index < 2)
  useEffect(() => {
    const observer = new IntersectionObserver(([entry]) => entry.isIntersecting && setNear(true), { rootMargin: '800px 0px' })
    observer.observe(box.current!)
    return () => observer.disconnect()
  }, [])
  useLayoutEffect(() => {
    if (!near || !content.current) return
    const p = engine.paint(index, true)
    if (p) content.current.innerHTML = itemsHtml(p.items, engine.styles, engine.media.url)
  }, [near, engine, index])
  return (
    <div ref={box} className="vp-page" style={{ width: w * scale, height: h * scale }}>
      <div ref={content} className="docx-page-content" style={{ inset: 'auto', left: 0, top: 0, width: w, height: h, zoom: scale }} />
    </div>
  )
}

function Slides({ engine }: { engine: SlidesEngine }) {
  const [ref, width] = useWidth()
  const slideWidth = Math.max(160, Math.min(960, width - SIDE * 2))
  return (
    <div ref={ref} className="vp-stack slides" role="document" aria-label="This version">
      {width > 0 &&
        engine.slides.map((s, i) => (
          <div key={i} className="vp-slide" data-hidden={s.hidden || undefined}>
            <span className="vp-slide-number">{i + 1}</span>
            <SlideImage engine={engine} index={i} revision={s.revision} width={slideWidth} />
          </div>
        ))}
    </div>
  )
}

const noop = () => {}

/** The workbook's grid, which can be scrolled, selected and copied from but not changed, and its sheets. */
function Sheets({ engine }: { engine: SheetsEngine }) {
  useSyncExternalStore(engine.subscribe, engine.getTick)
  const sheet = engine.state.sheet
  const sheets = engine.sheets.map((s, i) => ({ ...s, i })).filter((s) => !s.hidden || s.i === sheet)
  return (
    <div className="vp-sheets">
      <div className="vp-grid">
        <Grid engine={engine} zoom={1} active={false} editing={null} onEdit={noop} onCommit={noop} charts={engine.chartsOf(sheet)} />
      </div>
      {sheets.length > 1 && (
        <div className="vp-tabs" role="tablist" aria-label="Sheets">
          {sheets.map((s) => (
            <button
              key={s.i}
              type="button"
              role="tab"
              aria-selected={s.i === sheet}
              className="vp-tab"
              onClick={() => engine.run({ cmd: 'select', sheet: s.i, anchor: [0, 0], focus: [0, 0] })}
            >
              {s.name}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

/** The PDF viewer, kept from version to version so the place and zoom stay as you scrub. */
function Pdf({ bytes, onShown }: { bytes: Uint8Array; onShown: (bytes: Uint8Array) => void }) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<Promise<PdfView> | null>(null)
  const [failed, setFailed] = useState(false)
  const shown = useRef(onShown)
  shown.current = onShown

  useEffect(() => {
    const element = host.current!
    const made = import('../plugins/PdfPreview').then((m) => m.createPdfView(element))
    view.current = made
    return () => {
      void made.then((v) => v.destroy())
      view.current = null
    }
  }, [])

  useEffect(() => {
    let live = true
    setFailed(false)
    view.current
      ?.then((v) => v.show(bytes))
      .then(
        () => live && shown.current(bytes),
        (err) => {
          console.warn('version preview', err)
          if (live) setFailed(true)
        },
      )
    return () => {
      live = false
    }
  }, [bytes])

  return (
    <>
      <div ref={host} className="vp-pdf" hidden={failed} />
      {failed && <CantShow kind="pdf" bytes={bytes} onShown={onShown} />}
    </>
  )
}

// In the encoding the editors read it in: UTF-8, UTF-16 or Windows-1252.
const decode = (bytes: Uint8Array) => decodeText(bytes).text

/** Links don't navigate the app away: web links open in the browser, ones within the page scroll to it. */
function onLinkClick(e: React.MouseEvent<HTMLElement>) {
  const link = (e.target as HTMLElement).closest('a')
  if (!link) return
  e.preventDefault()
  const href = link.getAttribute('href') ?? ''
  if (href.startsWith('#')) e.currentTarget.querySelector(`[id="${CSS.escape(href.slice(1))}"]`)?.scrollIntoView({ behavior: 'smooth' })
  else if (/^(https?:|mailto:)/i.test(href)) void openExternal(href)
}

function Markdown({ bytes, onShown }: { bytes: Uint8Array; onShown: (bytes: Uint8Array) => void }) {
  useEffect(() => onShown(bytes), [bytes, onShown])
  const html = renderMarkdown(decode(bytes)).html
  return (
    <div className="vh-page" role="document" aria-label="This version" onClick={onLinkClick}>
      <article className="md-body" dangerouslySetInnerHTML={{ __html: html }} />
    </div>
  )
}

function Text({ bytes, mono, onShown }: { bytes: Uint8Array; mono: boolean; onShown: (bytes: Uint8Array) => void }) {
  useEffect(() => onShown(bytes), [bytes, onShown])
  return (
    <div className={`vh-page vp-text${mono ? ' mono' : ''}`} role="document" aria-label="This version">
      {decode(bytes)}
    </div>
  )
}

function Picture({ bytes, type, onShown }: { bytes: Uint8Array; type: string; onShown: (bytes: Uint8Array) => void }) {
  const [url, setUrl] = useState<string | null>(null)
  useEffect(() => {
    const u = URL.createObjectURL(new Blob([bytes as Uint8Array<ArrayBuffer>], { type }))
    setUrl(u)
    return () => URL.revokeObjectURL(u)
  }, [bytes, type])
  return <div className="vp-picture">{url && <img src={url} alt="This version" onLoad={() => onShown(bytes)} onError={() => onShown(bytes)} />}</div>
}
