// Presentation from a template: a searchable grid of starter decks beside a large preview of the
// selected one, with all its slides. Every picture is the deck itself, drawn by the engine, so
// what you see is what opens. Loaded on first use (it brings the slides engine).

import { Plus, Search } from 'lucide-react'
import { type RefObject, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { SlidesEngine } from '../engine/slides/engine'
import { SlideImage } from '../engine/slides/SlideImage'
import { createFromStarter, STARTERS, type StarterId, useStarters } from '../state/starters'
import { Button, Kbd, Modal, TextField } from './ui'

const TILE = 148
const GAP = 10
/** Slides shown under the large preview, after the title slide. */
const STRIP = 5

type Starter = (typeof STARTERS)[number]

// A deck is opened when its tile first comes into view, one at a time, and kept for the session:
// reopening the gallery draws at once, and a long list doesn't open decks nobody scrolls to.
const decks = new Map<StarterId, SlidesEngine>()
const loading = new Map<StarterId, Promise<SlidesEngine>>()
let queue: Promise<unknown> = Promise.resolve()

function loadDeck(id: StarterId): Promise<SlidesEngine> {
  let p = loading.get(id)
  if (!p) {
    p = queue.then(async () => {
      const engine = await SlidesEngine.open(await SlidesEngine.starter(id))
      decks.set(id, engine)
      return engine
    })
    queue = p.catch(() => {})
    loading.set(id, p)
    // A failed deck can be tried again next time.
    p.catch(() => loading.delete(id))
  }
  return p
}

function useDeck(id: StarterId, wanted: boolean) {
  const [engine, setEngine] = useState(() => decks.get(id) ?? null)
  const [failed, setFailed] = useState<string | null>(null)
  useEffect(() => {
    if (engine?.slides.length || !wanted) return
    let live = true
    loadDeck(id).then(
      (e) => live && setEngine(e),
      (e) => live && setFailed(String((e as Error)?.message ?? e)),
    )
    return () => {
      live = false
    }
  }, [id, wanted, engine])
  return { engine, failed }
}

function matches(s: Starter, query: string) {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  const text = `${s.name} ${s.about} ${s.tags}`.toLowerCase()
  return words.every((w) => text.includes(w))
}

/**
 * Sizes from the window: two columns of tiles and the preview in what's left. The preview is
 * about 0.68 of its width tall, plus 216 px of text, buttons and the gallery's own chrome, and
 * the gallery sits 13vh down (.modal-top), so a short window gets a narrower preview. Too small
 * for one, the tiles take the whole width.
 */
function useLayout() {
  const measure = () => {
    const width = Math.min(900, window.innerWidth - 80)
    const grid = 2 * (TILE + 8) + GAP
    const byWidth = width - 32 - grid - 24
    const byHeight = (window.innerHeight * 0.87 - 24 - 216) / 0.68
    const preview = Math.floor(Math.min(byWidth, byHeight))
    if (preview >= 220) return { total: 32 + grid + 24 + preview, columns: 2, preview }
    return { total: width, columns: Math.max(1, Math.floor((width - 32 + GAP) / (TILE + 8 + GAP))), preview: 0 }
  }
  const [layout, setLayout] = useState(measure)
  useEffect(() => {
    const onResize = () => setLayout(measure())
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])
  return layout
}

export default function StarterPicker() {
  const fallback = useStarters((s) => s.default)
  const [query, setQuery] = useState('')
  const shown = useMemo(() => STARTERS.filter((s) => matches(s, query)), [query])
  const [selected, setSelected] = useState<StarterId>(fallback)
  const current = shown.find((s) => s.id === selected) ?? shown[0]
  const index = current ? shown.indexOf(current) : -1
  const grid = useRef<HTMLDivElement>(null)
  const dialog = useRef<HTMLDivElement>(null)
  const { total, columns, preview } = useLayout()
  const close = () => useStarters.getState().setOpen(false)

  const pick = (id: StarterId) => {
    close()
    void createFromStarter(id)
  }
  const move = (to: number) => {
    const s = shown[Math.max(0, Math.min(shown.length - 1, to))]
    if (s) setSelected(s.id)
  }

  // Keep the selected tile in view as the arrows move through a long list.
  useLayoutEffect(() => {
    if (current) document.getElementById(`starter-${current.id}`)?.scrollIntoView({ block: 'nearest' })
  }, [current])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // A focused button answers return itself (tabbed to: a click leaves focus in the search box).
      const inButton = e.target instanceof HTMLButtonElement && !e.target.disabled && !!dialog.current?.contains(e.target)
      // With a query typed, left and right move the caret in it.
      const sideways = !(e.target instanceof HTMLInputElement && query)
      if (e.key === 'Escape') {
        if (query) setQuery('')
        else close()
      } else if (e.key === 'Tab') {
        // Focus stays in the gallery: the search box and its buttons, round and round.
        const stops = [...(dialog.current?.querySelectorAll<HTMLElement>('input, button:not([disabled]):not([tabindex="-1"])') ?? [])]
        const at = stops.indexOf(document.activeElement as HTMLElement)
        stops[(at + (e.shiftKey ? -1 : 1) + stops.length) % stops.length]?.focus()
      } else if (e.key === 'ArrowRight' && sideways) move(index + 1)
      else if (e.key === 'ArrowLeft' && sideways) move(index - 1)
      else if (e.key === 'ArrowDown') move(index + columns)
      else if (e.key === 'ArrowUp') move(index - columns)
      else if (e.key === 'Enter' && !inButton) current && pick(current.id)
      else return
      e.preventDefault()
      e.stopPropagation()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  })

  return (
    <Modal placement="top" dim="light" layer="picker" onClose={close}>
      <div ref={dialog} className="starter-picker" role="dialog" aria-modal="true" aria-label="Choose a template" style={{ width: total }}>
        <header className="starter-head">
          <h2>New presentation from a template</h2>
          <TextField
            autoFocus
            className="starter-search"
            icon={<Search size={14} />}
            placeholder="Search templates"
            aria-label="Search templates"
            value={query}
            onChange={(e) => setQuery(e.currentTarget.value)}
            role="combobox"
            aria-expanded
            aria-controls="starter-grid"
            aria-activedescendant={current ? `starter-${current.id}` : undefined}
          />
        </header>
        <div className="starter-body">
          <div
            id="starter-grid"
            className="starter-grid"
            role="listbox"
            aria-label="Templates"
            ref={grid}
            style={{ gridTemplateColumns: `repeat(${columns}, ${TILE + 8}px)`, gap: `12px ${GAP}px` }}
          >
            {shown.map((s) => (
              <Tile key={s.id} starter={s} on={s.id === current?.id} isDefault={s.id === fallback} root={grid} onSelect={() => setSelected(s.id)} onPick={() => pick(s.id)} />
            ))}
            {shown.length === 0 && <p className="starter-none">No templates match “{query}”.</p>}
          </div>
          {preview > 0 && current && <Preview key={current.id} starter={current} width={preview} isDefault={current.id === fallback} onPick={() => pick(current.id)} />}
        </div>
        <footer className="starter-foot">
          <span>
            <strong>New presentation</strong> starts from {STARTERS.find((s) => s.id === fallback)?.name}.
          </span>
          <span className="starter-keys">
            <Kbd keys="↩" /> create <Kbd keys="Esc" /> {query ? 'clear search' : 'close'}
          </span>
        </footer>
      </div>
    </Modal>
  )
}

function Tile({
  starter,
  on,
  isDefault,
  root,
  onSelect,
  onPick,
}: {
  starter: Starter
  on: boolean
  isDefault: boolean
  root: RefObject<HTMLDivElement | null>
  onSelect: () => void
  onPick: () => void
}) {
  const ref = useRef<HTMLButtonElement>(null)
  const [visible, setVisible] = useState(false)
  useEffect(() => {
    const observer = new IntersectionObserver(([entry]) => entry.isIntersecting && setVisible(true), { root: root.current, rootMargin: '200px 0px' })
    observer.observe(ref.current!)
    return () => observer.disconnect()
  }, [root])
  const { engine } = useDeck(starter.id, visible || on)
  const blank = starter.id === 'blank'
  return (
    <button
      ref={ref}
      id={`starter-${starter.id}`}
      type="button"
      role="option"
      aria-label={starter.name}
      aria-selected={on}
      // The search box keeps focus; arrows and return act on the selected tile.
      tabIndex={-1}
      className={`starter ${on ? 'on' : ''} ${blank ? 'blank' : ''}`}
      onMouseDown={(e) => e.preventDefault()}
      onClick={onSelect}
      onDoubleClick={onPick}
    >
      <span className="starter-slide">
        {engine?.slides.length ? (
          <SlideImage engine={engine} index={0} revision={engine.slides[0].revision} width={TILE} lazy={false} />
        ) : (
          <span className="starter-placeholder" style={{ width: TILE, height: (TILE * 9) / 16 }} />
        )}
        {blank && (
          <span className="starter-blank-mark" aria-hidden>
            <Plus size={20} strokeWidth={1.5} />
          </span>
        )}
      </span>
      <span className="starter-name">
        {starter.name}
        {isDefault && <span className="starter-default">Default</span>}
      </span>
    </button>
  )
}

function Preview({ starter, width: planned, isDefault, onPick }: { starter: Starter; width: number; isDefault: boolean; onPick: () => void }) {
  const { engine, failed } = useDeck(starter.id, true)
  const slides = engine?.slides ?? []
  // The slides are drawn at the pane's real width: a scrollbar that takes room narrows it.
  const ref = useRef<HTMLElement>(null)
  const [width, setWidth] = useState(planned)
  useLayoutEffect(() => {
    const pane = ref.current!
    const measure = () => setWidth(Math.floor(pane.clientWidth))
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(pane)
    return () => observer.disconnect()
  }, [])
  const small = Math.floor((width - (STRIP - 1) * 6) / STRIP)
  return (
    <section ref={ref} className="starter-preview" style={{ flexBasis: planned }} aria-label={`${starter.name} preview`}>
      <div className="starter-preview-main">
        {slides.length ? (
          <SlideImage engine={engine!} index={0} revision={slides[0].revision} width={width} lazy={false} />
        ) : (
          <span className="starter-placeholder" style={{ width, height: (width * 9) / 16 }} />
        )}
      </div>
      {slides.length > 1 && (
        <div className="starter-strip">
          {slides.slice(1, STRIP + 1).map((s, i) => (
            <span key={s.id} className="starter-strip-slide">
              <SlideImage engine={engine!} index={i + 1} revision={s.revision} width={small} lazy={false} />
            </span>
          ))}
        </div>
      )}
      <div className="starter-about">
        <strong>{starter.name}</strong>
        <span className="muted">{failed ? `Couldn’t draw the preview: ${failed}` : starter.about}</span>
      </div>
      <div className="starter-actions" onMouseDown={(e) => e.preventDefault()}>
        <Button variant="quiet" disabled={isDefault} onClick={() => useStarters.getState().setDefault(starter.id)}>
          {isDefault ? 'Your default' : 'Make default'}
        </Button>
        <Button variant="primary" onClick={onPick}>
          Create
        </Button>
      </div>
    </section>
  )
}
