import { Folder, FolderOpen, MoreHorizontal, Moon, Settings, Sun } from 'lucide-react'
import { useMemo, useRef, useState } from 'react'
import { KINDS } from '../lib/kinds'
import { basename, dirname, revealInFinder, revealLabel, isTauri, shortcut, tildify } from '../lib/platform'
import { folderName, useFolder } from '../state/folder'
import { keysFor, withKeys } from '../state/shortcuts'
import { enabledKinds, usePlugins } from '../state/plugins'
import { createNew } from '../state/starters'
import { useSettings } from '../state/settings'
import type { RecentFile } from '../state/workspace'
import { useWorkspace } from '../state/workspace'
import { ActionCard, ActionRow, DocBadge, EmptyState, IconButton, KindIcon, PillTabs, PopoverMenu, Wordmark } from './ui'

/** Recent's filters: file types grouped the way people think of them. */
const RECENT_GROUPS = [
  { id: 'office', label: 'Office', none: 'Office files', kinds: ['docx', 'pptx', 'xlsx', 'csv'] },
  { id: 'notes', label: 'Notes', none: 'notes', kinds: ['md', 'txt'] },
  { id: 'pdf', label: 'PDF', none: 'PDFs', kinds: ['pdf'] },
  { id: 'image', label: 'Images', none: 'images', kinds: ['image'] },
  { id: 'latex', label: 'LaTeX', none: 'LaTeX files', kinds: ['latex'] },
] as const

type RecentGroup = (typeof RECENT_GROUPS)[number]['id']

export function Home() {
  const recents = useWorkspace((s) => s.recents)
  const theme = useWorkspace((s) => s.theme)
  const dropHover = useWorkspace((s) => s.dropHover)
  const { openDialog, openPaths, removeRecent, setTheme } = useWorkspace.getState()
  const [filter, setFilter] = useState<RecentGroup | 'all'>('all')
  const [menu, setMenu] = useState<{ x: number; y: number; align?: 'end'; file: Recent; opener: HTMLElement; fromMore?: boolean } | null>(null)
  const list = useRef<HTMLUListElement>(null)
  const recentHeader = useRef<HTMLDivElement>(null)
  const plugins = usePlugins((s) => s.enabled)
  const kinds = enabledKinds(plugins)
  const folders = useFolder((s) => s.recent)
  // A group shows while one of its file types opens (LaTeX only once it's turned on).
  const groups = RECENT_GROUPS.filter((g) => kinds.some((k) => (g.kinds as readonly string[]).includes(k.id)))
  const group = groups.find((g) => g.id === filter)
  const shownFilter = group ? filter : 'all'

  // Files and folders together, newest first. Folders only show under All.
  const shown = useMemo(() => {
    const all: Recent[] = [...recents, ...folders.map((f) => ({ ...f, kind: 'folder' as const }))]
    return all.filter((r) => !group || (group.kinds as readonly string[]).includes(r.kind)).sort((a, b) => b.openedAt - a.openedAt)
  }, [recents, folders, group])
  const open = (r: Recent) => (r.kind === 'folder' ? useFolder.getState().open(r.path) : void openPaths([r.path]))

  return (
    <div className="home">
      <div className="home-inner">
        <header className="home-header">
          <h1>
            <Wordmark size={30} variant={theme === 'light' ? 'light' : 'dark'} />
          </h1>
          <div className="home-header-actions">
            <IconButton label={shortcut(withKeys(theme === 'dark' ? 'Switch to light' : 'Switch to dark', 'toggle-theme'))} onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}>
              {theme === 'dark' ? <Sun size={16} /> : <Moon size={16} />}
            </IconButton>
            <IconButton label={shortcut(withKeys('Settings', 'settings'))} onClick={() => useSettings.getState().setOpen(true)}>
              <Settings size={16} />
            </IconButton>
          </div>
        </header>

        <div className="home-actions">
          <ActionCard
            className={`home-open ${dropHover ? 'drop' : ''}`}
            icon={
              <span className="action-card-icon">
                <FolderOpen size={22} strokeWidth={1.75} />
              </span>
            }
            title="Open…"
            hint="Files or a folder, or drop them here"
            keys={keysFor('open')}
            onClick={() => void openDialog()}
          />
          <div className="home-new" role="group" aria-labelledby="home-new-title">
            <h2 id="home-new-title">New</h2>
            <div className="home-new-list">
              {kinds
                .filter((k) => k.create)
                .map((k) => (
                  <ActionRow
                    key={k.id}
                    icon={<DocBadge kind={k.id} size={30} />}
                    title={newName(k.create!.title)}
                    hint={k.create!.hint}
                    label={k.create!.title}
                    onClick={() => createNew(k.id)}
                  />
                ))}
            </div>
          </div>
        </div>

        <section className="home-recents">
          <div className="home-recents-header" ref={recentHeader}>
            <h2>Recent</h2>
            <PillTabs
              label="Show"
              value={shownFilter}
              onChange={setFilter}
              options={[{ value: 'all' as const, label: 'All' }, ...groups.map((g) => ({ value: g.id, label: g.label }))]}
            />
          </div>

          {shown.length === 0 ? (
            <EmptyState>
              {recents.length + folders.length === 0 ? 'Files and folders you open will show up here.' : `No recent ${group?.none ?? 'files'}.`}
            </EmptyState>
          ) : (
            <ul className="recent-list" ref={list}>
              {shown.map((r) => (
                <li
                  key={r.path}
                  className="recent-item"
                  // On the item, so the row and its More button both open the menu, and focus goes
                  // back to whichever had it.
                  onContextMenu={(e) => {
                    e.preventDefault()
                    const opener = (e.target as HTMLElement).closest('button')
                    if (opener) setMenu({ x: e.clientX, y: e.clientY, file: r, opener })
                  }}
                  onKeyDown={(e) => {
                    // WebKit doesn't turn the menu key or ⇧F10 into a contextmenu event; Chromium does,
                    // and preventing the key stops it, so the menu opens once either way.
                    if (e.key !== 'ContextMenu' && !(e.shiftKey && e.key === 'F10')) return
                    const opener = (e.target as HTMLElement).closest('button')
                    if (!opener) return
                    e.preventDefault()
                    const rect = opener.getBoundingClientRect()
                    setMenu({ x: rect.left + 8, y: rect.bottom + 4, file: r, opener })
                  }}
                >
                  <button type="button" className="recent-row" title={r.path} onClick={() => open(r)}>
                    {r.kind === 'folder' ? (
                      <span className="recent-icon faint">
                        <Folder size={16} strokeWidth={1.75} />
                      </span>
                    ) : (
                      <span className="recent-icon" style={{ color: KINDS[r.kind].color }}>
                        <KindIcon kind={r.kind} size={16} />
                      </span>
                    )}
                    <span className="recent-name">{r.kind === 'folder' ? folderName(r.path) : basename(r.path)}</span>
                    <span className="recent-path">
                      <bdi>{prettyDir(r.path)}</bdi>
                    </span>
                    <span className="recent-time">{relativeTime(r.openedAt)}</span>
                  </button>
                  {/* A sibling of the row, not inside it, so it's a real button that Tab reaches. */}
                  <IconButton
                    label={`More for ${r.kind === 'folder' ? folderName(r.path) : basename(r.path)}`}
                    className="recent-more"
                    // The name says which file; the tooltip doesn't need to repeat it.
                    title="More"
                    aria-haspopup="menu"
                    aria-expanded={menu?.file.path === r.path && !!menu.fromMore}
                    onClick={(e) => {
                      const rect = e.currentTarget.getBoundingClientRect()
                      setMenu({ x: rect.right, y: rect.bottom + 4, align: 'end', file: r, opener: e.currentTarget, fromMore: true })
                    }}
                  >
                    <MoreHorizontal size={16} />
                  </IconButton>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
      {menu && (
        <PopoverMenu
          anchor={menu}
          opener={menu.opener}
          onClose={() => setMenu(null)}
          items={[
            { label: 'Open', onSelect: () => open(menu.file) },
            ...(isTauri ? [{ label: revealLabel, onSelect: () => void revealInFinder(menu.file.path) }] : []),
            {
              label: 'Remove from Recent',
              separatorBefore: true,
              onSelect: () => {
                const at = shown.findIndex((x) => x.path === menu.file.path)
                if (menu.file.kind === 'folder') useFolder.getState().forgetRecent(menu.file.path)
                else removeRecent(menu.file.path)
                // Keep the user's place: the next row's More (or the previous one, or the filters
                // once the list is empty) takes the focus the removed row had.
                requestAnimationFrame(() => {
                  const more = list.current?.querySelectorAll<HTMLElement>('.recent-more')
                  const target = more?.[at] ?? more?.[at - 1] ?? recentHeader.current?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')
                  target?.focus()
                })
              },
            },
          ]}
        />
      )}
    </div>
  )
}

/** A row in the Recent list: a file, or a folder opened in the sidebar. */
type Recent = RecentFile | { path: string; openedAt: number; kind: 'folder' }

/** “New document” on its own under the New heading: “Document”. */
function newName(title: string) {
  const rest = title.replace(/^New\s+/i, '')
  return rest.charAt(0).toUpperCase() + rest.slice(1)
}

function prettyDir(path: string) {
  return tildify(dirname(path))
}

function relativeTime(ts: number) {
  const diff = Date.now() - ts
  const min = 60_000
  const hour = 60 * min
  const day = 24 * hour
  if (diff < min) return 'Just now'
  if (diff < hour) return `${Math.floor(diff / min)} min ago`
  if (diff < day) return `${Math.floor(diff / hour)} h ago`
  if (diff < 2 * day) return 'Yesterday'
  if (diff < 7 * day) return `${Math.floor(diff / day)} days ago`
  return new Date(ts).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: diff > 300 * day ? 'numeric' : undefined })
}
