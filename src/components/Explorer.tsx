import { ChevronRight, ChevronsDownUp, Copy, File, Folder, FolderOpen, MoreHorizontal, RotateCw, Search, X } from 'lucide-react'
import { memo, useCallback, useEffect, useMemo, useState } from 'react'
import { folderFiles, relativeDir, startFolderPalette } from '../lib/folderFiles'
import { KINDS, kindFromPath } from '../lib/kinds'
import type { DirEntry } from '../lib/platform'
import { basename, isTauri, listDir, revealInFinder, revealLabel } from '../lib/platform'
import { folderName, useFolder } from '../state/folder'
import { keysFor } from '../state/shortcuts'
import { useWorkspace } from '../state/workspace'
import { KindIcon } from './ui'
import { Resizer } from './Workbench'
import type { MenuEntry } from './ui'
import { IconButton, PopoverMenu } from './ui'

export { folderFiles, startFolderPalette }

/** A folder's entries, read when it's first shown and again after a refresh. */
const cache = new Map<string, Promise<DirEntry[]>>()
let cacheRev = -1
function entriesOf(path: string, rev: number) {
  if (rev !== cacheRev) {
    cache.clear()
    cacheRev = rev
  }
  let hit = cache.get(path)
  if (!hit) {
    hit = listDir(path).catch(() => [])
    cache.set(path, hit)
  }
  return hit
}

function useEntries(path: string) {
  const rev = useFolder((s) => s.rev)
  const [entries, setEntries] = useState<DirEntry[] | null>(null)
  useEffect(() => {
    let live = true
    void entriesOf(path, rev).then((e) => live && setEntries(e))
    return () => {
      live = false
    }
  }, [path, rev])
  return entries
}

type Menu = { x: number; y: number; entry: DirEntry }

export function Explorer() {
  const root = useFolder((s) => s.root)
  const showAll = useFolder((s) => s.showAll)
  const rev = useFolder((s) => s.rev)
  const [filter, setFilter] = useState('')
  const [menu, setMenu] = useState<Menu | null>(null)
  const [more, setMore] = useState<{ x: number; y: number; align: 'end' } | null>(null)
  const { toggleVisible, collapseAll, refresh, close, setShowAll } = useFolder.getState()

  // Files change behind our back (Finder, sync apps): look again when the window comes back,
  // throttled so rapid window switching doesn't re-stat an entire folder tree every click.
  useEffect(() => {
    let lastRefresh = Date.now()
    const onFocus = () => {
      const now = Date.now()
      if (now - lastRefresh < 15_000) return
      lastRefresh = now
      useFolder.getState().refresh()
    }
    window.addEventListener('focus', onFocus)
    return () => window.removeEventListener('focus', onFocus)
  }, [])

  const activePath = useWorkspace((s) => s.tabs.find((t) => t.id === s.activeId)?.path ?? null)
  const tabPathKey = useWorkspace((s) => s.tabs.map((t) => t.path ?? '').join('\n'))
  const openPaths = useMemo(() => new Set(tabPathKey ? tabPathKey.split('\n').filter(Boolean) : []), [tabPathKey])
  const handleMenu = useCallback((m: Menu) => setMenu(m), [])

  if (!root) return null
  const menuItems = (entry: DirEntry): MenuEntry[] => [
    ...(!entry.isDir && kindFromPath(entry.name) ? [{ label: 'Open', onSelect: () => void useWorkspace.getState().openPaths([entry.path]) }] : []),
    ...(isTauri ? [{ label: revealLabel, icon: <FolderOpen size={14} />, onSelect: () => void revealInFinder(entry.path) }] : []),
    { label: 'Copy path', icon: <Copy size={14} />, onSelect: () => void navigator.clipboard?.writeText(entry.path) },
  ]

  return (
    <aside className="pane explorer" aria-label="Folder">
      <div className="sidebar-header">
        <span className="doc-icon folder-badge">
          <Folder size={14} strokeWidth={1.75} />
        </span>
        <span className="title" title={root}>
          {folderName(root)}
        </span>
        <IconButton
          label="Folder options"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect()
            setMore({ x: r.right, y: r.bottom + 4, align: 'end' })
          }}
        >
          <MoreHorizontal size={16} />
        </IconButton>
      </div>
      <div className="explorer-filter">
        <label className="field">
          <Search size={13} className="faint" />
          <input
            value={filter}
            placeholder="Find a file"
            spellCheck={false}
            onChange={(e) => setFilter(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') setFilter('')
            }}
          />
          {filter && (
            <button type="button" className="explorer-clear" aria-label="Clear" onClick={() => setFilter('')}>
              <X size={12} />
            </button>
          )}
        </label>
      </div>
      <div className="sidebar-scroll explorer-tree" role="tree">
        {filter.trim() ? (
          <Matches root={root} query={filter.trim()} rev={rev} activePath={activePath} openPaths={openPaths} onMenu={handleMenu} />
        ) : (
          <DirChildren path={root} depth={0} showAll={showAll} activePath={activePath} openPaths={openPaths} onMenu={handleMenu} />
        )}
      </div>
      <Resizer side="explorer" />
      {menu && <PopoverMenu anchor={menu} onClose={() => setMenu(null)} items={menuItems(menu.entry)} />}
      {more && (
        <PopoverMenu
          anchor={more}
          onClose={() => setMore(null)}
          items={[
            { label: 'Refresh', icon: <RotateCw size={14} />, onSelect: refresh },
            { label: 'Collapse all folders', icon: <ChevronsDownUp size={14} />, onSelect: collapseAll },
            { label: showAll ? 'Show only documents' : 'Show all files', separatorBefore: true, onSelect: () => setShowAll(!showAll) },
            ...(isTauri ? [{ label: revealLabel, icon: <FolderOpen size={14} />, onSelect: () => void revealInFinder(root) }] : []),
            { label: 'Hide folder sidebar', shortcut: keysFor('toggle-explorer'), separatorBefore: true, onSelect: toggleVisible },
            { label: 'Close folder', onSelect: close },
          ]}
        />
      )}
    </aside>
  )
}

const DirChildren = memo(function DirChildren({
  path,
  depth,
  showAll,
  activePath,
  openPaths,
  onMenu,
}: {
  path: string
  depth: number
  showAll: boolean
  activePath: string | null
  openPaths: ReadonlySet<string>
  onMenu(m: Menu): void
}) {
  const entries = useEntries(path)
  if (!entries) return null
  const shown = entries.filter((e) => e.isDir || showAll || kindFromPath(e.name))
  if (!shown.length && depth === 0) return <p className="explorer-empty faint">{showAll ? 'This folder is empty.' : 'No documents in this folder.'}</p>
  return (
    <>
      {shown.map((e) =>
        e.isDir ? (
          <DirRow key={e.path} entry={e} depth={depth} showAll={showAll} activePath={activePath} openPaths={openPaths} onMenu={onMenu} />
        ) : (
          <FileRow key={e.path} entry={e} depth={depth} active={activePath === e.path} open={openPaths.has(e.path)} onMenu={onMenu} />
        ),
      )}
    </>
  )
})

const DirRow = memo(function DirRow({
  entry,
  depth,
  showAll,
  activePath,
  openPaths,
  onMenu,
}: {
  entry: DirEntry
  depth: number
  showAll: boolean
  activePath: string | null
  openPaths: ReadonlySet<string>
  onMenu(m: Menu): void
}) {
  const open = useFolder((s) => s.expanded.includes(entry.path))
  return (
    <>
      <button
        type="button"
        role="treeitem"
        aria-expanded={open}
        className="explorer-row dir"
        style={{ paddingLeft: 8 + depth * 14 }}
        onClick={() => useFolder.getState().toggleDir(entry.path)}
        onContextMenu={(e) => {
          e.preventDefault()
          onMenu({ x: e.clientX, y: e.clientY, entry })
        }}
      >
        <ChevronRight size={13} className={`explorer-chevron ${open ? 'open' : ''}`} />
        {open ? <FolderOpen size={14} /> : <Folder size={14} />}
        <span className="explorer-name">{entry.name}</span>
      </button>
      {open && (
        <div role="group">
          <DirChildren path={entry.path} depth={depth + 1} showAll={showAll} activePath={activePath} openPaths={openPaths} onMenu={onMenu} />
        </div>
      )}
    </>
  )
})

const FileRow = memo(function FileRow({
  entry,
  depth,
  detail,
  active,
  open,
  onMenu,
}: {
  entry: DirEntry
  depth: number
  detail?: string
  active: boolean
  open: boolean
  onMenu(m: Menu): void
}) {
  const kind = kindFromPath(entry.name)
  return (
    <button
      type="button"
      role="treeitem"
      className={`explorer-row file ${active ? 'active' : ''} ${open ? 'open' : ''} ${kind ? '' : 'other'}`}
      style={{ paddingLeft: 8 + depth * 14 + 17 }}
      title={entry.path}
      onClick={() => kind && void useWorkspace.getState().openPaths([entry.path])}
      onDoubleClick={() => !kind && void revealInFinder(entry.path)}
      onContextMenu={(e) => {
        e.preventDefault()
        onMenu({ x: e.clientX, y: e.clientY, entry })
      }}
    >
      <span className="explorer-icon" style={kind ? { color: KINDS[kind].color } : undefined}>
        {kind ? <KindIcon kind={kind} size={14} /> : <File size={14} />}
      </span>
      <span className="explorer-name">{entry.name}</span>
      {detail && <span className="explorer-detail">{detail}</span>}
    </button>
  )
})

/** Searching: every document under the folder whose name (or folder) has the letters typed. */
const Matches = memo(function Matches({
  root,
  query,
  rev,
  activePath,
  openPaths,
  onMenu,
}: {
  root: string
  query: string
  rev: number
  activePath: string | null
  openPaths: ReadonlySet<string>
  onMenu(m: Menu): void
}) {
  const [files, setFiles] = useState<string[] | null>(null)
  useEffect(() => {
    let live = true
    setFiles(null)
    void folderFiles(root, rev).then((f) => live && setFiles(f))
    return () => {
      live = false
    }
  }, [root, rev])
  const matches = useMemo(() => {
    if (!files) return []
    const q = query.toLowerCase()
    const scored: { path: string; score: number }[] = []
    for (const path of files) {
      const name = basename(path).toLowerCase()
      const rel = path.slice(root.length).toLowerCase()
      const at = name.indexOf(q)
      if (at !== -1) scored.push({ path, score: 1000 - at * 10 - name.length })
      else if (rel.includes(q)) scored.push({ path, score: 100 - rel.length })
      else if (subsequence(q, name)) scored.push({ path, score: -name.length })
    }
    return scored.sort((a, b) => b.score - a.score).slice(0, 200)
  }, [files, query, root])
  if (!files) return <p className="explorer-empty faint">Looking…</p>
  if (!matches.length) return <p className="explorer-empty faint">No documents match “{query}”.</p>
  return (
    <>
      {matches.map(({ path }) => (
        <FileRow
          key={path}
          entry={{ name: basename(path), path, isDir: false }}
          depth={0}
          detail={relativeDir(root, path)}
          active={activePath === path}
          open={openPaths.has(path)}
          onMenu={onMenu}
        />
      ))}
    </>
  )
})

function subsequence(q: string, text: string) {
  let i = 0
  for (const ch of text) {
    if (ch === q[i] && ++i === q.length) return true
  }
  return i === q.length
}
