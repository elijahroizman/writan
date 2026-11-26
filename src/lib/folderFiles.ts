import { create } from 'zustand'
import { KINDS, kindFromPath } from './kinds'
import { basename, dirname, walkDir } from './platform'
import { folderName, useFolder } from '../state/folder'
import { addPaletteSource } from '../state/modals'
import { useWorkspace } from '../state/workspace'

/** Every extension some plugin opens (on or off: opening one offers to turn its plugin on). */
const OPENABLE = [...new Set(Object.values(KINDS).flatMap((k) => k.extensions))]

let walked: { key: string; files: Promise<string[]> } | null = null

/** Every document under the open folder (cached until a refresh). */
export function folderFiles(root: string, rev = useFolder.getState().rev) {
  const key = `${root}\n${rev}`
  if (walked?.key !== key) walked = { key, files: walkDir(root, OPENABLE).catch(() => []) }
  return walked.files
}

export const usePaletteSourceRev = create<{ rev: number }>(() => ({ rev: 0 }))

export function relativeDir(root: string, path: string) {
  const dir = dirname(path)
  return dir.length > root.length ? dir.slice(root.length + 1) : ''
}

/**
 * The open folder's documents for the command palette (⌘K). Marks stale on folder changes and
 * walks disk lazily only when the palette actually reads sources.
 */
export function startFolderPalette() {
  let files: string[] = []
  let loadedKey = ''
  let pendingKey = ''

  const ensureLoaded = () => {
    const s = useFolder.getState()
    if (!s.root) {
      files = []
      loadedKey = ''
      return null
    }
    const key = `${s.root}\n${s.rev}`
    if (loadedKey !== key && pendingKey !== key) {
      pendingKey = key
      const targetRoot = s.root
      void folderFiles(targetRoot, s.rev).then((f) => {
        if (pendingKey !== key) return
        const cur = useFolder.getState()
        if (cur.root === targetRoot && `${cur.root}\n${cur.rev}` === key) {
          files = f
          loadedKey = key
          usePaletteSourceRev.setState((st) => ({ rev: st.rev + 1 }))
        }
      })
    }
    return { root: s.root, files }
  }

  const unsubscribe = useFolder.subscribe((s, prev) => {
    if (s.root !== prev.root || s.rev !== prev.rev) {
      pendingKey = ''
      if (!s.root) {
        files = []
        loadedKey = ''
      }
    }
  })

  const removeSource = addPaletteSource(() => {
    const info = ensureLoaded()
    if (!info) return []
    return info.files.map((path) => ({
      id: `folder:${path}`,
      label: basename(path),
      detail: relativeDir(info.root, path) || folderName(info.root),
      group: 'Folder' as const,
      path,
      kind: kindFromPath(path) ?? undefined,
      run: () => useWorkspace.getState().openPaths([path]),
    }))
  })

  return () => {
    unsubscribe()
    removeSource()
  }
}
