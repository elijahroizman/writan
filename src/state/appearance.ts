// Which color theme is used in light and in dark, the user's own themes, and the ones published
// in the themes folder (src-tauri/src/themes.rs) by people and their agents. See lib/themes.ts.
// The appearance itself (light, dark, or the system's) is `theme` in the workspace store.

import { create } from 'zustand'
import { isTauri, isWindows, listen } from '../lib/platform'
import { applyThemeCss } from '../lib/themeCss'
import type { Appearance, RoleColors, ThemeDef } from '../lib/themes'
import { DEFAULT_THEME_ID, isColor, isHex, isThemeId, parseThemeFile, PRESET_THEMES, ROLE_KEYS, THEME_CSS_MAX_LENGTH, themeVars } from '../lib/themes'
import { useWorkspace } from './workspace'

const KEY = 'workspace.appearance'

interface AppearanceState {
  /** Theme ids for each appearance. */
  light: string
  dark: string
  custom: ThemeDef[]
  /** The themes folder's, newest read. */
  published: ThemeDef[]
  /** Where the themes folder is, once known. */
  themesDir: string | null
  /** The theme editor's draft, shown in place of the theme in use while it's open. */
  preview: { theme: ThemeDef; mode: Appearance } | null
  /** One appearance, or both when `mode` is omitted. */
  use(id: string, mode?: Appearance): void
  /** Adds a theme, or replaces the custom theme with the same id. */
  save(theme: ThemeDef): void
  remove(id: string): void
  setPreview(preview: AppearanceState['preview']): void
}

/** A stored theme's colors and css, keeping only what's valid. */
function cleanVariants(value: unknown): ThemeDef['variants'] {
  if (!value || typeof value !== 'object') return undefined
  const out: NonNullable<ThemeDef['variants']> = {}
  for (const mode of ['light', 'dark'] as const) {
    const colors = (value as Record<string, unknown>)[mode]
    if (!colors || typeof colors !== 'object') continue
    const kept: RoleColors = {}
    for (const key of ROLE_KEYS) {
      const c = (colors as Record<string, unknown>)[key]
      if (isColor(c)) kept[key] = c
    }
    if (Object.keys(kept).length) out[mode] = kept
  }
  return Object.keys(out).length ? out : undefined
}

function load(): Pick<AppearanceState, 'light' | 'dark' | 'custom'> {
  const none = { light: DEFAULT_THEME_ID, dark: DEFAULT_THEME_ID, custom: [] }
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(KEY) || 'null')
    if (typeof parsed !== 'object' || parsed === null) return none
    const p = parsed as Record<string, unknown>
    const custom = (Array.isArray(p.custom) ? p.custom : [])
      .filter((t): t is ThemeDef => !!t && typeof t.id === 'string' && typeof t.name === 'string' && isHex(t.accent) && isHex(t.light) && isHex(t.dark))
      .map((t) => {
        const theme: ThemeDef = { id: t.id, name: t.name.slice(0, 48), accent: t.accent, light: t.light, dark: t.dark, custom: true }
        const variants = cleanVariants(t.variants)
        if (variants) theme.variants = variants
        if (typeof t.css === 'string' && t.css.trim()) theme.css = t.css.slice(0, THEME_CSS_MAX_LENGTH)
        return theme
      })
    // A published theme isn't read yet, so its id is kept; an unknown one shows the default.
    const known = (id: unknown) => typeof id === 'string' && (PRESET_THEMES.some((t) => t.id === id) || custom.some((t) => t.id === id) || isThemeId(id))
    return { light: known(p.light) ? (p.light as string) : DEFAULT_THEME_ID, dark: known(p.dark) ? (p.dark as string) : DEFAULT_THEME_ID, custom }
  } catch {
    return none
  }
}

export const useAppearance = create<AppearanceState>((set, get) => {
  const persist = () => {
    const { light, dark, custom } = get()
    try {
      localStorage.setItem(KEY, JSON.stringify({ light, dark, custom }))
    } catch (err) {
      console.warn('Themes weren’t saved', err)
    }
  }
  return {
    ...load(),
    published: [],
    themesDir: null,
    preview: null,
    use: (id, mode) => {
      set(mode === 'light' ? { light: id } : mode === 'dark' ? { dark: id } : { light: id, dark: id })
      persist()
    },
    save: (theme) => {
      const next: ThemeDef = { ...theme, custom: true, published: undefined, background: undefined }
      const custom = get().custom.some((t) => t.id === theme.id) ? get().custom.map((t) => (t.id === theme.id ? next : t)) : [...get().custom, next]
      set({ custom })
      persist()
    },
    remove: (id) => {
      const { light, dark } = get()
      set({ custom: get().custom.filter((t) => t.id !== id), light: light === id ? DEFAULT_THEME_ID : light, dark: dark === id ? DEFAULT_THEME_ID : dark })
      persist()
    },
    setPreview: (preview) => set({ preview }),
  }
})

/** Built-in, then published, then the user's own. A custom theme with a published one's id wins. */
export function allThemes(custom = useAppearance.getState().custom, published = useAppearance.getState().published): ThemeDef[] {
  const mine = new Set(custom.map((t) => t.id))
  return [...PRESET_THEMES, ...published.filter((t) => !mine.has(t.id)), ...custom]
}
export const themeById = (id: string, custom = useAppearance.getState().custom, published = useAppearance.getState().published) =>
  allThemes(custom, published).find((t) => t.id === id) ?? PRESET_THEMES[0]

// ── The themes folder

let reading: Promise<void> | null = null
let again: Promise<void> | null = null

/**
 * Reads the themes folder again. Files that aren't valid themes are left out. Asked while a read
 * is under way, it resolves after a fresh one, so a file just written is in it.
 */
export function refreshPublished(): Promise<void> {
  if (!isTauri) return Promise.resolve()
  if (reading) {
    again ??= reading.then(() => {
      again = null
      return refreshPublished()
    })
    return again
  }
  reading = readFolder().finally(() => {
    reading = null
  })
  return reading
}

async function readFolder() {
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    const read = await invoke<{ dir: string; themes: { id: string; text: string; background: ThemeDef['background'] | null }[] } | null>('themes_read')
    // Nothing to read (a test build's stand-in for the app answers nothing).
    if (!read || !Array.isArray(read.themes)) return
    const { dir, themes } = read
    const published: ThemeDef[] = []
    for (const entry of themes) {
      if (!isThemeId(entry.id)) continue
      try {
        published.push({ ...parseThemeFile(entry.text, entry.id), id: entry.id, published: true, ...(entry.background ? { background: entry.background } : {}) })
      } catch (err) {
        console.warn(`Theme ${entry.id}.json left out: ${(err as Error).message}`)
      }
    }
    useAppearance.setState({ themesDir: dir, published })
  } catch (err) {
    console.warn('Themes folder', err)
  }
}

/** Writes a theme into the themes folder as `<id>.json`, and reads the folder again. */
export async function publishTheme(id: string, file: object): Promise<string> {
  const { invoke } = await import('@tauri-apps/api/core')
  const path = await invoke<string>('theme_publish', { id, text: `${JSON.stringify(file, null, 2)}\n` })
  await refreshPublished()
  return path
}

// ── Putting it on the page

let applied: string[] = []
/** The background image's blob URL, by theme and revision. */
let backdrop: { key: string; url: string } | null = null
let backdropWanted = ''

/** A published theme's background image, as `--theme-background-image`. */
async function applyBackground(theme: ThemeDef) {
  const root = document.documentElement
  const bg = theme.published && theme.background
  const key = bg ? `${theme.id}:${bg.revision}` : ''
  backdropWanted = key
  if (!bg) {
    root.style.removeProperty('--theme-background-image')
    return
  }
  if (backdrop?.key !== key) {
    try {
      const { invoke } = await import('@tauri-apps/api/core')
      const data = await invoke<ArrayBuffer | number[]>('theme_background', { id: theme.id, ext: bg.ext })
      // Tauri's fallback transport sends bytes as a list of numbers.
      const bytes = data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data)
      const type = bg.ext === 'jpg' || bg.ext === 'jpeg' ? 'image/jpeg' : `image/${bg.ext}`
      const url = URL.createObjectURL(new Blob([bytes as BlobPart], { type }))
      if (backdropWanted !== key) return URL.revokeObjectURL(url)
      if (backdrop) URL.revokeObjectURL(backdrop.url)
      backdrop = { key, url }
    } catch (err) {
      console.warn('Theme background', err)
      return
    }
  }
  if (backdropWanted === key && backdrop) root.style.setProperty('--theme-background-image', `url("${backdrop.url}")`)
}

/** The theme showing now, and in which appearance. */
export function shownTheme(): { theme: ThemeDef; mode: Appearance } {
  const { light, dark, preview } = useAppearance.getState()
  if (preview) return preview
  const mode = useWorkspace.getState().theme
  return { theme: themeById(mode === 'dark' ? dark : light), mode }
}

/** Puts the appearance and its theme's colors on <html>. */
function apply() {
  const root = document.documentElement
  const { theme, mode } = shownTheme()
  root.dataset.theme = mode
  for (const name of applied) root.style.removeProperty(name)
  const vars = themeVars(theme, mode)
  for (const [name, value] of Object.entries(vars)) root.style.setProperty(name, value)
  applied = Object.keys(vars)
  applyThemeCss(theme.css)
  void applyBackground(theme)
  if (isTauri && isWindows) void paintWindowChrome()
}

/** "#rrggbb" for a CSS color, as the page draws it. */
function hexOf(color: string) {
  const probe = document.createElement('i')
  probe.style.color = color
  document.body.appendChild(probe)
  const rgb = getComputedStyle(probe).color.match(/\d+(\.\d+)?/g)?.map(Number) ?? [0, 0, 0]
  probe.remove()
  return `#${rgb
    .slice(0, 3)
    .map((v) => Math.round(v).toString(16).padStart(2, '0'))
    .join('')}`
}

/** Windows draws the title and menu bars itself; give them the tab bar's colors so the top of the window is one band. */
async function paintWindowChrome() {
  const { themeMode } = useWorkspace.getState()
  const { invoke } = await import('@tauri-apps/api/core')
  await invoke('set_window_chrome', {
    background: hexOf('var(--chrome)'),
    text: hexOf('var(--fg)'),
    hot: hexOf('var(--field-hover)'),
    disabled: hexOf('var(--fg-faint)'),
    theme: themeMode === 'system' ? null : themeMode,
  }).catch((err) => console.warn('Window colors', err))
}

/** Applies now, and again whenever the appearance or the theme changes. */
export function startAppearance() {
  apply()
  useWorkspace.subscribe((s, prev) => (s.theme !== prev.theme || s.themeMode !== prev.themeMode) && apply())
  useAppearance.subscribe(apply)
  void refreshPublished()
  void listen('themes-changed', () => void refreshPublished())
}
