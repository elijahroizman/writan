import { create } from 'zustand'
import { basename, pickFolder } from '../lib/platform'
import { track } from './analytics'

/**
 * The open folder, like a VS Code workspace: its files in a sidebar beside the tabs. One folder
 * at a time; the last few are offered again on Home. Reopens at launch with the folders you'd
 * expanded.
 */
interface FolderState {
  root: string | null
  /** Folders expanded in the tree. */
  expanded: string[]
  /** The sidebar is showing (the folder stays open while it's hidden). */
  visible: boolean
  /** List every file, not only the ones Writan opens. */
  showAll: boolean
  /** Folders opened lately, newest first; Home lists them with the recent files. */
  recent: RecentFolder[]
  /** Bumped to make the tree read the disk again. */
  rev: number
  open(path: string): void
  close(): void
  toggleVisible(): void
  toggleDir(path: string, open?: boolean): void
  collapseAll(): void
  setShowAll(on: boolean): void
  refresh(): void
  forgetRecent(path: string): void
  /** Forget folders last opened before this time (ms since 1970); all of them with no time. */
  pruneRecent(before?: number): void
}

export interface RecentFolder {
  path: string
  openedAt: number
}

const KEY = 'workspace.folder'

function load() {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) || '{}') as Partial<Record<'root' | 'expanded' | 'visible' | 'showAll' | 'recent', unknown>>
    const strings = (v: unknown) => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [])
    // The dev preview kept only paths: count those as opened now.
    const recent = Array.isArray(saved.recent)
      ? saved.recent.flatMap((r: unknown): RecentFolder[] => {
          if (typeof r === 'string') return [{ path: r, openedAt: Date.now() }]
          const f = r as Partial<RecentFolder> | null
          return f && typeof f.path === 'string' && typeof f.openedAt === 'number' ? [{ path: f.path, openedAt: f.openedAt }] : []
        })
      : []
    return {
      root: typeof saved.root === 'string' ? saved.root : null,
      expanded: strings(saved.expanded),
      visible: saved.visible !== false,
      showAll: saved.showAll === true,
      recent: recent.slice(0, 8),
    }
  } catch {
    return { root: null, expanded: [], visible: true, showAll: false, recent: [] }
  }
}

const persist = () => {
  const { root, expanded, visible, showAll, recent } = useFolder.getState()
  localStorage.setItem(KEY, JSON.stringify({ root, expanded, visible, showAll, recent }))
}

export const useFolder = create<FolderState>((set, get) => ({
  ...load(),
  rev: 0,
  open: (path) => {
    const same = get().root === path
    if (!same) track('feature_used', { feature: 'open_folder' })
    set({
      root: path,
      visible: true,
      expanded: same ? get().expanded : [],
      recent: [{ path, openedAt: Date.now() }, ...get().recent.filter((r) => r.path !== path)].slice(0, 8),
      rev: get().rev + 1,
    })
    persist()
  },
  close: () => {
    set({ root: null, expanded: [] })
    persist()
  },
  toggleVisible: () => {
    set({ visible: !get().visible })
    persist()
  },
  toggleDir: (path, open) => {
    const has = get().expanded.includes(path)
    const want = open ?? !has
    if (want === has) return
    set({ expanded: want ? [...get().expanded, path] : get().expanded.filter((p) => p !== path) })
    persist()
  },
  collapseAll: () => {
    set({ expanded: [] })
    persist()
  },
  setShowAll: (showAll) => {
    set({ showAll })
    persist()
  },
  refresh: () => set({ rev: get().rev + 1 }),
  forgetRecent: (path) => {
    set({ recent: get().recent.filter((r) => r.path !== path) })
    persist()
  },
  pruneRecent: (before = Infinity) => {
    const recent = get().recent.filter((r) => r.openedAt >= before)
    if (recent.length === get().recent.length) return
    set({ recent })
    persist()
  },
}))

/** Ask for a folder and open it. */
export async function chooseFolder() {
  const path = await pickFolder()
  if (path) useFolder.getState().open(path)
}

export const folderName = (path: string) => basename(path.replace(/[\\/]+$/, '')) || path
