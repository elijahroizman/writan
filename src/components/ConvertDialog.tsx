import { ArrowDown, ArrowUp, Check, ClipboardCopy, Download, FileStack, FileText, FolderOpen } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { create } from 'zustand'
import {
  bundleTabsToPdf,
  canExportMarkdown,
  canIncludeInPdfBundle,
  convertAndDeliver,
  copyTabAsMarkdown,
  targetsForKind,
} from '../lib/convert'
import type { ConvertFormat } from '../lib/convert'
import * as platform from '../lib/platform'
import { useWorkspace } from '../state/workspace'
import { DocBadge, Kbd, Modal } from './ui'

export type ConvertModalMode = 'convert' | 'bundle'

interface ConvertModalState {
  open: boolean
  mode: ConvertModalMode
  tabId: string | null
  openConvert(tabId?: string | null): void
  openBundle(): void
  close(): void
}

export const useConvertDialog = create<ConvertModalState>((set) => ({
  open: false,
  mode: 'convert',
  tabId: null,
  openConvert: (tabId = null) => set({ open: true, mode: 'convert', tabId }),
  openBundle: () => set({ open: true, mode: 'bundle', tabId: null }),
  close: () => set({ open: false, tabId: null }),
}))

export function ConvertModal() {
  const mode = useConvertDialog((s) => s.mode)
  return mode === 'bundle' ? <BundlePdfDialog /> : <ConvertFileDialog />
}

function ConvertFileDialog() {
  const tabs = useWorkspace((s) => s.tabs)
  const activeId = useWorkspace((s) => s.activeId)
  const requestedId = useConvertDialog((s) => s.tabId)
  const close = useConvertDialog((s) => s.close)

  const tab = useMemo(() => tabs.find((t) => t.id === (requestedId ?? activeId)) ?? null, [tabs, requestedId, activeId])
  const targets = useMemo(() => (tab ? targetsForKind(tab.kind) : []), [tab])

  const [index, setIndex] = useState(0)
  const [delivery, setDelivery] = useState<'open' | 'save'>('open')
  const [busy, setBusy] = useState<string | null>(null)
  const [ocrRunning, setOcrRunning] = useState(false)
  const [copied, setCopied] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const conversion = useRef<AbortController | null>(null)

  useEffect(() => {
    listRef.current?.focus()
    return () => conversion.current?.abort()
  }, [])

  const runConvert = async (format: ConvertFormat) => {
    if (!tab || busy || conversion.current) return
    const usesOcr = tab.kind === 'image' || (tab.kind === 'pdf' && format === 'docx')
    const job = new AbortController()
    conversion.current = job
    setError(null)
    setOcrRunning(usesOcr)
    setBusy(`Converting to ${format.toUpperCase()}…`)
    try {
      const ok = await convertAndDeliver(tab.id, format, delivery, { signal: job.signal, onProgress: setBusy })
      if (ok) close()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      conversion.current = null
      setBusy(null)
      setOcrRunning(false)
    }
  }

  const dismiss = () => {
    if (ocrRunning) conversion.current?.abort()
    else close()
  }

  const runCopyMarkdown = async () => {
    if (!tab || busy) return
    setError(null)
    setBusy('Preparing Markdown…')
    try {
      await copyTabAsMarkdown(tab.id)
      setCopied(true)
      setTimeout(() => close(), 450)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(null)
    }
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        dismiss()
      } else if (targets.length && e.key === 'ArrowDown') {
        setIndex((i) => (i + 1) % targets.length)
      } else if (targets.length && e.key === 'ArrowUp') {
        setIndex((i) => (i - 1 + targets.length) % targets.length)
      } else if (targets.length && e.key === 'Enter' && !busy) {
        if (e.target instanceof HTMLButtonElement) return
        void runConvert((targets[index] ?? targets[0]).format)
      } else if (!e.metaKey && !e.ctrlKey && !e.altKey && !busy) {
        const at = Number(e.key)
        if (at >= 1 && at <= Math.min(9, targets.length)) {
          void runConvert(targets[at - 1].format)
        } else {
          return
        }
      } else {
        return
      }
      e.preventDefault()
      e.stopPropagation()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  })

  if (!tab) {
    return (
      <Modal placement="top" dim="light" layer="picker" onClose={close}>
        <div className="new-file convert-dialog" role="dialog" aria-modal="true" aria-label="Convert file">
          <h2>Convert file</h2>
          <p className="convert-empty muted">Open a document first to convert it.</p>
        </div>
      </Modal>
    )
  }

  const showCopyMd = canExportMarkdown(tab.kind)

  return (
    <Modal placement="top" dim="light" layer="picker" onClose={dismiss}>
      <div className="new-file convert-dialog" role="dialog" aria-modal="true" aria-label="Convert file">
        <div className="convert-head">
          <DocBadge kind={tab.kind} size={22} />
          <div className="convert-head-text">
            <h2>Convert “{tab.title}”</h2>
          </div>
          <button
            type="button"
            className="convert-switch"
            disabled={!!busy}
            onClick={() => useConvertDialog.getState().openBundle()}
            title="Bundle open files into one PDF"
          >
            <FileStack size={13} />
            <span>Bundle PDF…</span>
          </button>
        </div>

        {showCopyMd && (
          <button type="button" className="convert-copy-btn" onClick={() => void runCopyMarkdown()} disabled={!!busy}>
            {copied ? <Check size={14} /> : <ClipboardCopy size={14} />}
            <span>{copied ? 'Copied clean Markdown to clipboard' : 'Copy as clean Markdown (for LLMs)'}</span>
          </button>
        )}

        {targets.length === 0 ? (
          <p className="convert-empty muted">No conversion targets are available for this file type.</p>
        ) : (
          <div className="new-file-list" role="listbox" tabIndex={-1} ref={listRef} aria-activedescendant={`convert-${targets[index]?.format}`}>
            {targets.map((t, i) => (
              <button
                key={t.format}
                id={`convert-${t.format}`}
                type="button"
                role="option"
                aria-selected={i === index}
                disabled={!!busy}
                className={`new-file-item ${i === index ? 'on' : ''}`}
                onMouseEnter={() => setIndex(i)}
                onClick={() => void runConvert(t.format)}
              >
                <DocBadge kind={t.format} size={28} />
                <span className="new-file-text">
                  <strong>{t.label}</strong>
                  <span className="muted">{t.hint}</span>
                </span>
                {i < 9 && <Kbd keys={String(i + 1)} />}
              </button>
            ))}
          </div>
        )}

        <div className="convert-footer">
          <div className="convert-mode" role="radiogroup" aria-label="Delivery mode">
            <button
              type="button"
              role="radio"
              aria-checked={delivery === 'open'}
              className={delivery === 'open' ? 'on' : ''}
              onClick={() => setDelivery('open')}
            >
              <FolderOpen size={13} />
              <span>Open in tab</span>
            </button>
            <button
              type="button"
              role="radio"
              aria-checked={delivery === 'save'}
              className={delivery === 'save' ? 'on' : ''}
              onClick={() => setDelivery('save')}
            >
              <Download size={13} />
              <span>Save to disk</span>
            </button>
          </div>
          {busy && <span className="convert-status muted" role="status">{busy}</span>}
          {ocrRunning && <button type="button" className="btn" onClick={() => conversion.current?.abort()}>Cancel</button>}
        </div>
        {error && <p className="convert-error">{error}</p>}
      </div>
    </Modal>
  )
}

function BundlePdfDialog() {
  const tabs = useWorkspace((s) => s.tabs)
  const close = useConvertDialog((s) => s.close)

  const eligible = useMemo(() => tabs.filter((t) => canIncludeInPdfBundle(t.kind)), [tabs])
  const [order, setOrder] = useState<string[]>(() => eligible.map((t) => t.id))
  const [selected, setSelected] = useState<Set<string>>(() => new Set(eligible.map((t) => t.id)))
  const [name, setName] = useState(() => (eligible.length > 0 ? `${eligible[0].title} bundle` : 'Bundle'))
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const orderedTabs = useMemo(() => {
    const byId = new Map(eligible.map((t) => [t.id, t]))
    const list = order.map((id) => byId.get(id)).filter((t): t is (typeof eligible)[number] => !!t)
    for (const t of eligible) {
      if (!order.includes(t.id)) list.push(t)
    }
    return list
  }, [eligible, order])

  const chosenTabs = orderedTabs.filter((t) => selected.has(t.id))

  const toggle = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const move = (idx: number, delta: -1 | 1) => {
    const nextIdx = idx + delta
    if (nextIdx < 0 || nextIdx >= orderedTabs.length) return
    const ids = orderedTabs.map((t) => t.id)
    const [moved] = ids.splice(idx, 1)
    ids.splice(nextIdx, 0, moved)
    setOrder(ids)
  }

  const runBundle = async (delivery: 'open' | 'save') => {
    if (!chosenTabs.length || busy) return
    setError(null)
    setBusy(`Preparing ${chosenTabs.length} file${chosenTabs.length === 1 ? '' : 's'}…`)
    try {
      const pdfBytes = await bundleTabsToPdf(chosenTabs, (done, total, current) => {
        setBusy(done < total ? `Rendering “${current}” (${done + 1}/${total})…` : current)
      })
      const cleanName = (name.trim() || 'Bundle').replace(/\.pdf$/i, '') + '.pdf'
      if (delivery === 'save') {
        const saved = await platform.saveBytesAs(cleanName, pdfBytes)
        if (saved) close()
      } else {
        useWorkspace.getState().openUnsaved(pdfBytes, cleanName)
        close()
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(null)
    }
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        close()
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [close])

  return (
    <Modal placement="top" dim="light" layer="picker" onClose={close}>
      <div className="new-file convert-dialog bundle-dialog" role="dialog" aria-modal="true" aria-label="Bundle open files into one PDF">
        <div className="convert-head">
          <FileStack size={18} />
          <div className="convert-head-text">
            <h2>Bundle open files into one PDF</h2>
          </div>
          {tabs.length > 0 && (
            <button
              type="button"
              className="convert-switch"
              onClick={() => useConvertDialog.getState().openConvert()}
              title="Convert active file"
            >
              <FileText size={13} />
              <span>Convert…</span>
            </button>
          )}
        </div>

        {orderedTabs.length === 0 ? (
          <p className="convert-empty muted">Open one or more Word, PowerPoint, Excel, Markdown, image, or PDF files to bundle them.</p>
        ) : (
          <>
            <div className="bundle-list">
              {orderedTabs.map((t, i) => {
                const checked = selected.has(t.id)
                return (
                  <div key={t.id} className={`bundle-row ${checked ? 'checked' : ''}`}>
                    <label className="bundle-check">
                      <input type="checkbox" checked={checked} onChange={() => toggle(t.id)} disabled={!!busy} />
                      <DocBadge kind={t.kind} size={22} />
                      <span className="bundle-title">{t.title}</span>
                      <span className="bundle-kind muted">.{t.kind}</span>
                    </label>
                    <div className="bundle-order">
                      <button
                        type="button"
                        aria-label={`Move ${t.title} up`}
                        disabled={i === 0 || !!busy}
                        onClick={() => move(i, -1)}
                      >
                        <ArrowUp size={13} />
                      </button>
                      <button
                        type="button"
                        aria-label={`Move ${t.title} down`}
                        disabled={i === orderedTabs.length - 1 || !!busy}
                        onClick={() => move(i, 1)}
                      >
                        <ArrowDown size={13} />
                      </button>
                    </div>
                  </div>
                )
              })}
            </div>

            <label className="bundle-name">
              <span className="muted">Output filename</span>
              <div className="bundle-name-field">
                <input
                  type="text"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  disabled={!!busy}
                  placeholder="Bundle"
                />
                <span className="muted">.pdf</span>
              </div>
            </label>

            {busy && <p className="convert-status muted">{busy}</p>}
            {error && <p className="convert-error">{error}</p>}

            <div className="bundle-actions">
              <button type="button" className="btn" disabled={!chosenTabs.length || !!busy} onClick={() => void runBundle('save')}>
                <Download size={14} />
                <span>Save PDF…</span>
              </button>
              <button type="button" className="btn primary" disabled={!chosenTabs.length || !!busy} onClick={() => void runBundle('open')}>
                <FileStack size={14} />
                <span>Merge {chosenTabs.length} into PDF</span>
              </button>
            </div>
          </>
        )}
      </div>
    </Modal>
  )
}
