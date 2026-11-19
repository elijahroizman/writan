// New presentation: pick a starter deck. Each tile is the deck's own title slide, drawn by the
// engine, so what you see is what opens. Loaded on first use (it brings the slides engine).

import { useEffect, useRef, useState } from 'react'
import { SlidesEngine } from '../engine/slides/engine'
import { SlideImage } from '../engine/slides/SlideImage'
import { STARTERS, type StarterId, useStarters } from '../state/starters'
import { useWorkspace } from '../state/workspace'
import { Kbd, Modal } from './ui'

const COLUMNS = 3

interface Ready {
  bytes: Uint8Array
  engine: SlidesEngine
}

export default function StarterPicker() {
  const [index, setIndex] = useState(() => Math.max(0, STARTERS.findIndex((s) => s.id === useStarters.getState().last)))
  const [ready, setReady] = useState<Partial<Record<StarterId, Ready>>>({})
  const [error, setError] = useState<string | null>(null)
  const list = useRef<HTMLDivElement>(null)
  const width = Math.max(120, Math.min(184, Math.floor((window.innerWidth - 120) / COLUMNS)))
  const close = () => useStarters.getState().setOpen(false)

  useEffect(() => {
    list.current?.focus()
    let live = true
    const opened: SlidesEngine[] = []
    void (async () => {
      try {
        for (const { id } of STARTERS) {
          const bytes = await SlidesEngine.starter(id)
          const engine = await SlidesEngine.open(bytes)
          if (!live) return engine.dispose()
          opened.push(engine)
          setReady((r) => ({ ...r, [id]: { bytes, engine } }))
        }
      } catch (e) {
        if (live) setError(String((e as Error)?.message ?? e))
      }
    })()
    return () => {
      live = false
      for (const e of opened) e.dispose()
    }
  }, [])

  const pick = async (id: StarterId) => {
    close()
    useStarters.getState().remember(id)
    const bytes = ready[id]?.bytes ?? (await SlidesEngine.starter(id))
    useWorkspace.getState().newDocument('pptx', bytes)
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const at = Number(e.key)
      const n = STARTERS.length
      if (e.key === 'Escape') close()
      else if (e.key === 'ArrowRight') setIndex((i) => (i + 1) % n)
      else if (e.key === 'ArrowLeft') setIndex((i) => (i - 1 + n) % n)
      else if (e.key === 'ArrowDown') setIndex((i) => Math.min(n - 1, i + COLUMNS))
      else if (e.key === 'ArrowUp') setIndex((i) => Math.max(0, i - COLUMNS))
      else if (e.key === 'Enter') void pick(STARTERS[index].id)
      // Focus stays on the list, so return can't act on something behind the gallery.
      else if (e.key === 'Tab') list.current?.focus()
      else if (at >= 1 && at <= n && !e.metaKey && !e.ctrlKey) void pick(STARTERS[at - 1].id)
      else return
      e.preventDefault()
      e.stopPropagation()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  })

  return (
    <Modal placement="top" dim="light" layer="picker" onClose={close}>
      <div className="starter-picker" role="dialog" aria-modal="true" aria-label="New presentation">
        <h2>New presentation</h2>
        <div
          className="starter-grid"
          role="listbox"
          tabIndex={-1}
          ref={list}
          aria-activedescendant={`starter-${STARTERS[index].id}`}
          style={{ gridTemplateColumns: `repeat(${COLUMNS}, ${width}px)` }}
        >
          {STARTERS.map((s, i) => {
            const deck = ready[s.id]
            return (
              <button
                key={s.id}
                id={`starter-${s.id}`}
                type="button"
                role="option"
                aria-label={s.name}
                aria-selected={i === index}
                // The list keeps focus and return picks the highlighted tile.
                tabIndex={-1}
                className={`starter ${i === index ? 'on' : ''}`}
                // Moving, not entering: the gallery can open under a resting pointer.
                onMouseMove={() => i !== index && setIndex(i)}
                onClick={() => void pick(s.id)}
              >
                <span className="starter-slide">
                  {deck ? (
                    <SlideImage engine={deck.engine} index={0} revision={deck.engine.slides[0]?.revision ?? 0} width={width} lazy={false} />
                  ) : (
                    <span className="starter-placeholder" style={{ width, height: (width * 9) / 16 }} />
                  )}
                </span>
                <span className="starter-name">
                  {s.name}
                  <Kbd keys={String(i + 1)} />
                </span>
              </button>
            )
          })}
        </div>
        {error && <p className="starter-error">Couldn’t draw the previews: {error}</p>}
      </div>
    </Modal>
  )
}
