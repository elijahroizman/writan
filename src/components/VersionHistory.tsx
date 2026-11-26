import { ArchiveRestore, ChevronLeft, ChevronRight, MoreHorizontal, Pencil, X } from 'lucide-react'
import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { KINDS } from '../lib/kinds'
import type { FileVersion } from '../lib/platform'
import * as platform from '../lib/platform'
import { formatVersionSize as formatSize, showVersionHistory, useVersionHistory } from '../state/modals'
import { useWorkspace } from '../state/workspace'
import { Button, EmptyState, IconButton, Modal, PopoverMenu, Spinner, TextField } from './ui'

export { formatSize, showVersionHistory, useVersionHistory }

// The engines that draw a version load the first time history opens.
const VersionPreview = lazy(() => import('./VersionPreview'))

const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString()

/** "Today", "Yesterday", or the date, long ("Wednesday 18 November") or short ("18 Nov"). */
function dayName(time: number, short = false) {
  const d = new Date(time)
  const today = new Date()
  if (sameDay(d, today)) return 'Today'
  if (sameDay(d, new Date(Date.now() - 86_400_000))) return 'Yesterday'
  const year = d.getFullYear() === today.getFullYear() ? undefined : 'numeric'
  return short
    ? d.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year })
    : d.toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric', year })
}

const timeOf = (time: number) => new Date(time).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })

/** "Today at 10:27", "Wednesday 18 November at 10:39". */
const whenOf = (time: number) => `${dayName(time)} at ${timeOf(time)}`

const sourceNote = (v: FileVersion) => (v.source === 'original' ? 'Before Writan first saved it' : v.source === 'outside' ? 'Changed in another app' : null)

export function VersionHistory() {
  const tabId = useVersionHistory((s) => s.tabId)
  const tab = useWorkspace((s) => s.tabs.find((t) => t.id === tabId))
  const close = useCallback(() => useVersionHistory.getState().show(null), [])
  const [versions, setVersions] = useState<FileVersion[] | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  // The version on screen: while the next one loads, the last stays up.
  const [shown, setShown] = useState<{ id: string; bytes: Uint8Array | null } | null>(null)
  // The version whose preview has finished drawing.
  const [drawn, setDrawn] = useState<string | null>(null)
  const [renaming, setRenaming] = useState(false)
  const [menu, setMenu] = useState<{ x: number; y: number; id: string; opener?: HTMLElement } | null>(null)
  // Each version's bytes, read once, so scrubbing back and forth doesn't read them again.
  const reads = useRef(new Map<string, Promise<Uint8Array>>())
  const versionOf = useRef(new WeakMap<Uint8Array, string>())
  const dialogRef = useRef<HTMLDivElement>(null)
  const path = tab?.path ?? null

  const refresh = useCallback(async () => {
    if (!path) return
    const list = await platform.listVersions(path).catch(() => [])
    setVersions(list)
    setSelected((cur) => (cur && list.some((v) => v.id === cur) ? cur : (list[0]?.id ?? null)))
  }, [path])

  useEffect(() => {
    void refresh()
  }, [refresh])

  // Take the keyboard from the editor behind, which would otherwise still get what's typed.
  useEffect(() => {
    if (tabId) dialogRef.current?.focus()
  }, [tabId])

  // The tab closed underneath us.
  useEffect(() => {
    if (tabId && !tab) close()
  }, [tabId, tab, close])

  const version = versions?.find((v) => v.id === selected) ?? null
  const index = versions && version ? versions.indexOf(version) : -1
  const name = path?.split(/[\\/]/).pop() ?? ''

  // Read the version picked, then show it.
  useEffect(() => {
    if (!path || !version) return
    let live = true
    let read = reads.current.get(version.id)
    if (!read) {
      read = platform.readVersion(path, version.id)
      read.catch(() => reads.current.delete(version.id))
      reads.current.set(version.id, read)
    }
    read.then(
      (bytes) => {
        versionOf.current.set(bytes, version.id)
        if (live) setShown({ id: version.id, bytes })
      },
      (err) => {
        console.warn('version read', err)
        if (!live) return
        setShown({ id: version.id, bytes: null })
        setDrawn(version.id)
      },
    )
    return () => {
      live = false
    }
  }, [path, version])

  // Which version a drawing is of: by the time one finishes, another may have been picked.
  const onShown = useCallback((bytes: Uint8Array) => {
    const id = versionOf.current.get(bytes)
    if (id) setDrawn(id)
  }, [])

  const step = useCallback(
    (to: number) => {
      if (!versions?.length) return
      setSelected(versions[Math.max(0, Math.min(versions.length - 1, to))].id)
      setRenaming(false)
    },
    [versions],
  )

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement) {
        if (e.key === 'Escape') e.target.blur()
        return
      }
      if (document.querySelector('.menu')) return
      if (e.key === 'Escape') close()
      // In the version itself (a workbook's grid, a PDF), the arrow keys move around it.
      else if (e.target instanceof Element && e.target.closest('.vh-canvas')) return
      // The timeline runs oldest to newest, left to right; the list is newest first.
      else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') step(index + 1)
      else if (e.key === 'ArrowRight' || e.key === 'ArrowDown') step(index - 1)
      else if (e.key === 'Home') step(Infinity)
      else if (e.key === 'End') step(0)
      else return
      e.preventDefault()
      e.stopPropagation()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [close, step, index])

  if (!tab || !path) return null
  const ext = KINDS[tab.kind]?.extensions[0] ?? ''
  const loading = !!version && drawn !== version.id
  // Workbooks and PDFs scroll inside their own view.
  const fills = ['xlsx', 'csv', 'pdf'].includes(tab.kind)

  const saveNow = async () => {
    if (!(await useWorkspace.getState().save(tab.id))) return
    // The copy is written just after the save returns.
    window.setTimeout(() => {
      setSelected(null)
      void refresh()
    }, 250)
  }

  const restore = async () => {
    if (!version) return
    const day = dayName(version.time)
    const when = `${day === 'Today' || day === 'Yesterday' ? day.toLowerCase() : day} at ${timeOf(version.time)}`
    const ok = await platform.confirmDialog(`The document goes back to how it was ${day === 'Today' || day === 'Yesterday' ? when : `on ${when}`}. What it is now stays in the history, so you can come back to it.`, {
      title: `Restore this version of “${tab.title}”?`,
      okLabel: 'Restore',
      kind: 'info',
    })
    if (!ok) return
    const bytes = await platform.readVersion(path, version.id)
    if (await useWorkspace.getState().restoreVersion(tab.id, bytes, `Restored from ${when}`)) {
      window.setTimeout(() => {
        setSelected(null)
        void refresh()
      }, 250)
    }
  }

  const openCopy = async (v: FileVersion) => {
    const bytes = await platform.readVersion(path, v.id)
    const stamp = new Date(v.time).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' }).replace(/[/:]/g, '.')
    const id = useWorkspace.getState().openUnsaved(bytes, `${tab.title} (${v.label || stamp}).${ext}`)
    if (id) close()
  }

  const remove = async (v: FileVersion) => {
    const ok = await platform.confirmDialog('This copy is deleted for good.', { title: 'Delete this version?', okLabel: 'Delete', danger: true })
    if (!ok) return
    await platform.deleteVersion(path, v.id)
    reads.current.delete(v.id)
    await refresh()
  }

  const rename = async (label: string) => {
    setRenaming(false)
    if (!version || label.trim() === (version.label ?? '')) return
    await platform.labelVersion(path, version.id, label.trim() || null)
    await refresh()
  }

  const menuVersion = menu && versions?.find((v) => v.id === menu.id)
  return (
    <Modal onClose={close}>
      <div ref={dialogRef} tabIndex={-1} className="vh" role="dialog" aria-modal="true" aria-label={`Version history of ${tab.title}`}>
        <header className="vh-head">
          <div className="vh-title">
            {version && renaming ? (
              <TextField
                className="vh-rename"
                autoFocus
                defaultValue={version.label ?? ''}
                placeholder="Name this version. Named versions are always kept."
                onBlur={(e) => void rename(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') void rename(e.currentTarget.value)
                  if (e.key === 'Escape') setRenaming(false)
                }}
              />
            ) : version ? (
              <button type="button" className="vh-name" title="Name this version (named versions are always kept)" onClick={() => setRenaming(true)}>
                <h2>{version.label || whenOf(version.time)}</h2>
                <Pencil size={13} />
              </button>
            ) : (
              <h2>{tab.title}</h2>
            )}
            {version && (
              <p className="vh-sub">
                {(() => {
                  const facts = [version.label && whenOf(version.time), index === 0 && 'Latest', sourceNote(version), index === versions!.length - 1 && 'Oldest kept'].filter(Boolean)
                  return facts.length > 0 && <span>{facts.join(' · ')}</span>
                })()}
              </p>
            )}
          </div>
          <div className="vh-actions">
            {version && (
              <>
                <IconButton
                  label="More"
                  onClick={(e) => {
                    const r = e.currentTarget.getBoundingClientRect()
                    setMenu({ x: r.right, y: r.bottom + 4, id: version.id, opener: e.currentTarget })
                  }}
                >
                  <MoreHorizontal size={16} />
                </IconButton>
                <Button variant="primary" disabled={loading || (index === 0 && !tab.dirty)} title={index === 0 && !tab.dirty ? 'This is how the document is now' : undefined} onClick={() => void restore()}>
                  <ArchiveRestore size={14} /> Restore
                </Button>
              </>
            )}
            <IconButton label="Close (Esc)" onClick={close}>
              <X size={16} />
            </IconButton>
          </div>
        </header>

        <div className="vh-canvas" data-loading={loading || undefined} data-fill={fills || undefined}>
          {versions?.length === 0 ? (
            <div className="vh-center">
              <EmptyState>No versions yet. Each time you save (or autosave), Writan keeps a copy here.</EmptyState>
            </div>
          ) : shown && !shown.bytes ? (
            <div className="vh-center">
              <EmptyState>This version couldn’t be read.</EmptyState>
            </div>
          ) : (
            <>
              {shown?.bytes && (
                <Suspense fallback={null}>
                  <VersionPreview kind={tab.kind} bytes={shown.bytes} name={name} onShown={onShown} />
                </Suspense>
              )}
              {loading && drawn === null && (
                <div className="vh-center vh-spinner">
                  <Spinner />
                </div>
              )}
            </>
          )}
        </div>

        {(!!versions?.length || tab.dirty) && (
          <footer className="vh-foot">
            {!!versions?.length && <Timeline versions={versions} index={index} onPick={step} />}
            {tab.dirty && (
              <Button className="vh-save" onClick={() => void saveNow()} title="Save the document now, as a new version">
                Save version
              </Button>
            )}
          </footer>
        )}
      </div>
      {menu && menuVersion && (
        <PopoverMenu
          anchor={{ x: menu.x, y: menu.y, align: 'end' }}
          opener={menu.opener}
          onClose={() => setMenu(null)}
          items={[
            {
              label: menuVersion.label ? 'Rename…' : 'Name this version…',
              onSelect: () => {
                setSelected(menuVersion.id)
                setRenaming(true)
              },
            },
            { label: 'Open as a copy', onSelect: () => void openCopy(menuVersion) },
            { label: 'Delete version', danger: true, separatorBefore: true, onSelect: () => void remove(menuVersion) },
          ]}
        />
      )}
    </Modal>
  )
}

/**
 * Every version on one line, oldest on the left, grouped by day. Click or drag along it to scrub
 * through them; the arrow keys step one at a time.
 */
function Timeline({ versions, index, onPick }: { versions: FileVersion[]; index: number; onPick: (index: number) => void }) {
  const track = useRef<HTMLDivElement>(null)
  const [hover, setHover] = useState<number | null>(null)
  const n = versions.length
  const days = useMemo(() => {
    const oldestFirst = [...versions].reverse()
    const out: { label: string; from: number; items: FileVersion[] }[] = []
    oldestFirst.forEach((v, i) => {
      const label = dayName(v.time, true)
      if (out[out.length - 1]?.label === label) out[out.length - 1].items.push(v)
      else out.push({ label, from: i, items: [v] })
    })
    return out
  }, [versions])
  const pos = n - 1 - index
  const tickCenters = useRef<number[] | null>(null)
  useEffect(() => {
    tickCenters.current = null
  }, [versions])

  const readCenters = () => {
    const ticks = [...(track.current?.querySelectorAll<HTMLElement>('.vh-tick') ?? [])]
    return ticks.map((t) => {
      const r = t.getBoundingClientRect()
      return r.left + r.width / 2
    })
  }

  /** The version (newest-first index) nearest a point on the track. */
  const at = (x: number) => {
    const centers = tickCenters.current ?? readCenters()
    let best = 0
    let dist = Infinity
    for (let i = 0; i < centers.length; i++) {
      const d = Math.abs(centers[i] - x)
      if (d < dist) {
        dist = d
        best = i
      }
    }
    return n - 1 - best
  }

  const hovered = hover !== null && hover !== index ? versions[hover] : null
  const tickLeft = (i: number) => {
    const el = track.current?.querySelectorAll<HTMLElement>('.vh-tick')[n - 1 - i]
    const box = track.current?.getBoundingClientRect()
    if (!el || !box) return 0
    const r = el.getBoundingClientRect()
    return r.left + r.width / 2 - box.left
  }
  const v = versions[index]

  return (
    <div className="vh-time">
      <IconButton label="Older (←)" size="sm" disabled={index >= n - 1} onClick={() => onPick(index + 1)}>
        <ChevronLeft size={16} />
      </IconButton>
      <div
        ref={track}
        className="vh-track"
        role="slider"
        tabIndex={0}
        aria-label="Versions, oldest to newest"
        aria-valuemin={1}
        aria-valuemax={n}
        aria-valuenow={pos + 1}
        aria-valuetext={v ? `${whenOf(v.time)}${v.label ? `, ${v.label}` : ''}` : undefined}
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId)
          e.currentTarget.focus()
          tickCenters.current = readCenters()
          onPick(at(e.clientX))
        }}
        onPointerMove={(e) => {
          if (e.currentTarget.hasPointerCapture(e.pointerId)) {
            onPick(at(e.clientX))
            setHover(null)
          } else {
            tickCenters.current ??= readCenters()
            setHover(at(e.clientX))
          }
        }}
        onPointerUp={() => { tickCenters.current = null }}
        onPointerCancel={() => { tickCenters.current = null }}
        onPointerLeave={() => { tickCenters.current = null; setHover(null) }}
      >
        {days.map((d) => (
          <div key={d.from} className="vh-dayrun" style={{ flexGrow: d.items.length }}>
            <div className="vh-ticks">
              {d.items.map((item, j) => {
                const i = n - 1 - (d.from + j)
                return <span key={item.id} className="vh-tick" data-on={i === index || undefined} data-named={!!item.label || undefined} />
              })}
            </div>
            <span className="vh-daylabel" data-short={/\d/.test(d.label) ? new Date(d.items[0].time).getDate() : d.label}>
              <span>{d.label}</span>
            </span>
          </div>
        ))}
        {hovered && (
          <span className="vh-tip" style={{ left: tickLeft(hover!) }}>
            {hovered.label ? <strong>{hovered.label}</strong> : null}
            {`${dayName(hovered.time, true)}, ${timeOf(hovered.time)}`}
          </span>
        )}
      </div>
      <IconButton label="Newer (→)" size="sm" disabled={index <= 0} onClick={() => onPick(index - 1)}>
        <ChevronRight size={16} />
      </IconButton>
      <span className="vh-count">{`${pos + 1} of ${n}`}</span>
    </div>
  )
}
