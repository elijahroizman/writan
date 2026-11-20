import { useEffect, useRef, useState } from 'react'
import { create } from 'zustand'
import { enabledKinds, usePlugins } from '../state/plugins'
import { createNew } from '../state/starters'
import { DocBadge, Kbd, Modal } from './ui'

export const useNewFile = create<{ open: boolean; setOpen(open: boolean): void }>((set) => ({
  open: false,
  setOpen: (open) => set({ open }),
}))

/** ⌘N: pick the kind of file to create. Arrows and return, or the number beside it. */
export function NewFilePicker() {
  const enabled = usePlugins((s) => s.enabled)
  const kinds = enabledKinds(enabled).filter((k) => k.create)
  const [index, setIndex] = useState(0)
  const list = useRef<HTMLDivElement>(null)
  const close = () => useNewFile.getState().setOpen(false)
  const pick = (kind: string) => {
    close()
    createNew(kind)
  }

  useEffect(() => {
    list.current?.focus()
  }, [])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const at = Number(e.key)
      if (e.key === 'Escape') close()
      else if (e.key === 'ArrowDown') setIndex((i) => (i + 1) % kinds.length)
      else if (e.key === 'ArrowUp') setIndex((i) => (i - 1 + kinds.length) % kinds.length)
      else if (e.key === 'Enter') pick(kinds[index].id)
      else if (at >= 1 && at <= kinds.length && !e.metaKey && !e.ctrlKey) pick(kinds[at - 1].id)
      else return
      e.preventDefault()
      e.stopPropagation()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  })

  return (
    <Modal placement="top" dim="light" layer="picker" onClose={close}>
      <div className="new-file" role="dialog" aria-modal="true" aria-label="New file">
        <h2>New file</h2>
        <div className="new-file-list" role="listbox" tabIndex={-1} ref={list} aria-activedescendant={`new-${kinds[index]?.id}`}>
          {kinds.map((k, i) => (
            <button
              key={k.id}
              id={`new-${k.id}`}
              type="button"
              role="option"
              aria-selected={i === index}
              className={`new-file-item ${i === index ? 'on' : ''}`}
              onMouseEnter={() => setIndex(i)}
              onClick={() => pick(k.id)}
            >
              <DocBadge kind={k.id} size={28} />
              <span className="new-file-text">
                <strong>{k.create!.title}</strong>
                <span className="muted">{k.create!.hint}</span>
              </span>
              {i < 9 && <Kbd keys={String(i + 1)} />}
            </button>
          ))}
        </div>
      </div>
    </Modal>
  )
}
