import { create } from 'zustand'
import type { DocKind } from '../lib/kinds'
import { KINDS, isKnownKind, kindFromPath, stripExtension } from '../lib/kinds'
import * as platform from '../lib/platform'
import type { PluginInfo } from '../plugins/catalog'
import { pluginForKind } from '../plugins/catalog'
import { useFolder } from './folder'
import { enabledKinds, kindEnabled, usePlugins } from './plugins'
import { useSettings } from './settings'

export const HOME_TAB_ID = 'home'

/**
 * The kinds that open side by side in split view. Markdown and LaTeX have their own
 * source-and-preview split instead.
 */
export const SPLIT_KINDS: ReadonlySet<DocKind> = new Set<DocKind>(['docx', 'pdf'])

export interface DocTab {
  id: string
  kind: DocKind
  title: string
  /** Where the file lives on disk; null until an untitled document is saved. */
  path: string | null
  /** Bytes to load into the editor; null means “start a blank document”. */
  bytes: Uint8Array | null
  /** Bumped to make the editor reload `bytes` (revert, external reload). */
  loadId: number
  dirty: boolean
}

export interface RecentFile {
  path: string
  kind: DocKind
  openedAt: number
}

/** What every editor exposes to the shell (save, find, zoom, print). */
export interface EditorHandle {
  getBytes(): Promise<Uint8Array>
  /** Commit pending input before taking the save snapshot. */
  prepareSave?(): void
  /** Content revision, excluding caret and selection changes. */
  getRevision?(): number | string
  /** Called after a successful save so the editor can reset its own dirty state. */
  markSaved?(): void
  find?(): void
  /** Find with the replace field showing. */
  replace?(): void
  zoom?(direction: 'in' | 'out' | 'reset'): void
  print?(): Promise<void>
  /** Source-and-preview editors (Markdown, LaTeX): read, split, write, or cycle through them. */
  viewMode?(mode: 'read' | 'split' | 'write' | 'cycle'): void
  /**
   * True while the user is partway through an entry that saving would cut short (typing into a
   * PDF form field or text box). Autosave waits for it; the Save command doesn't.
   */
  busy?(): boolean
  /** Editor shortcuts by id (see state/shortcuts.ts). One that can't run right now answers false, and the key goes on to the document. */
  actions?: Record<string, () => boolean | void>
}

const handles = new Map<string, EditorHandle>()
export const registerEditor = (tabId: string, handle: EditorHandle) => {
  handles.set(tabId, handle)
  return () => {
    if (handles.get(tabId) === handle) handles.delete(tabId)
  }
}
export const getEditor = (tabId: string) => handles.get(tabId)

/** Counts edits per tab, so a save can tell whether the document changed while it was writing. */
const edits = new Map<string, number>()

/** Bytes last read by a tab, or a digest of its last successful write. */
type DiskState = { bytes: Uint8Array } | { digest: string }
const disk = new Map<string, DiskState>()
const saves = new Map<string, Promise<void>>()
/** Serialize each tab, including Save As dialogs and changes to its destination. */
const saving = new Map<string, Promise<unknown>>()

async function digest(bytes: Uint8Array) {
  const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes as Uint8Array<ArrayBuffer>))
  return Array.from(hash, (b) => b.toString(16).padStart(2, '0')).join('')
}

async function remember(tabId: string, bytes: Uint8Array, written: boolean) {
  disk.set(tabId, written ? { digest: await digest(bytes) } : { bytes })
}

async function unchangedOnDisk(tabId: string, path: string): Promise<boolean> {
  const known = disk.get(tabId)
  if (!known) return true
  // Compare content even when the timestamp and size match. Another tab or a sync tool may
  // replace bytes without changing either. Failed reads abort saving rather than allow a write.
  if (!(await platform.pathExists(path))) return false
  const bytes = await platform.readFile(path)
  if ('bytes' in known) return bytes.length === known.bytes.length && bytes.every((b, i) => b === known.bytes[i])
  return (await digest(bytes)) === known.digest
}

/** Keep conflict checking and writing together, including saves from different tabs. */
async function withSaveLock<T>(path: string, save: () => Promise<T>): Promise<T> {
  const previous = saves.get(path)
  let release!: () => void
  const pending = new Promise<void>((resolve) => { release = resolve })
  saves.set(path, pending)
  try {
    await previous
    return await save()
  } finally {
    release()
    if (saves.get(path) === pending) saves.delete(path)
  }
}

/** Autosave plugs in here (see lib/autosave.ts); until it does, nothing autosaves. */
export const autosaveHooks = {
  /** A tab's document just changed. */
  edited: (_tabId: string) => {},
  /** Save a tab now if autosave covers it. Resolves true once its changes are on disk. */
  flush: async (_tabId: string) => false,
}

type Theme = 'dark' | 'light'
/** What the user chose: a fixed appearance, or whatever the system is set to. */
export type ThemeMode = Theme | 'system'

interface WorkspaceState {
  tabs: DocTab[]
  activeId: string
  /**
   * Split view: the tabs in the left and right panes, or null for one pane. The focused pane's
   * tab is `activeId`; commands (save, find, zoom) go to it.
   */
  split: [string, string] | null
  /** In split view each pane's sidebar and inspector start hidden; showing one only lasts while it's split. By tab id. */
  splitPanels: Record<string, { left: boolean; right: boolean }>
  recents: RecentFile[]
  closedPaths: string[]
  /** The appearance in use: `themeMode`, with 'system' resolved. */
  theme: Theme
  themeMode: ThemeMode
  leftOpen: Record<DocKind, boolean>
  rightOpen: Record<DocKind, boolean>
  dropHover: boolean

  activate(id: string): void
  /** Show `id` beside the tab in front (a copy of it if they're the same), or leave split view with null. Word and PDF only. */
  splitWith(id: string | null): Promise<void>
  /** Drag the line between the panes: the left pane's share, 0.2–0.8. */
  setSplitRatio(ratio: number): void
  splitRatio: number
  activateOffset(delta: number): void
  activateIndex(index: number): void
  moveTab(id: string, toIndex: number): void
  openPaths(paths: string[]): Promise<void>
  openFiles(files: platform.OpenedFile[]): void
  openDialog(): Promise<void>
  /**
   * Open bytes that aren't a file yet (a plugin's compiled PDF) as an unsaved tab of the kind
   * `name`'s extension says. Passing the id this returned last time reopens that tab instead
   * (from the top, under a new id), if it's still open and unchanged. Returns the tab's id, or
   * null if no plugin opens the kind.
   */
  openUnsaved(bytes: Uint8Array, name: string, replace?: string | null): string | null
  /**
   * Replace the bytes of a tab openUnsaved made, without switching to it. Returns false once the
   * tab is gone, or saved or edited (it's the user's now, so a rebuild must not overwrite it).
   */
  refreshUnsaved(id: string, bytes: Uint8Array): boolean
  newDocument(kind: DocKind): void
  markDirty(id: string, dirty: boolean): void
  /**
   * Save a tab. `to` saves to that path without asking. `quiet` throws on failure instead of
   * showing an error (autosave reports failures itself). `label` names this version in the
   * file's version history.
   */
  save(id?: string, opts?: { saveAs?: boolean; to?: string; quiet?: boolean; label?: string }): Promise<boolean>
  /**
   * Put an earlier version back: unsaved changes are saved first (so they're in the history
   * too), then the version is saved over the file, named `label`, and the tab reloads it.
   */
  restoreVersion(id: string, bytes: Uint8Array, label: string): Promise<boolean>
  revert(id?: string): Promise<void>
  closeTab(id?: string): Promise<boolean>
  closeAll(): Promise<boolean>
  reopenClosed(): Promise<void>
  /** Open a second, independent tab on the same file (e.g. a textbook's questions and answers side by side). */
  duplicateTab(id?: string): Promise<void>
  closeOthers(id: string): Promise<void>
  removeRecent(path: string): void
  clearRecents(): void
  /** Show or hide a kind's sidebar; in split view only that pane's (`tabId`, or the focused one). */
  toggleLeft(kind?: DocKind, tabId?: string): void
  toggleRight(kind?: DocKind, tabId?: string): void
  /** Switch between light and dark (⇧⌘L): a fixed choice from then on. */
  setTheme(theme: Theme): void
  setThemeMode(mode: ThemeMode): void
  setDropHover(on: boolean): void
}

const STORAGE = {
  recents: 'workspace.recents',
  session: 'workspace.session',
  theme: 'workspace.theme',
  themeMode: 'workspace.themeMode',
  panels: 'workspace.panels',
}

function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key)
    return raw ? (JSON.parse(raw) as T) : fallback
  } catch {
    return fallback
  }
}

/** Recents from the last session, without kinds this build no longer opens. */
function loadRecents(): RecentFile[] {
  const saved = load<RecentFile[]>(STORAGE.recents, [])
  if (!Array.isArray(saved)) return []
  const recents = saved.filter((r) => r && typeof r.kind === 'string' && isKnownKind(r.kind))
  if (recents.length !== saved.length) localStorage.setItem(STORAGE.recents, JSON.stringify(recents))
  return recents
}

const defaultPanels = {
  left: Object.fromEntries(Object.values(KINDS).map((k) => [k.id, k.panels.left])) as Record<DocKind, boolean>,
  right: Object.fromEntries(Object.values(KINDS).map((k) => [k.id, k.panels.right])) as Record<DocKind, boolean>,
}

const savedPanels = load<Partial<typeof defaultPanels>>(STORAGE.panels, {})
// Kinds added since the panels were last saved take their defaults.
const initialPanels = {
  left: { ...defaultPanels.left, ...savedPanels.left },
  right: { ...defaultPanels.right, ...savedPanels.right },
}
const systemTheme = (): Theme => (typeof matchMedia !== 'undefined' && matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark')

/** An earlier build stored only a fixed light or dark; with nothing stored, follow the system. */
function loadThemeMode(): ThemeMode {
  const mode = load<unknown>(STORAGE.themeMode, null)
  if (mode === 'light' || mode === 'dark' || mode === 'system') return mode
  const old = load<unknown>(STORAGE.theme, null)
  return old === 'light' || old === 'dark' ? old : 'system'
}
const initialThemeMode = loadThemeMode()

const newId = () => Math.random().toString(36).slice(2, 10)

export const useWorkspace = create<WorkspaceState>((set, get) => {
  const activeDoc = (id?: string) => {
    const tabId = id ?? get().activeId
    return get().tabs.find((t) => t.id === tabId)
  }

  const addRecent = (path: string, kind: DocKind) => {
    if (path.startsWith('browser:')) return
    const recents = [{ path, kind, openedAt: Date.now() }, ...get().recents.filter((r) => r.path !== path)].slice(0, 30)
    set({ recents })
    localStorage.setItem(STORAGE.recents, JSON.stringify(recents))
  }

  const addTab = (tab: DocTab) => {
    const { tabs, activeId } = get()
    // Open next to the current tab, like a browser.
    const at = tabs.findIndex((t) => t.id === activeId)
    const next = [...tabs]
    next.splice(at === -1 ? tabs.length : at + 1, 0, tab)
    set({ tabs: next })
    get().activate(tab.id)
  }

  /** In split view, show or hide one pane's panel. False when not split, for the normal toggle. */
  const toggleSplitPanel = (side: 'left' | 'right', tabId = get().activeId) => {
    const { split, splitPanels } = get()
    if (!split?.includes(tabId)) return false
    const now = splitPanels[tabId] ?? { left: false, right: false }
    set({ splitPanels: { ...splitPanels, [tabId]: { ...now, [side]: !now[side] } } })
    return true
  }

  return {
    tabs: [],
    activeId: HOME_TAB_ID,
    split: null,
    splitPanels: {},
    splitRatio: 0.5,
    recents: loadRecents(),
    closedPaths: [],
    theme: initialThemeMode === 'system' ? systemTheme() : initialThemeMode,
    themeMode: initialThemeMode,
    leftOpen: initialPanels.left,
    rightOpen: initialPanels.right,
    dropHover: false,

    activate: (id) => {
      const { split, activeId, tabs } = get()
      // In split view another Word or PDF tab takes the focused pane's place; anything else leaves split view.
      if (split && !split.includes(id)) {
        const kind = tabs.find((t) => t.id === id)?.kind
        if (!kind || !SPLIT_KINDS.has(kind)) set({ split: null, splitPanels: {} })
        else set({ split: split[0] === activeId ? [id, split[1]] : [split[0], id] })
      }
      set({ activeId: id })
    },

    splitWith: async (id) => {
      const { activeId, tabs } = get()
      const kindOf = (tabId: string) => tabs.find((t) => t.id === tabId)?.kind
      if (id === null || activeId === HOME_TAB_ID || !SPLIT_KINDS.has(kindOf(activeId)!) || (id !== activeId && !SPLIT_KINDS.has(kindOf(id)!))) {
        set({ split: null, splitPanels: {} })
        return
      }
      let other = id
      if (other === activeId) {
        // The same document twice: an independent copy, so each side scrolls on its own.
        await get().duplicateTab(activeId)
        other = get().activeId
        set({ activeId })
      } else if (!tabs.some((t) => t.id === other)) return
      set({ split: [activeId, other], activeId: other, splitPanels: {} })
    },

    setSplitRatio: (ratio) => set({ splitRatio: Math.min(0.8, Math.max(0.2, ratio)) }),

    activateOffset: (delta) => {
      const ids = [HOME_TAB_ID, ...get().tabs.map((t) => t.id)]
      const i = ids.indexOf(get().activeId)
      get().activate(ids[(i + delta + ids.length) % ids.length])
    },

    activateIndex: (index) => {
      const ids = [HOME_TAB_ID, ...get().tabs.map((t) => t.id)]
      // ⌘9 always jumps to the last tab, as in Safari.
      const id = index >= 8 ? ids[ids.length - 1] : ids[index]
      if (id) get().activate(id)
    },

    moveTab: (id, toIndex) => {
      const tabs = [...get().tabs]
      const from = tabs.findIndex((t) => t.id === id)
      if (from === -1) return
      const [tab] = tabs.splice(from, 1)
      tabs.splice(Math.max(0, Math.min(toIndex, tabs.length)), 0, tab)
      set({ tabs })
    },

    openPaths: async (paths) => {
      const failures: string[] = []
      const opened: DocTab[] = []
      const needPlugin: string[] = []
      for (const path of paths) {
        const existing = get().tabs.find((t) => t.path === path)
        if (existing) {
          get().activate(existing.id)
          continue
        }
        const kind = kindFromPath(path)
        // A folder (dropped on the window, from Finder or the command line) opens in the sidebar.
        if (!kind && (await platform.isDirectory(path))) {
          useFolder.getState().open(path)
          continue
        }
        if (!kind) {
          failures.push(`“${platform.basename(path)}” isn't a file type Writan opens (${openableTypes()}).`)
          continue
        }
        if (!kindEnabled(kind)) {
          needPlugin.push(path)
          continue
        }
        if (opened.some((t) => t.path === path)) continue
        try {
          const bytes = await platform.readFile(path)
          const id = newId()
          await remember(id, bytes, false)
          opened.push({ id, kind, title: stripExtension(platform.basename(path)), path, bytes, loadId: 0, dirty: false })
        } catch (err) {
          failures.push(String(err))
          if (!(await platform.pathExists(path))) get().removeRecent(path)
        }
      }
      // Add them in one go, so only the last one is shown. The others load when first opened
      // (see EditorSlot), which keeps a restored session from loading every document at launch.
      for (const tab of opened) {
        addTab(tab)
        addRecent(tab.path!, tab.kind)
      }
      if (failures.length) void platform.showError(failures.join('\n\n'))
      if (needPlugin.length) {
        const ready = await offerPlugins(needPlugin, platform.basename)
        if (ready.length) await get().openPaths(ready)
      }
    },

    openFiles: (files) => {
      const needPlugin = files.filter((f) => {
        const kind = kindFromPath(f.name)
        return kind && !kindEnabled(kind)
      })
      if (needPlugin.length) {
        void offerPlugins(needPlugin, (f) => f.name).then((ready) => ready.length && get().openFiles(ready))
      }
      for (const file of files) {
        const kind = kindFromPath(file.name)
        if (!kind || !kindEnabled(kind)) continue
        const existing = get().tabs.find((t) => t.path === file.path)
        if (existing) {
          get().activate(existing.id)
          continue
        }
        addTab({ id: newId(), kind, title: stripExtension(file.name), path: file.path, bytes: file.bytes, loadId: 0, dirty: false })
        addRecent(file.path, kind)
      }
    },

    openDialog: async () => {
      if (!platform.isTauri) return get().openFiles(await platform.pickFiles())
      // Documents open in tabs, a folder in the sidebar.
      await get().openPaths(await platform.pickPaths())
    },

    openUnsaved: (bytes, name, replace) => {
      const kind = kindFromPath(name)
      if (!kind || !kindEnabled(kind)) return null
      const existing = replace ? get().tabs.find((t) => t.id === replace && !t.path && !t.dirty && t.kind === kind) : undefined
      if (existing) {
        // Opening it again starts at the top, so it gets a new id: a viewer finds its old place by
        // tab id (refreshUnsaved keeps the id, so a rebuild stays where you were).
        const id = newId()
        set({ tabs: get().tabs.map((t) => (t.id === existing.id ? { ...t, id, bytes, loadId: t.loadId + 1 } : t)), activeId: id })
        return id
      }
      const id = newId()
      addTab({ id, kind, title: stripExtension(name), path: null, bytes, loadId: 0, dirty: false })
      return id
    },

    refreshUnsaved: (id, bytes) => {
      const tab = get().tabs.find((t) => t.id === id)
      if (!tab || tab.path || tab.dirty) return false
      set({ tabs: get().tabs.map((t) => (t.id === id ? { ...t, bytes, loadId: t.loadId + 1 } : t)) })
      return true
    },

    newDocument: (kind) => {
      // The first free name among the open tabs, so closing or saving an untitled one frees its number.
      const taken = new Set(get().tabs.filter((t) => t.kind === kind).map((t) => t.title))
      let n = 1
      while (taken.has(n === 1 ? KINDS[kind].untitled : `${KINDS[kind].untitled} ${n}`)) n++
      const title = n === 1 ? KINDS[kind].untitled : `${KINDS[kind].untitled} ${n}`
      addTab({ id: newId(), kind, title, path: null, bytes: null, loadId: 0, dirty: false })
    },

    markDirty: (id, dirty) => {
      const tab = get().tabs.find((t) => t.id === id)
      if (!tab) return
      if (tab.dirty !== dirty) set({ tabs: get().tabs.map((t) => (t.id === id ? { ...t, dirty } : t)) })
      if (dirty) {
        edits.set(id, (edits.get(id) ?? 0) + 1)
        autosaveHooks.edited(id)
      }
    },

    save: async (id, opts) => {
      const first = activeDoc(id)
      if (!first || !getEditor(first.id)) return false
      while (saving.has(first.id)) await saving.get(first.id)
      // Register without another await, so a waiting save sees this tab's new path and editor.
      const job = (async () => {
        try {
          const tab = get().tabs.find((t) => t.id === first.id)
          const editor = getEditor(first.id)
          if (!tab || !editor) return false
          let path = opts?.to ?? tab.path
          const saveAsPpt = tab.kind === 'pptx' && !opts?.to && !!path && path.toLowerCase().endsWith('.ppt')
          if (!path || opts?.saveAs || saveAsPpt) {
            path = await platform.pickSavePath(tab.title, tab.kind)
            if (!path) return false
          }
          if (tab.kind === 'pptx' && path.toLowerCase().endsWith('.ppt')) path = path.replace(/\.ppt$/i, '.pptx')
          const savePath = path
          return await withSaveLock(savePath, async () => {
            editor.prepareSave?.()
            const editsBefore = edits.get(tab.id) ?? 0
            const revisionBefore = editor.getRevision?.()
            const bytes = await editor.getBytes()
            // Another app, or another tab on this file, changed it since: ask before writing over that.
            if (savePath === tab.path && !(await unchangedOnDisk(tab.id, savePath))) {
              if (opts?.quiet) throw new Error(`“${tab.title}” was changed in another app or tab. Save it yourself to choose which to keep.`)
              const ok = await platform.confirmDialog('It was changed in another app or another tab since you opened it here. Saving replaces those changes with this version.', {
                title: `“${tab.title}” changed on disk`,
                okLabel: 'Save Anyway',
                danger: true,
              })
              if (!ok) return false
            }
            await platform.writeFile(savePath, bytes, { history: useSettings.getState().history, label: opts?.label })
            await remember(tab.id, bytes, true)
            const title = stripExtension(platform.basename(savePath))
            const dirty = (edits.get(tab.id) ?? 0) !== editsBefore || editor.getRevision?.() !== revisionBefore || getEditor(tab.id) !== editor
            if (!dirty) editor.markSaved?.()
            set({ tabs: get().tabs.map((t) => (t.id === tab.id ? { ...t, path: savePath, title, dirty } : t)) })
            addRecent(savePath, tab.kind)
            return true
          })
        } catch (err) {
          if (opts?.quiet) throw err
          await platform.showError(String(err))
          return false
        }
      })()
      saving.set(first.id, job.catch(() => {}))
      try {
        return await job
      } finally {
        saving.delete(first.id)
      }
    },

    restoreVersion: async (id, bytes, label) => {
      const tab = get().tabs.find((t) => t.id === id)
      if (!tab?.path) return false
      if (tab.dirty && (!(await get().save(id)) || get().tabs.find((t) => t.id === id)?.dirty)) return false
      try {
        await platform.writeFile(tab.path, bytes, { history: true, label })
        await remember(id, bytes, false)
      } catch (err) {
        await platform.showError(String(err))
        return false
      }
      set({ tabs: get().tabs.map((t) => (t.id === id ? { ...t, bytes, loadId: t.loadId + 1, dirty: false } : t)) })
      return true
    },

    revert: async (id) => {
      const tab = activeDoc(id)
      if (!tab?.path) return
      if (tab.dirty) {
        const ok = await platform.confirmDialog(`Revert “${tab.title}” to the last saved version? Your unsaved changes will be lost.`, {
          okLabel: 'Revert',
          danger: true,
        })
        if (!ok) return
      }
      try {
        const bytes = await platform.readFile(tab.path)
        await remember(tab.id, bytes, false)
        set({
          tabs: get().tabs.map((t) => (t.id === tab.id ? { ...t, bytes, loadId: t.loadId + 1, dirty: false } : t)),
        })
      } catch (err) {
        await platform.showError(String(err))
      }
    },

    closeTab: async (id) => {
      const tabId = id ?? get().activeId
      if (tabId === HOME_TAB_ID) return true
      const tab = get().tabs.find((t) => t.id === tabId)
      if (!tab) return true
      // Autosaved tabs close without asking once their last changes are written.
      if (tab.dirty && !(await autosaveHooks.flush(tab.id))) {
        get().activate(tab.id)
        const choice = await platform.askSaveChanges(tab.title)
        if (choice === 'cancel') return false
        if (choice === 'save' && (!(await get().save(tab.id)) || get().tabs.find((t) => t.id === tab.id)?.dirty)) return false
      }
      const { tabs, activeId, split } = get()
      const index = tabs.findIndex((t) => t.id === tabId)
      const remaining = tabs.filter((t) => t.id !== tabId)
      let nextActive = activeId
      if (split?.includes(tabId)) {
        // Closing one side of a split leaves the other.
        nextActive = split[0] === tabId ? split[1] : split[0]
        set({ split: null, splitPanels: {} })
      } else if (activeId === tabId) {
        const neighbour = remaining[index] ?? remaining[index - 1]
        nextActive = neighbour?.id ?? HOME_TAB_ID
      }
      // Save from the close prompt may have just given an untitled document a path.
      const path = get().tabs.find((t) => t.id === tabId)?.path ?? tab.path
      const closedPaths = path ? [...get().closedPaths, path].slice(-20) : get().closedPaths
      edits.delete(tabId)
      disk.delete(tabId)
      set({ tabs: remaining, activeId: nextActive, closedPaths })
      return true
    },

    closeAll: async () => {
      for (const tab of [...get().tabs]) {
        if (!(await get().closeTab(tab.id))) return false
      }
      return true
    },

    duplicateTab: async (id) => {
      const tab = activeDoc(id)
      if (!tab) return
      try {
        // Unsaved edits come along (and the copy starts out unsaved too); otherwise read the file fresh.
        const fromEditor = tab.dirty || !tab.path || tab.path.startsWith('browser:')
        const bytes = fromEditor ? ((await getEditor(tab.id)?.getBytes()) ?? tab.bytes) : await platform.readFile(tab.path!)
        const base = tab.title.replace(/ \(\d+\)$/, '')
        const copies = get().tabs.filter((t) => t.path === tab.path && t.path !== null).length
        const id = newId()
        // A copy read fresh starts from what's on disk now; one with unsaved edits, from what the original last read.
        if (!fromEditor && bytes) await remember(id, bytes, false)
        else if (disk.has(tab.id)) disk.set(id, disk.get(tab.id)!)
        addTab({ ...tab, id, title: tab.path ? `${base} (${copies + 1})` : `${base} copy`, bytes, loadId: 0, dirty: tab.dirty })
      } catch (err) {
        await platform.showError(String(err))
      }
    },

    closeOthers: async (id) => {
      for (const tab of get().tabs.filter((t) => t.id !== id)) {
        if (!(await get().closeTab(tab.id))) return
      }
      set({ activeId: id, split: null, splitPanels: {} })
    },

    reopenClosed: async () => {
      const closedPaths = [...get().closedPaths]
      const path = closedPaths.pop()
      set({ closedPaths })
      if (path) await get().openPaths([path])
    },

    removeRecent: (path) => {
      const recents = get().recents.filter((r) => r.path !== path)
      set({ recents })
      localStorage.setItem(STORAGE.recents, JSON.stringify(recents))
    },

    clearRecents: () => {
      set({ recents: [], closedPaths: [] })
      localStorage.setItem(STORAGE.recents, '[]')
      useFolder.getState().pruneRecent()
    },

    toggleLeft: (kind, tabId) => {
      if (toggleSplitPanel('left', tabId)) return
      const k = kind ?? activeDoc()?.kind
      if (!k) return
      set({ leftOpen: { ...get().leftOpen, [k]: !get().leftOpen[k] } })
      persistPanels()
    },

    toggleRight: (kind, tabId) => {
      if (toggleSplitPanel('right', tabId)) return
      const k = kind ?? activeDoc()?.kind
      if (!k) return
      set({ rightOpen: { ...get().rightOpen, [k]: !get().rightOpen[k] } })
      persistPanels()
    },

    setTheme: (theme) => get().setThemeMode(theme),

    setThemeMode: (mode) => {
      set({ themeMode: mode, theme: mode === 'system' ? systemTheme() : mode })
      localStorage.setItem(STORAGE.themeMode, JSON.stringify(mode))
    },

    setDropHover: (dropHover) => set({ dropHover }),
  }
})

// While the mode is 'system', the app follows the system as it changes.
if (typeof matchMedia !== 'undefined') {
  matchMedia('(prefers-color-scheme: light)').addEventListener('change', () => {
    const { themeMode } = useWorkspace.getState()
    if (themeMode === 'system') useWorkspace.setState({ theme: systemTheme() })
  })
}

/** “Word, PowerPoint, PDF, Markdown or plain text”, from the plugins that are on. */
function openableTypes() {
  const names = enabledKinds().map((k) => k.prose)
  return names.length > 1 ? `${names.slice(0, -1).join(', ')} or ${names[names.length - 1]}` : names.join('')
}

/** Files whose plugin is off: offer to turn each plugin on. Resolves to the files that can open now. */
async function offerPlugins<T>(files: T[], name: (file: T) => string): Promise<T[]> {
  const ready: T[] = []
  const byPlugin = new Map<PluginInfo, T[]>()
  for (const file of files) {
    const plugin = pluginForKind(kindFromPath(name(file))!)
    byPlugin.set(plugin, [...(byPlugin.get(plugin) ?? []), file])
  }
  for (const [plugin, group] of byPlugin) {
    const what = group.length === 1 ? `“${name(group[0])}” needs` : `${group.length} files need`
    const ok = await platform.confirmDialog(`${what} the ${plugin.name} plugin. Turn it on?`, { okLabel: 'Turn On', kind: 'info' })
    if (!ok) continue
    if (await usePlugins.getState().enable(plugin.id)) ready.push(...group)
    else await platform.showError(usePlugins.getState().errors[plugin.id] ?? `Couldn't turn on ${plugin.name}.`)
  }
  return ready
}

function persistPanels() {
  const { leftOpen, rightOpen } = useWorkspace.getState()
  localStorage.setItem(STORAGE.panels, JSON.stringify({ left: leftOpen, right: rightOpen }))
}

// Read once at startup, before anything can overwrite it.
const initialSession = load<{ paths: string[]; active: string | null }>(STORAGE.session, { paths: [], active: null })
let sessionRestored = false

/**
 * Forget files and folders last opened longer ago than Settings › General allows (1, 3, 7 or 30
 * days). Runs at launch, every hour and when the setting changes.
 */
export function pruneRecents() {
  const before = Date.now() - useSettings.getState().recentDays * 86_400_000
  const ws = useWorkspace.getState()
  const recents = ws.recents.filter((r) => r.openedAt >= before)
  if (recents.length !== ws.recents.length) {
    useWorkspace.setState({ recents })
    localStorage.setItem(STORAGE.recents, JSON.stringify(recents))
  }
  useFolder.getState().pruneRecent(before)
}

/** Remember which files were open so the next launch picks up where you left off. */
export function persistSession() {
  if (!sessionRestored) return
  const { tabs, activeId } = useWorkspace.getState()
  const paths = tabs.filter((t) => t.path && !t.path.startsWith('browser:')).map((t) => t.path as string)
  const active = tabs.find((t) => t.id === activeId)?.path ?? null
  localStorage.setItem(STORAGE.session, JSON.stringify({ paths, active }))
}

export async function restoreSession() {
  try {
    await reopen(initialSession)
  } finally {
    sessionRestored = true
    persistSession()
  }
}

async function reopen(session: { paths: string[]; active: string | null }) {
  if (!session.paths.length) return
  const existing = []
  for (const path of session.paths) {
    // A kind this build no longer has (a plugin that was removed) is left out, with no error.
    if (!kindFromPath(path)) continue
    if (await platform.pathExists(path)) existing.push(path)
  }
  const { openPaths } = useWorkspace.getState()
  // Open in order without moving focus around, then focus the tab that was active.
  useWorkspace.setState({ activeId: HOME_TAB_ID })
  await openPaths(existing)
  const state = useWorkspace.getState()
  const active = state.tabs.find((t) => t.path === session.active)
  useWorkspace.setState({ activeId: active?.id ?? HOME_TAB_ID })
}
