import type { ReactNode } from 'react'
import { create } from 'zustand'
import type { ConvertFormat } from '../lib/convertMeta'
import type { DocKind } from '../lib/kinds'
import * as platform from '../lib/platform'
import type { ThemeDef } from '../lib/themes'
import { useSettings } from './settings'
import { useWorkspace } from './workspace'

// ── Command palette

export interface Picker {
  placeholder: string
  items(): PaletteItem[]
}

export interface PaletteItem {
  id: string
  label: string
  detail?: string
  group: 'Tabs' | 'Recent' | 'Folder' | 'Open' | 'Commands'
  path?: string
  icon?: ReactNode
  kind?: DocKind
  keys?: string
  keywords?: string
  disabled?: boolean
  run(): void | Promise<void>
}

export const usePalette = create<{
  open: boolean
  query: string
  picker: Picker | null
  setOpen(open: boolean, query?: string): void
  pick(picker: Picker): void
}>((set) => ({
  open: false,
  query: '',
  picker: null,
  setOpen: (open, query = '') => set({ open, query, picker: null }),
  pick: (picker) => set({ open: true, query: '', picker }),
}))

const sources: (() => PaletteItem[])[] = []
export const sourceItems = () => sources.flatMap((s) => s())
export function addPaletteSource(source: () => PaletteItem[]) {
  sources.push(source)
  return () => {
    const i = sources.indexOf(source)
    if (i !== -1) sources.splice(i, 1)
  }
}

// ── Convert dialog

export type ConvertModalMode = 'convert' | 'bundle'

interface ConvertModalState {
  open: boolean
  mode: ConvertModalMode
  tabId: string | null
  format: ConvertFormat | null
  openConvert(tabId?: string | null, format?: ConvertFormat): void
  openBundle(): void
  close(): void
}

export const useConvertDialog = create<ConvertModalState>((set) => ({
  open: false,
  mode: 'convert',
  tabId: null,
  format: null,
  openConvert: (tabId = null, format) => set({ open: true, mode: 'convert', tabId, format: format ?? null }),
  openBundle: () => set({ open: true, mode: 'bundle', tabId: null, format: null }),
  close: () => set({ open: false, tabId: null, format: null }),
}))

// ── Version history

export const useVersionHistory = create<{ tabId: string | null; show(tabId: string | null): void }>((set) => ({
  tabId: null,
  show: (tabId) => set({ tabId }),
}))

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

export function formatVersionSize(n: number) {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`
  return `${(n / (1024 * 1024)).toFixed(1)} MB`
}

// ── New file picker

export const useNewFile = create<{ open: boolean; setOpen(open: boolean): void }>((set) => ({
  open: false,
  setOpen: (open) => set({ open }),
}))

// ── Theme editor

export interface ThemeEditorSession {
  theme: ThemeDef
  editing: boolean
  key: number
}

export const useThemeEditor = create<{ session: ThemeEditorSession | null; open(theme: ThemeDef, editing: boolean): void; close(): void }>((set) => ({
  session: null,
  open: (theme, editing) => set({ session: { theme: structuredClone(theme), editing, key: Date.now() } }),
  close: () => set({ session: null }),
}))

// ── Share panel & join dialog

export const useSharePanel = create<{ tabId: string | null; joining: boolean }>(() => ({ tabId: null, joining: false }))
export const showSharePanel = (tabId = useWorkspace.getState().activeId) => useSharePanel.setState({ tabId })
export const showJoinDialog = () => useSharePanel.setState({ joining: true, tabId: null })
