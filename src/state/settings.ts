import { create } from 'zustand'

export type AppIcon = 'dark' | 'blue' | 'light'

export interface AutosaveSettings {
  /** When false, nothing autosaves. Files still save with the Save command. */
  enabled: boolean
  /** Seconds to wait after the last edit before saving. */
  delay: number
  /** By kind id. A kind not listed here (a plugin's) autosaves. */
  kinds: Record<string, boolean>
}

export interface Settings {
  appIcon: AppIcon
  autosave: AutosaveSettings
  /** Seconds after typing stops before a slow preview (a LaTeX compile) rebuilds. */
  previewDelay: number
  /** Keep a copy of each saved version (see src-tauri/src/history.rs). */
  history: boolean
  /** Days a file or folder stays in Home's Recent list after it was last opened. */
  recentDays: number
}

export const DEFAULT_SETTINGS: Settings = {
  appIcon: 'dark',
  autosave: {
    enabled: true,
    delay: 3,
    kinds: {},
  },
  previewDelay: 3,
  history: true,
  recentDays: 3,
}

const KEY = 'workspace.settings'
/**
 * Stored with the settings. Before 2, PDFs started with autosave off, so a stored `pdf: false`
 * was almost always that default rather than a choice; version 1 settings drop it.
 */
const VERSION = 2

/** The delays Settings offers. Anything else in storage is ignored. */
const DELAYS = [1, 3, 10, 30]
export const PREVIEW_DELAYS = [1, 2, 3, 5, 10]
export const RECENT_DAYS = [1, 3, 7, 30]

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function load(): Settings {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(KEY) || '{}')
    if (!isRecord(parsed)) return DEFAULT_SETTINGS
    const appIcon = parsed.appIcon === 'blue' || parsed.appIcon === 'light' ? parsed.appIcon : DEFAULT_SETTINGS.appIcon
    const previewDelay = typeof parsed.previewDelay === 'number' && PREVIEW_DELAYS.includes(parsed.previewDelay) ? parsed.previewDelay : DEFAULT_SETTINGS.previewDelay
    const history = typeof parsed.history === 'boolean' ? parsed.history : DEFAULT_SETTINGS.history
    const recentDays = typeof parsed.recentDays === 'number' && RECENT_DAYS.includes(parsed.recentDays) ? parsed.recentDays : DEFAULT_SETTINGS.recentDays
    if (!isRecord(parsed.autosave)) return { ...DEFAULT_SETTINGS, appIcon, previewDelay, history, recentDays }
    const old = parsed.autosave
    const defaults = DEFAULT_SETTINGS.autosave
    const kinds = { ...defaults.kinds }
    if (isRecord(old.kinds)) {
      for (const kind of Object.keys(old.kinds)) {
        if (typeof old.kinds[kind] === 'boolean') kinds[kind] = old.kinds[kind]
      }
    }
    if (parsed.version !== VERSION && kinds.pdf === false) delete kinds.pdf
    const delay = typeof old.delay === 'number' && DELAYS.includes(old.delay) ? old.delay : defaults.delay
    // A build from before `enabled` stored the choice as whether a folder was picked.
    let enabled = defaults.enabled
    if (typeof old.enabled === 'boolean') enabled = old.enabled
    else if ('root' in old) enabled = typeof old.root === 'string' && old.root.length > 0
    return { autosave: { enabled, delay, kinds }, appIcon, previewDelay, history, recentDays }
  } catch {
    return DEFAULT_SETTINGS
  }
}

export type SettingsPage = 'general' | 'plugins' | 'autosave' | 'appearance' | 'ai' | 'shortcuts' | 'licenses'

interface SettingsState extends Settings {
  /** Whether the Settings window is showing. */
  open: boolean
  /** The page it shows. */
  page: SettingsPage
  setOpen(open: boolean, page?: SettingsPage): void
  setPage(page: SettingsPage): void
  setAutosave(patch: Partial<AutosaveSettings>): void
  setPreviewDelay(seconds: number): void
  setHistory(on: boolean): void
  setRecentDays(days: number): void
  setAppIcon(icon: AppIcon): void
}

const persist = () => {
  const { autosave, appIcon, previewDelay, history, recentDays } = useSettings.getState()
  localStorage.setItem(KEY, JSON.stringify({ version: VERSION, autosave, appIcon, previewDelay, history, recentDays } satisfies Settings & { version: number }))
}

export const useSettings = create<SettingsState>((set, get) => ({
  ...load(),
  open: false,
  page: 'general',
  // Settings always opens on General, wherever it was left.
  setOpen: (open, page) => set(open ? { open, page: page ?? 'general' } : { open }),
  setPage: (page) => set({ page }),
  setAutosave: (patch) => {
    set({ autosave: { ...get().autosave, ...patch } })
    persist()
  },
  setPreviewDelay: (previewDelay) => {
    set({ previewDelay })
    persist()
  },
  setHistory: (history) => {
    set({ history })
    persist()
  },
  setRecentDays: (recentDays) => {
    set({ recentDays })
    persist()
  },
  setAppIcon: (appIcon) => {
    set({ appIcon })
    persist()
  },
}))
