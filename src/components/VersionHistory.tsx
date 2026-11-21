import { ArchiveRestore, ChevronLeft, ChevronRight, MoreHorizontal, Pencil, X } from 'lucide-react'
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { create } from 'zustand'
import type { DiffOp } from '../lib/diff'
import { diffLines } from '../lib/diff'
import { KINDS } from '../lib/kinds'
import type { FileVersion } from '../lib/platform'
import * as platform from '../lib/platform'
import { useSettings } from '../state/settings'
import { getEditor, useWorkspace } from '../state/workspace'
import { Button, EmptyState, IconButton, KindIcon, Modal, PopoverMenu, Segmented, Spinner, TextField } from './ui'

/** Which tab's history is showing, if any. */
export const useVersionHistory = create<{ tabId: string | null; show(tabId: string | null): void }>((set) => ({
  tabId: null,
  show: (tabId) => set({ tabId }),
}))

/** Open the history of the tab in front, or explain why there isn't one. */
export function showVersionHistory(tabId = useWorkspace.getState().activeId) {
  const tab = useWorkspace.getState().tabs.find((t) => t.id === tabId)
  if (!tab) return
  if (!tab.path) {
    void platform.showError('Save this document first. Writan keeps a version each time you save a file.', 'No versions yet')
    return
  }
  if (!useSettings.getState().history) {
    void platform
      .confirmDialog('Version history is off, so Writan isn’t keeping copies when you save. Turn it on?', { okLabel: 'Turn On', kind: 'info' })
      .then((ok) => {
        if (!ok) return
        useSettings.getState().setHistory(true)
        useVersionHistory.getState().show(tabId)
      })
    return
  }
  useVersionHistory.getState().show(tabId)
}

const TEXT_KINDS = new Set(['md', 'txt', 'latex'])

/** Marks a Word heading's line, so the page can draw it as one. */
const HEADING = '\u0001'
/** Separates the cells of a Word table row, which compares as one line. */
const CELL = '\u0002'

/** The text of a paragraph's runs, with tabs and line breaks. */
const runText = (xml: string, tag: 'w' | 'a') =>
  [...xml.matchAll(new RegExp(`<${tag}:t(?:\\s[^>]*)?>([^<]*)</${tag}:t>|<${tag}:(tab|br)\\b`, 'g'))]
    .map((m) => (m[2] === 'tab' ? '\t' : m[2] === 'br' ? ' ' : decodeEntities(m[1])))
    .join('')

/** No runs of blank lines. */
const tidy = (lines: string[]) => lines.filter((line, i, all) => line.trim() || (i > 0 && all[i - 1].trim()))

/** A Word document's paragraphs, one line each: headings marked, and each table row one line of cells. */
function docxLines(xml: string): string[] {
  const out: string[] = []
  let row: string[] | null = null
  for (const chunk of xml.split('</w:p>')) {
    const start = Math.max(chunk.lastIndexOf('<w:p>'), chunk.lastIndexOf('<w:p '))
    // Tables open and close between paragraphs.
    for (const t of (start < 0 ? chunk : chunk.slice(0, start)).matchAll(/<w:tr[\s>]|<\/w:tr>|<w:tc[\s>]/g)) {
      if (t[0] === '</w:tr>') {
        if (row) out.push(row.join(CELL))
        row = null
      } else if (t[0].startsWith('<w:tr')) row = []
      else row?.push('')
    }
    const text = runText(chunk, 'w')
    if (row && row.length) row[row.length - 1] = [row[row.length - 1], text].filter(Boolean).join(' ')
    else if (start >= 0 || text) out.push(/<w:pStyle w:val="(Heading\d|Title)"/.test(chunk.slice(start)) && text.trim() ? HEADING + text : text)
  }
  if (row) out.push(row.join(CELL))
  return tidy(out)
}

/** What a version says, as lines, for comparing. Null for files that aren't text (PDF, pictures). */
async function readableText(kind: string, bytes: Uint8Array): Promise<string[] | null> {
  if (TEXT_KINDS.has(kind)) return new TextDecoder().decode(bytes).replace(/\r\n?/g, '\n').split('\n')
  if (kind !== 'docx' && kind !== 'pptx') return null
  const { default: JSZip } = await import('jszip')
  const zip = await JSZip.loadAsync(bytes)
  if (kind === 'docx') {
    const xml = await zip.file('word/document.xml')?.async('string')
    return xml ? docxLines(xml) : null
  }
  const slides = Object.keys(zip.files)
    .map((name) => /^ppt\/slides\/slide(\d+)\.xml$/.exec(name))
    .filter((m): m is RegExpExecArray => !!m)
    .sort((a, b) => Number(a[1]) - Number(b[1]))
  const lines: string[] = []
  for (const [name, n] of slides) {
    lines.push(`── Slide ${n} ──`)
    const xml = await zip.file(name)!.async('string')
    lines.push(...xml.split('</a:p>').map((p) => runText(p, 'a')).filter((l) => l.trim()))
  }
  return lines
}

function decodeEntities(s: string) {
  return s.replace(/&(amp|lt|gt|quot|apos|#\d+|#x[0-9a-f]+);/gi, (m, e: string) => {
    const named: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'" }
    if (named[e.toLowerCase()]) return named[e.toLowerCase()]
    const code = e[1] === 'x' || e[1] === 'X' ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10)
    return Number.isFinite(code) ? String.fromCodePoint(code) : m
  })
}

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

export function formatSize(n: number) {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`
  return `${(n / 1024 / 1024).toFixed(1)} MB`
}

/** 'save': what that save changed (the version before it → it). 'now': what restoring it would undo (it → the document now). */
type Compare = 'save' | 'now'

/** One paragraph (or line) of the page. `change` is a paragraph edited in place, word by word. */
type Row = { type: 'same' | 'add' | 'del'; text: string } | { type: 'change'; parts: DiffOp[] }

/** Words, numbers ("91,200"), runs of spaces, and punctuation on its own, so "approval." → "approval, along with…" only marks the new words. */
const words = (s: string) => s.match(/\s+|\p{N}+(?:[.,:]\p{N}+)*|[\p{L}\p{N}_'’-]+|[^\s\p{L}\p{N}]/gu) ?? []

/**
 * Each changed phrase as one removal then one addition: "~~is paused~~ runs as usual", not
 * "~~is~~ runs ~~paused~~ as usual". Spaces between changed words belong to the phrase.
 */
function merge(parts: DiffOp[]): DiffOp[] {
  const out: DiffOp[] = []
  for (let i = 0; i < parts.length; ) {
    if (parts[i].type === 'same') {
      out.push({ ...parts[i++] })
      continue
    }
    const del = { text: '', changed: false }
    const add = { text: '', changed: false }
    while (i < parts.length) {
      const p = parts[i]
      if (p.type !== 'same') {
        const side = p.type === 'del' ? del : add
        side.text += p.text
        side.changed = true
      }
      // A space carries on the phrase only if another change follows it. Both versions have it.
      else if (!p.text.trim() && parts[i + 1] && parts[i + 1].type !== 'same') {
        del.text += p.text
        add.text += p.text
      } else break
      i++
    }
    // A side with nothing of its own is just the space the two versions share.
    for (const [type, side] of [['del', del], ['add', add]] as const) {
      if (side.text) out.push({ type: side.changed ? type : 'same', text: side.text })
    }
  }
  return out
}

/** A removed paragraph and an added one that mostly share their words are one edited paragraph. */
function edited(before: string, after: string): DiffOp[] | null {
  const parts = diffLines(words(before), words(after), 800)
  if (!parts) return null
  const kept = parts.reduce((n, p) => n + (p.type === 'same' && p.text.trim() ? p.text.length : 0), 0)
  return kept >= 0.4 * Math.max(before.length, after.length) ? merge(parts) : null
}

/** Paragraph diff → the whole page, with removed and added paragraphs that are really one edit paired up. */
function toRows(ops: DiffOp[]): Row[] {
  const rows: Row[] = []
  for (let i = 0; i < ops.length; ) {
    if (ops[i].type === 'same') {
      rows.push(ops[i++] as Row)
      continue
    }
    const dels: string[] = []
    const adds: string[] = []
    while (i < ops.length && ops[i].type !== 'same') (ops[i].type === 'del' ? dels : adds).push(ops[i++].text)
    const pairs = Math.min(dels.length, adds.length)
    for (let j = 0; j < pairs; j++) {
      const parts = edited(dels[j], adds[j])
      if (parts) rows.push({ type: 'change', parts })
      else rows.push({ type: 'del', text: dels[j] }, { type: 'add', text: adds[j] })
    }
    rows.push(...dels.slice(pairs).map((text) => ({ type: 'del' as const, text })), ...adds.slice(pairs).map((text) => ({ type: 'add' as const, text })))
  }
  return rows
}

interface Comparison {
  rows: Row[] | null
  added: number
  removed: number
  changed: number
}

function compareTexts(before: string[], after: string[]): Comparison {
  const ops = diffLines(before, after)
  if (!ops) return { rows: null, added: 0, removed: 0, changed: 0 }
  const rows = toRows(ops)
  const count = (type: Row['type']) => rows.filter((r) => r.type === type).length
  return { rows, added: count('add'), removed: count('del'), changed: count('change') }
}

type View = Comparison | 'binary' | null

const sourceNote = (v: FileVersion) => (v.source === 'original' ? 'Before Writan first saved it' : v.source === 'outside' ? 'Changed in another app' : null)

export function VersionHistory() {
  const tabId = useVersionHistory((s) => s.tabId)
  const tab = useWorkspace((s) => s.tabs.find((t) => t.id === tabId))
  const close = useCallback(() => useVersionHistory.getState().show(null), [])
  const [versions, setVersions] = useState<FileVersion[] | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  const [compare, setCompare] = useState<Compare>('save')
  // What the page shows, and which version and comparison it's for: while the next one loads, the last stays up.
  const [view, setView] = useState<{ key: string; view: View } | null>(null)
  const [renaming, setRenaming] = useState(false)
  const [menu, setMenu] = useState<{ x: number; y: number; id: string; opener?: HTMLElement } | null>(null)
  const canvasRef = useRef<HTMLDivElement>(null)
  // Each version's text, read once, so scrubbing back and forth doesn't read and unzip it again.
  const texts = useRef(new Map<string, Promise<string[] | null>>())
  const path = tab?.path ?? null

  const refresh = useCallback(async () => {
    if (!path) return
    const list = await platform.listVersions(path).catch(() => [])
    texts.current.delete('now')
    setVersions(list)
    setSelected((cur) => (cur && list.some((v) => v.id === cur) ? cur : (list[0]?.id ?? null)))
  }, [path])

  useEffect(() => {
    void refresh()
  }, [refresh])

  // The tab closed underneath us.
  useEffect(() => {
    if (tabId && !tab) close()
  }, [tabId, tab, close])

  const version = versions?.find((v) => v.id === selected) ?? null
  const index = versions && version ? versions.indexOf(version) : -1
  const kind = tab?.kind ?? ''
  const key = version ? `${version.id}:${compare}` : ''

  const textOf = useCallback(
    (id: string) => {
      let hit = texts.current.get(id)
      if (!hit) {
        hit = (async () => {
          if (id === 'now') {
            const editor = tab && getEditor(tab.id)
            return readableText(kind, editor ? await editor.getBytes() : await platform.readFile(path!))
          }
          return readableText(kind, await platform.readVersion(path!, id))
        })()
        hit.catch(() => texts.current.delete(id))
        texts.current.set(id, hit)
      }
      return hit
    },
    [tab, kind, path],
  )

  // The page: this version, marked with what its save changed or with what's changed since.
  useEffect(() => {
    if (!path || !version || !versions) return
    let live = true
    void (async () => {
      let next: View
      try {
        const text = await textOf(version.id)
        if (!text) next = 'binary'
        else if (compare === 'save') {
          const prev = versions[index + 1]
          // The oldest version kept has nothing before it: show it as it was.
          next = compareTexts(prev ? ((await textOf(prev.id)) ?? []) : text, text)
        } else next = compareTexts(text, (await textOf('now')) ?? [])
      } catch (err) {
        console.warn('version diff', err)
        next = 'binary'
      }
      if (live) setView({ key: `${version.id}:${compare}`, view: next })
    })()
    return () => {
      live = false
    }
  }, [path, version, versions, index, compare, textOf])

  // Bring the first change into view when the page changes, unless it's already showing.
  useLayoutEffect(() => {
    const canvas = canvasRef.current
    const first = canvas?.querySelector<HTMLElement>('.vh-p.add, .vh-p.del, .vh-p.change')
    if (!canvas || !first) return
    const box = canvas.getBoundingClientRect()
    const at = first.getBoundingClientRect()
    if (at.top < box.top + 24 || at.bottom > box.bottom - 24) canvas.scrollTo({ top: canvas.scrollTop + at.top - box.top - box.height / 3, behavior: 'smooth' })
  }, [view?.key])

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
      // The timeline runs oldest to newest, left to right; the list is newest first.
      if (e.key === 'Escape') close()
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
  const unit = tab.kind === 'docx' || tab.kind === 'pptx' ? 'paragraph' : 'line'
  const shown = view?.view ?? null
  const loading = !!version && view?.key !== key

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
    texts.current.delete(v.id)
    await refresh()
  }

  const rename = async (label: string) => {
    setRenaming(false)
    if (!version || label.trim() === (version.label ?? '')) return
    await platform.labelVersion(path, version.id, label.trim() || null)
    await refresh()
  }

  const menuVersion = menu && versions?.find((v) => v.id === menu.id)
  const stats = shown && typeof shown === 'object' && shown.rows ? shown : null
  return (
    <Modal onClose={close}>
      <div className="vh" role="dialog" aria-modal="true" aria-label={`Version history of ${tab.title}`}>
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
                  const facts = [version.label && whenOf(version.time), index === 0 && 'Latest', sourceNote(version), index === versions!.length - 1 && compare === 'save' && 'Oldest kept'].filter(Boolean)
                  return facts.length > 0 && <span>{facts.join(' · ')}</span>
                })()}
                {stats && <Stats stats={stats} unit={unit} compare={compare} />}
              </p>
            )}
          </div>
          <div className="vh-actions">
            {version && shown !== 'binary' && (
              <Segmented
                value={compare}
                onChange={setCompare}
                options={[
                  { value: 'save', label: 'This save', title: 'What this save changed' },
                  { value: 'now', label: 'Compare with now', title: 'What restoring this version would undo' },
                ]}
              />
            )}
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

        <div className="vh-canvas" ref={canvasRef} data-loading={loading || undefined}>
          {versions?.length === 0 ? (
            <div className="vh-center">
              <EmptyState>No versions yet. Each time you save (or autosave), Writan keeps a copy here.</EmptyState>
            </div>
          ) : shown ? (
            <Page view={shown} compare={compare} kind={tab.kind} unit={unit} />
          ) : (
            <div className="vh-center">
              <Spinner />
            </div>
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

function Stats({ stats, unit, compare }: { stats: Comparison; unit: string; compare: Compare }) {
  const parts = [
    stats.changed > 0 && <span key="c" className="vh-stat change">{`${stats.changed} edited`}</span>,
    stats.added > 0 && <span key="a" className="vh-stat add">{`${stats.added} added`}</span>,
    stats.removed > 0 && <span key="d" className="vh-stat del">{`${stats.removed} removed`}</span>,
  ].filter(Boolean)
  if (!parts.length) return <span className="vh-stats">{compare === 'now' ? 'Same as now' : `No ${unit}s changed`}</span>
  return <span className="vh-stats">{parts}</span>
}

/**
 * Every version on one line, oldest on the left, grouped by day. Click or drag along it to scrub
 * through them; the arrow keys step one at a time.
 */
function Timeline({ versions, index, onPick }: { versions: FileVersion[]; index: number; onPick: (index: number) => void }) {
  const track = useRef<HTMLDivElement>(null)
  const [hover, setHover] = useState<number | null>(null)
  const n = versions.length
  const oldestFirst = [...versions].reverse()
  const days: { label: string; from: number; items: FileVersion[] }[] = []
  oldestFirst.forEach((v, i) => {
    const label = dayName(v.time, true)
    if (days[days.length - 1]?.label === label) days[days.length - 1].items.push(v)
    else days.push({ label, from: i, items: [v] })
  })
  const pos = n - 1 - index

  /** The version (newest-first index) nearest a point on the track. */
  const at = (x: number) => {
    const ticks = [...(track.current?.querySelectorAll<HTMLElement>('.vh-tick') ?? [])]
    let best = 0
    let dist = Infinity
    ticks.forEach((t, i) => {
      const r = t.getBoundingClientRect()
      const d = Math.abs(r.left + r.width / 2 - x)
      if (d < dist) {
        dist = d
        best = i
      }
    })
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
          onPick(at(e.clientX))
        }}
        onPointerMove={(e) => {
          if (e.currentTarget.hasPointerCapture(e.pointerId)) {
            onPick(at(e.clientX))
            setHover(null)
          } else setHover(at(e.clientX))
        }}
        onPointerLeave={() => setHover(null)}
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

function Page({ view, compare, kind, unit }: { view: Comparison | 'binary'; compare: Compare; kind: string; unit: string }) {
  if (view === 'binary') {
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
  if (!view.rows) {
    return (
      <div className="vh-center">
        <p className="muted">
          {compare === 'now' ? 'This version and now' : 'This version and the one before it'} are too different to compare {unit} by {unit}. Open it as a copy to look at it.
        </p>
      </div>
    )
  }
  return (
    <div className={`vh-page ${unit === 'line' ? 'lines' : ''} ${kind === 'latex' ? 'mono' : ''}`} role="document" aria-label="This version">
      {view.rows.map((r, i) => (
        <PageRow key={i} row={r} />
      ))}
    </div>
  )
}

/** A row as pieces of text, each kept, added or removed. */
const partsOf = (row: Row): DiffOp[] => (row.type === 'change' ? row.parts : [{ type: 'same', text: row.text }])

/** Pieces split at the table's cell breaks: one list of pieces per cell, in the newer version's columns. */
function cellsOf(parts: DiffOp[]): DiffOp[][] {
  const cells: DiffOp[][] = [[]]
  for (const p of parts) {
    // A removed cell break isn't a column any more.
    const pieces = p.type === 'del' ? [p.text.replaceAll(CELL, ' ')] : p.text.split(CELL)
    pieces.forEach((text, i) => {
      if (i > 0) cells.push([])
      if (text) cells[cells.length - 1].push({ type: p.type, text })
    })
  }
  return cells
}

const Pieces = ({ parts }: { parts: DiffOp[] }) => parts.map((p, i) => (p.type === 'same' ? p.text : p.type === 'add' ? <ins key={i}>{p.text}</ins> : <del key={i}>{p.text}</del>))

function PageRow({ row }: { row: Row }) {
  if (row.type !== 'change') {
    const slide = /^── Slide (\d+) ──$/.exec(row.text)
    if (slide) return <h4 className={`vh-slide ${row.type}`}>Slide {slide[1]}</h4>
  }
  const parts = partsOf(row).map((p) => ({ ...p, text: p.text.replaceAll(HEADING, '') }))
  const heading = partsOf(row)[0]?.text.startsWith(HEADING)
  const cells = cellsOf(parts)
  const className = `vh-p ${row.type}${heading ? ' heading' : ''}${cells.length > 1 ? ' cells' : ''}`
  if (cells.length > 1) {
    return (
      <div className={className} style={{ gridTemplateColumns: `repeat(${cells.length}, minmax(0, 1fr))` }}>
        {cells.map((c, i) => (
          <span key={i}>
            <Pieces parts={c} />
          </span>
        ))}
      </div>
    )
  }
  const Tag = heading ? 'h3' : 'p'
  return <Tag className={className}>{cells[0].length ? <Pieces parts={cells[0]} /> : ' '}</Tag>
}
