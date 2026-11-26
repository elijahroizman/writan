import { ArrowRightLeft, Check, CircleAlert, ClipboardCopy, Columns2, Copy, FileStack, FolderOpen, History, LayoutGrid, Plus, Save, SaveOff, Users, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { autosaveBlock, useAutosaveStatus } from '../lib/autosave'
import { isMac, isTauri, revealInFinder, revealLabel, shortcut, tildify } from '../lib/platform'
import { enabledKinds, usePlugins } from '../state/plugins'
import { useSettings } from '../state/settings'
import { keysFor, withKeys } from '../state/shortcuts'
import { HOME_TAB_ID, SPLIT_KINDS, useWorkspace } from '../state/workspace'
import type { DocKind } from '../lib/kinds'
import { pickSplit } from '../App'
import { keeperOf, useCollab } from '../collab/session'
import { canExportMarkdown, canIncludeInPdfBundle, copyTabAsMarkdown, targetsForKind } from '../lib/convert'
import { useConvertDialog } from './convertDialogState'
import { ChatButton } from './AgentChat'
import { canShare, ShareButton, showSharePanel } from './Share'
import { UpdateButton } from './UpdateButton'
import { showVersionHistory } from './versionHistoryState'
import { IconButton, KindIcon, PopoverMenu } from './ui'

export function TitleBar() {
  const tabs = useWorkspace((s) => s.tabs)
  const activeId = useWorkspace((s) => s.activeId)
  const split = useWorkspace((s) => s.split)
  const { activate, closeTab, moveTab, newDocument, openDialog, duplicateTab, closeOthers, splitWith } = useWorkspace.getState()
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null)
  const plugins = usePlugins((s) => s.enabled)
  const [tabMenu, setTabMenu] = useState<{ x: number; y: number; id: string } | null>(null)
  const [fullscreen, setFullscreen] = useState(false)
  const [drag, setDrag] = useState<{ id: string; dx: number } | null>(null)
  const tabRefs = useRef(new Map<string, HTMLDivElement>())
  const scrollRef = useRef<HTMLDivElement>(null)
  const shared = useCollab((s) => s)

  // Traffic lights disappear in full screen, so reclaim their space.
  useEffect(() => {
    if (!isTauri || !isMac) return
    let unlisten: (() => void) | undefined
    void import('@tauri-apps/api/window').then(async ({ getCurrentWindow }) => {
      const win = getCurrentWindow()
      setFullscreen(await win.isFullscreen())
      unlisten = await win.onResized(async () => setFullscreen(await win.isFullscreen()))
    })
    return () => unlisten?.()
  }, [])

  // Tabs that don't fit scroll; the one in front is always in view.
  useEffect(() => {
    const box = scrollRef.current
    const tab = tabRefs.current.get(activeId)
    if (!box || !tab) return
    const b = box.getBoundingClientRect()
    const r = tab.getBoundingClientRect()
    if (r.left < b.left) box.scrollLeft -= b.left - r.left
    else if (r.right > b.right) box.scrollLeft += r.right - b.right
  }, [activeId, tabs.length])

  /** Pointer-driven reordering (HTML5 drag-and-drop is owned by the native file drop handler). */
  const startDrag = (e: React.PointerEvent, id: string) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest('.tab-close')) return
    // Activating the tab can scroll it into view, and the strip can scroll mid-drag; the tab moves with it.
    const scroll0 = scrollRef.current?.scrollLeft ?? 0
    activate(id)
    const el = e.currentTarget as HTMLElement
    el.setPointerCapture(e.pointerId)
    const rectOf = (tabId: string | undefined) => (tabId ? tabRefs.current.get(tabId)?.getBoundingClientRect() : undefined)
    // Where the pointer would be if the tab hadn't moved; shifts as the tab swaps places.
    let anchorX = e.clientX
    let moved = false

    const onMove = (ev: PointerEvent) => {
      if (!moved && Math.abs(ev.clientX - anchorX) < 4) return
      moved = true
      const order = useWorkspace.getState().tabs.map((t) => t.id)
      const index = order.indexOf(id)
      const next = rectOf(order[index + 1])
      const prev = rectOf(order[index - 1])
      if (next && ev.clientX > next.left + next.width / 2) {
        moveTab(id, index + 1)
        anchorX += next.width + 2
      } else if (prev && ev.clientX < prev.left + prev.width / 2) {
        moveTab(id, index - 1)
        anchorX -= prev.width + 2
      }
      setDrag({ id, dx: ev.clientX - anchorX + (scrollRef.current?.scrollLeft ?? 0) - scroll0 })
    }
    const onUp = () => {
      el.removeEventListener('pointermove', onMove)
      el.removeEventListener('pointerup', onUp)
      el.removeEventListener('pointercancel', onUp)
      setDrag(null)
    }
    el.addEventListener('pointermove', onMove)
    el.addEventListener('pointerup', onUp)
    el.addEventListener('pointercancel', onUp)
  }

  return (
    <header className={`titlebar ${isMac && isTauri ? 'mac' : ''} ${fullscreen ? 'fullscreen' : ''}`} data-tauri-drag-region>
      <div className="tabs" role="tablist" data-tauri-drag-region>
        <div
          role="tab"
          aria-selected={activeId === HOME_TAB_ID}
          className={`tab tab-home ${activeId === HOME_TAB_ID ? 'active' : ''}`}
          title={shortcut('Home (⌘1)')}
          onPointerDown={() => activate(HOME_TAB_ID)}
        >
          <span className="tab-icon">
            <LayoutGrid size={15} strokeWidth={1.75} />
          </span>
          <span className="tab-title">Home</span>
        </div>
        <div
          className="tabs-scroll"
          ref={scrollRef}
          data-tauri-drag-region
          onWheel={(e) => {
            // A mouse wheel scrolls the tabs sideways; a trackpad already does.
            if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) e.currentTarget.scrollLeft += e.deltaY
          }}
        >
          {tabs.map((tab) => {
            const active = tab.id === activeId
            const beside = !active && !!split?.includes(tab.id)
            const dragging = drag?.id === tab.id
            // Someone else's shared document has nothing unsaved here: they have the changes.
            const dirty = tab.dirty && !keeperOf(tab.id)
            return (
              <div
                key={tab.id}
                ref={(el) => {
                  if (el) tabRefs.current.set(tab.id, el)
                  else tabRefs.current.delete(tab.id)
                }}
                role="tab"
                aria-selected={active}
                className={`tab ${active ? 'active' : ''} ${beside ? 'beside' : ''} ${dirty ? 'dirty' : ''} ${dragging ? 'dragging' : ''}`}
                title={tab.path ?? tab.title}
                style={dragging ? { transform: `translateX(${drag.dx}px)` } : undefined}
                onPointerDown={(e) => startDrag(e, tab.id)}
                onAuxClick={(e) => {
                  if (e.button === 1) void closeTab(tab.id)
                }}
                onContextMenu={(e) => {
                  e.preventDefault()
                  activate(tab.id)
                  setTabMenu({ x: e.clientX, y: e.clientY, id: tab.id })
                }}
              >
                <span className="tab-icon">
                  <KindIcon kind={tab.kind} size={15} />
                </span>
                <span className="tab-title">{tab.title}</span>
                {(shared[tab.id]?.status === 'live' || shared[tab.id]?.status === 'connecting') && !shared[tab.id].private && (
                  <span className="tab-shared" title={shared[tab.id].role === 'host' ? 'Shared on your network' : 'A shared document'}>
                    <Users size={12} />
                  </span>
                )}
                <button
                  type="button"
                  className="tab-close"
                  aria-label={dirty ? `Close ${tab.title} (unsaved changes)` : `Close ${tab.title}`}
                  title={shortcut(withKeys(dirty ? 'Unsaved changes — close' : 'Close', 'close-tab'))}
                  onClick={(e) => {
                    e.stopPropagation()
                    void closeTab(tab.id)
                  }}
                >
                  <span className="dirty-dot" style={dirty ? undefined : { display: 'none' }} />
                  <X size={13} className="x" />
                </button>
              </div>
            )
          })}
        </div>
      </div>
      <IconButton
        label="New"
        className="new-tab-btn"
        aria-label="New"
        title="New document or open a file"
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect()
          setMenu({ x: r.left, y: r.bottom + 6 })
        }}
      >
        <Plus size={16} />
      </IconButton>
      <div className="titlebar-spacer" data-tauri-drag-region />
      <ChatButton />
      <ShareButton />
      <AutosaveChip />
      <UpdateButton />
      {tabMenu && (
        <PopoverMenu
          anchor={tabMenu}
          onClose={() => setTabMenu(null)}
          items={[
            { label: 'Duplicate tab', icon: <Copy size={14} />, shortcut: keysFor('duplicate-tab'), onSelect: () => void duplicateTab(tabMenu.id) },
            ...(split?.includes(tabMenu.id)
              ? [{ label: 'Close split view', icon: <Columns2 size={14} />, onSelect: () => void splitWith(null) }]
              : SPLIT_KINDS.has(tabs.find((t) => t.id === tabMenu.id)?.kind as DocKind)
                ? [{ label: 'Open to the side…', icon: <Columns2 size={14} />, onSelect: () => void openBeside(tabMenu.id) }]
                : []),
            ...(canShare(tabMenu.id) ? [{ label: shared[tabMenu.id] && !shared[tabMenu.id].private ? 'Sharing…' : isTauri ? 'Share…' : 'Share on your network…', icon: <Users size={14} />, onSelect: () => showSharePanel(tabMenu.id) }] : []),
            ...(() => {
              const t = tabs.find((x) => x.id === tabMenu.id)
              if (!t) return []
              const out: { label: string; icon?: React.ReactNode; separatorBefore?: boolean; onSelect(): void }[] = []
              if (targetsForKind(t.kind).length > 0) {
                out.push({
                  label: 'Convert file…',
                  icon: <ArrowRightLeft size={14} />,
                  separatorBefore: true,
                  onSelect: () => useConvertDialog.getState().openConvert(t.id),
                })
              }
              if (canExportMarkdown(t.kind)) {
                out.push({
                  label: 'Copy as Markdown (LLM)',
                  icon: <ClipboardCopy size={14} />,
                  onSelect: () => void copyTabAsMarkdown(t.id),
                })
              }
              if (tabs.some((x) => canIncludeInPdfBundle(x.kind))) {
                out.push({
                  label: 'Bundle open files to PDF…',
                  icon: <FileStack size={14} />,
                  onSelect: () => useConvertDialog.getState().openBundle(),
                })
              }
              return out
            })(),
            ...(isTauri && tabs.find((t) => t.id === tabMenu.id)?.path
              ? [
                  { label: revealLabel, icon: <FolderOpen size={14} />, onSelect: () => void revealInFinder(tabs.find((t) => t.id === tabMenu.id)!.path!) },
                  { label: 'Version history…', icon: <History size={14} />, onSelect: () => showVersionHistory(tabMenu.id) },
                ]
              : []),
            { label: 'Close tab', shortcut: keysFor('close-tab'), separatorBefore: true, onSelect: () => void closeTab(tabMenu.id) },
            { label: 'Close other tabs', onSelect: () => void closeOthers(tabMenu.id) },
          ]}
        />
      )}
      {menu && (
        <PopoverMenu
          anchor={menu}
          onClose={() => setMenu(null)}
          items={[
            ...enabledKinds(plugins)
              .filter((k) => k.create)
              .map((k) => ({ label: k.create!.title, icon: <KindIcon kind={k.id} />, onSelect: () => newDocument(k.id) })),
            { label: 'Open…', icon: <FolderOpen size={14} />, shortcut: keysFor('open'), onSelect: () => void openDialog(), separatorBefore: true },
          ]}
        />
      )}
    </header>
  )
}

/** Right-clicking a tab brings it to the front, so “to the side” means beside the tab that was there before. */
let previousActive: string | null = null
useWorkspace.subscribe((s, prev) => {
  if (s.activeId !== prev.activeId) previousActive = prev.activeId
})

/**
 * Split view with this tab on the right of the Word document or PDF that was in front. With
 * nothing like that to go beside, pick what goes beside it.
 */
async function openBeside(id: string) {
  const ws = useWorkspace.getState()
  const prev = previousActive && previousActive !== id ? ws.tabs.find((t) => t.id === previousActive) : undefined
  if (prev && SPLIT_KINDS.has(prev.kind)) {
    ws.activate(prev.id)
    await ws.splitWith(id)
    return
  }
  ws.activate(id)
  pickSplit(id)
}

/** Where the active tab autosaves to, and how the last autosave went; clickable when saving enables autosave. */
function AutosaveChip() {
  const tabs = useWorkspace((s) => s.tabs)
  const activeId = useWorkspace((s) => s.activeId)
  const autosave = useSettings((s) => s.autosave)
  const status = useAutosaveStatus((s) => s[activeId])
  const tab = tabs.find((t) => t.id === activeId)
  // Following someone else's shared document: the file is theirs.
  const keeper = useCollab(() => keeperOf(activeId))
  if (!tab) return null
  if (keeper !== null) {
    return (
      <span className="autosave-chip" role="status" title={`${keeper} has this document’s file and saves it. Your changes go to them as you make them.`}>
        <Users size={14} />
        <span>On {keeper}’s computer</span>
      </span>
    )
  }

  const block = autosaveBlock(tab, autosave)
  let icon = <Save size={14} />
  let text = ''
  let tone = ''
  let title = tab.path ? `Autosaves to ${tildify(tab.path)}` : 'This file has not been saved yet.'
  if (block) {
    if (block.canSaveToEnable) {
      icon = <Save size={14} />
      text = block.reason === 'legacy-ppt' ? 'Save as .pptx to autosave' : 'Save to enable autosave'
      title = `${block.message} ${shortcut('Click or press ⌘S to save.')}`
      return (
        <button
          type="button"
          className="autosave-chip"
          title={title}
          onClick={() => void useWorkspace.getState().save(tab.id)}
        >
          {icon}
          <span>{text}</span>
        </button>
      )
    }
    icon = <SaveOff size={14} />
    text = 'Not autosaved'
    title = `${block.message} ${shortcut('Save with ⌘S.')}`
  } else if (status?.state === 'error') {
    icon = <CircleAlert size={14} />
    text = 'Couldn’t autosave'
    tone = 'error'
    title = status.message ?? ''
  } else if (status?.state === 'saving') {
    text = 'Saving…'
  } else if (status?.state === 'saved' && !tab.dirty) {
    icon = <Check size={14} />
    text = 'Saved'
  }
  return (
    <span className={`autosave-chip ${tone}`} role="status" title={title}>
      {icon}
      {text && <span>{text}</span>}
    </span>
  )
}
