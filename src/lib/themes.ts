// Color themes. A theme is an accent plus two tones: one tints the light appearance and one the
// dark. The surfaces, borders and text of each appearance are worked out from its tone (a fixed
// ladder of lightnesses, so text stays readable whatever color the user picks), and the accent
// replaces the app's blue. The default theme sets nothing and keeps the colors in app.css.
//
// On top of that, a theme can set any color role itself for one appearance (`variants`), and
// carry its own CSS (lib/themeCss.ts). Themes are saved in this browser (custom), or come from
// the themes folder, where people and their agents publish them (state/appearance.ts).

export interface ThemeDef {
  id: string
  name: string
  /** #rrggbb. Buttons, selections and focus rings. */
  accent: string
  /** #rrggbb. Tints the light appearance's surfaces. */
  light: string
  /** #rrggbb. Tints the dark appearance's surfaces. */
  dark: string
  /** Colors set by hand, over the ones worked out from the tones, for each appearance. */
  variants?: Partial<Record<Appearance, RoleColors>>
  /** Styles applied after the app's own. */
  css?: string
  /** Saved in this browser: it can be edited and deleted. */
  custom?: boolean
  /** From the themes folder. */
  published?: boolean
  /** A published theme's background image: the file's extension and revision. */
  background?: { ext: string; revision: string }
}

export type Appearance = 'light' | 'dark'

export const DEFAULT_THEME_ID = 'workspace'

export const PRESET_THEMES: ThemeDef[] = [
  { id: DEFAULT_THEME_ID, name: 'Writan', accent: '#4f8cff', light: '#e6e6e6', dark: '#1c1c1c' },
  { id: 'iris', name: 'Iris', accent: '#6d5ce8', light: '#cfc8f7', dark: '#221d3d' },
  { id: 'rose', name: 'Rose', accent: '#d6477f', light: '#f5cddd', dark: '#36182a' },
  { id: 'grove', name: 'Grove', accent: '#2f9a64', light: '#c9e5cf', dark: '#16301f' },
  { id: 'ocean', name: 'Ocean', accent: '#2f7fc8', light: '#c5dfee', dark: '#112d40' },
  { id: 'ember', name: 'Ember', accent: '#d9692a', light: '#f3d2bd', dark: '#35200f' },
]

// ── Color roles: the app's CSS custom properties a theme can set

export interface RoleDef {
  key: string
  /** The custom property it sets. */
  css: string
  label: string
  group: 'Surfaces' | 'Text' | 'Lines' | 'Accent' | 'Status' | 'Documents'
  /** Where it shows, for the editor. */
  hint: string
}

export const ROLES = [
  { key: 'canvas', css: '--canvas', label: 'Background', group: 'Surfaces', hint: 'Behind documents and Home' },
  { key: 'chrome', css: '--chrome', label: 'Title bar', group: 'Surfaces', hint: 'The tab bar and the window around everything' },
  { key: 'panel', css: '--panel', label: 'Sidebars', group: 'Surfaces', hint: 'Sidebars, the inspector and Settings' },
  { key: 'raised', css: '--raised', label: 'Raised', group: 'Surfaces', hint: 'Menus, cards, dialogs and popovers' },
  { key: 'tabActive', css: '--tab-active', label: 'Active tab', group: 'Surfaces', hint: 'The tab you’re on' },
  { key: 'field', css: '--field', label: 'Fields', group: 'Surfaces', hint: 'Text boxes, buttons and pickers' },
  { key: 'fieldHover', css: '--field-hover', label: 'Fields on hover', group: 'Surfaces', hint: 'A field or button under the pointer' },
  { key: 'hover', css: '--hover', label: 'Hover', group: 'Surfaces', hint: 'A row or icon under the pointer' },
  { key: 'press', css: '--press', label: 'Pressed', group: 'Surfaces', hint: 'A row or icon being clicked' },
  { key: 'fg', css: '--fg', label: 'Text', group: 'Text', hint: 'Most text and icons' },
  { key: 'fgMuted', css: '--fg-muted', label: 'Secondary text', group: 'Text', hint: 'Hints, labels and quieter icons' },
  { key: 'fgFaint', css: '--fg-faint', label: 'Faint text', group: 'Text', hint: 'Placeholders and the quietest details' },
  { key: 'line', css: '--line', label: 'Borders', group: 'Lines', hint: 'Dividers and outlines' },
  { key: 'lineSoft', css: '--line-soft', label: 'Soft borders', group: 'Lines', hint: 'Lighter dividers inside panels' },
  { key: 'scrollbar', css: '--scrollbar', label: 'Scrollbars', group: 'Lines', hint: 'Scrollbar thumbs' },
  { key: 'accent', css: '--accent', label: 'Accent', group: 'Accent', hint: 'Primary buttons, switches and focus rings' },
  { key: 'accentFg', css: '--accent-fg', label: 'Text on accent', group: 'Accent', hint: 'Text on primary buttons' },
  { key: 'accentSoft', css: '--accent-soft', label: 'Selection', group: 'Accent', hint: 'Selected rows and selected text' },
  { key: 'focusStrong', css: '--focus-strong', label: 'Strong focus', group: 'Accent', hint: 'Focus rings in menus and on light fills' },
  { key: 'danger', css: '--danger', label: 'Danger', group: 'Status', hint: 'Errors and destructive buttons' },
  { key: 'menuDanger', css: '--menu-danger', label: 'Danger in menus', group: 'Status', hint: 'Delete and similar menu items' },
  { key: 'success', css: '--success', label: 'Success', group: 'Status', hint: 'Done and connected' },
  { key: 'mdPaper', css: '--md-paper', label: 'Markdown page', group: 'Documents', hint: 'The page behind Markdown text' },
  { key: 'mdCodeBg', css: '--md-code-bg', label: 'Markdown code', group: 'Documents', hint: 'Code blocks in Markdown' },
  { key: 'mdCode', css: '--md-code', label: 'Markdown code text', group: 'Documents', hint: 'Text in Markdown code blocks' },
  { key: 'txtPaper', css: '--txt-paper', label: 'Text page', group: 'Documents', hint: 'The page behind plain text files' },
] as const satisfies readonly RoleDef[]

export type Role = (typeof ROLES)[number]['key']
export type RoleColors = Partial<Record<Role, string>>

export const ROLE_KEYS: readonly Role[] = ROLES.map((r) => r.key)
const isRole = (key: string): key is Role => (ROLE_KEYS as readonly string[]).includes(key)

// ── Colors

const HEX = /^#[0-9a-f]{6}$/i
export const isHex = (value: unknown): value is string => typeof value === 'string' && HEX.test(value)

const NUMBERS = String.raw`[\d.\s,%/+-]*`
/** A color a role may hold: hex, or rgb() and hsl() with plain numbers. Nothing that can load a file. */
const COLOR = new RegExp(String.raw`^(#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})|(?:rgba?|hsla?)\(${NUMBERS}\))$`, 'i')
export const isColor = (value: unknown): value is string => typeof value === 'string' && value.length <= 64 && COLOR.test(value.trim())

/** `#rrggbb` from `#rgb`, or null for anything else. */
export function toHex6(value: string): string | null {
  const v = value.trim().toLowerCase()
  if (/^#[0-9a-f]{6}$/.test(v)) return v
  if (/^#[0-9a-f]{3}$/.test(v)) return `#${[...v.slice(1)].map((c) => c + c).join('')}`
  return null
}

function rgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16)
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255]
}

function toHsl(hex: string): [number, number, number] {
  const [r, g, b] = rgb(hex).map((v) => v / 255)
  const max = Math.max(r, g, b)
  const min = Math.min(r, g, b)
  const l = (max + min) / 2
  if (max === min) return [0, 0, l]
  const d = max - min
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
  const h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4
  return [h * 60, s, l]
}

/** h in degrees, s and l from 0 to 1. */
function hsl(h: number, s: number, l: number) {
  const k = (n: number) => (n + h / 30) % 12
  const a = s * Math.min(l, 1 - l)
  const f = (n: number) => l - a * Math.max(-1, Math.min(k(n) - 3, 9 - k(n), 1))
  return `#${[f(0), f(8), f(4)].map((v) => Math.round(v * 255).toString(16).padStart(2, '0')).join('')}`
}

/** Black or white, whichever reads on `hex`. */
function textOn(hex: string) {
  const [r, g, b] = rgb(hex).map((v) => {
    const c = v / 255
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  })
  return 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.45 ? '#111111' : '#ffffff'
}

/** A tone for the other appearance, in the same hue, for a theme that gives only one. */
export function otherTone(tone: string, mode: Appearance): string {
  const [h, s] = toHsl(tone)
  return mode === 'dark' ? hsl(h, Math.min(0.75, s + 0.15), 0.88) : hsl(h, Math.min(0.5, s * 0.6), 0.13)
}

// ── What a theme sets

/** The custom properties worked out from a theme's tones, for one appearance. */
function derivedVars(theme: ThemeDef, mode: Appearance, accentColor: string): Record<string, string> {
  const [accentR, accentG, accentB] = rgb(accentColor)
  const accent = {
    '--accent': accentColor,
    '--accent-fg': textOn(accentColor),
    '--accent-soft': `rgba(${accentR}, ${accentG}, ${accentB}, ${mode === 'dark' ? 0.22 : 0.16})`,
  }
  // Writan's own keeps app.css's colors, and its accent unless a variant changes it.
  if (theme.id === DEFAULT_THEME_ID) return accentColor === theme.accent ? {} : accent
  const [h, s] = toHsl(mode === 'dark' ? theme.dark : theme.light)
  if (mode === 'dark') {
    // Strong enough that the hue reads on every surface, not just the orbs in Settings.
    const tint = Math.min(1, s * 1.15 + 0.08) * 0.62
    const at = (l: number, extra = 0) => hsl(h, Math.min(1, tint * (1 + extra)), l / 100)
    return {
      ...accent,
      '--chrome': at(9.5),
      '--panel': at(12),
      '--canvas': at(7),
      '--raised': at(15.5),
      '--line': at(21, -0.1),
      '--line-soft': at(16, -0.1),
      '--field': at(17),
      '--field-hover': at(21),
      '--tab-active': at(16.5),
      '--fg': at(94, -0.55),
      '--fg-muted': at(70, -0.3),
      '--fg-faint': at(53, -0.2),
      '--md-paper': at(9.5),
      '--md-code-bg': at(14),
      '--txt-paper': at(9.5),
    }
  }
  const tint = Math.min(1, s * 1.1 + 0.1) * 0.78
  const at = (l: number, extra = 0) => hsl(h, Math.min(1, tint * (1 + extra)), l / 100)
  return {
    ...accent,
    '--chrome': at(89.5),
    '--panel': at(95),
    '--canvas': at(86.5),
    '--raised': at(99),
    '--line': at(81, -0.15),
    '--line-soft': at(87, -0.15),
    '--field': at(90.5),
    '--field-hover': at(86.5),
    '--tab-active': at(98.5),
    '--fg': at(13, -0.45),
    '--fg-muted': at(40, -0.35),
    '--fg-faint': at(55, -0.3),
    '--md-paper': at(99.5),
    '--md-code-bg': at(93),
    '--txt-paper': at(99.5),
  }
}

/** The custom properties a theme sets for one appearance, over app.css's defaults. */
export function themeVars(theme: ThemeDef, mode: Appearance): Record<string, string> {
  const own = theme.variants?.[mode] ?? {}
  const accent = own.accent && toHex6(own.accent) ? (toHex6(own.accent) as string) : theme.accent
  const vars = derivedVars(theme, mode, accent)
  for (const role of ROLES) {
    const value = own[role.key]
    if (isColor(value)) vars[role.css] = value.trim()
  }
  return vars
}

/** The theme's look in one appearance, for the orbs in Settings. */
export function orbBackground(theme: ThemeDef, mode: Appearance) {
  const own = theme.variants?.[mode] ?? {}
  const tone = (own.canvas && toHex6(own.canvas)) || (mode === 'dark' ? theme.dark : theme.light)
  const accent = (own.accent && toHex6(own.accent)) || theme.accent
  const [h, s, l] = toHsl(tone)
  const high = hsl(h, Math.min(1, s) * (mode === 'dark' ? 0.6 : 0.45), mode === 'dark' ? Math.min(0.34, l + 0.16) : 0.97)
  return `radial-gradient(circle at 30% 24%, ${high} 0%, ${tone} 52%, ${accent} 175%)`
}

// ── Theme files: what's exported, imported, and published in the themes folder

export const THEME_CSS_MAX_LENGTH = 100_000
const THEME_ID = /^(?!(?:system|light|dark)$)[a-z0-9][a-z0-9-]{0,47}$/

/** Whether `id` can name a published theme: lowercase letters, digits and hyphens, not a built-in. */
export const isThemeId = (id: unknown): id is string => typeof id === 'string' && THEME_ID.test(id) && !PRESET_THEMES.some((t) => t.id === id)

/** A theme as a file: what Export saves, and what the themes folder holds. */
export interface ThemeFile {
  version: 1
  name: string
  accent: string
  light: string
  dark: string
  variants?: Partial<Record<Appearance, RoleColors>>
  css?: string
}

export function themeFile(theme: ThemeDef): ThemeFile {
  const variants: Partial<Record<Appearance, RoleColors>> = {}
  for (const mode of ['light', 'dark'] as const) {
    const colors = theme.variants?.[mode]
    if (colors && Object.keys(colors).length) variants[mode] = { ...colors }
  }
  return {
    version: 1,
    name: theme.name,
    accent: theme.accent,
    light: theme.light,
    dark: theme.dark,
    ...(Object.keys(variants).length ? { variants } : {}),
    ...(theme.css?.trim() ? { css: theme.css } : {}),
  }
}

export class ThemeFileError extends Error {}

/** T3 Code's color roles that mean the same as one of Writan's. */
const T3_ROLES: Record<string, Role> = {
  canvas: 'canvas',
  chrome: 'chrome',
  toolbar: 'chrome',
  surface: 'panel',
  sidebar: 'panel',
  surfaceRaised: 'raised',
  surfaceOverlay: 'raised',
  text: 'fg',
  textMuted: 'fgMuted',
  mutedForeground: 'fgMuted',
  placeholder: 'fgFaint',
  border: 'line',
  input: 'field',
  accent: 'accent',
  accentForeground: 'accentFg',
  focus: 'focusStrong',
  error: 'danger',
  codeBackground: 'mdCodeBg',
  codeForeground: 'mdCode',
}

/** VS Code's colors, first match wins, for each of Writan's roles. */
const VSCODE_ROLES: [Role, string[]][] = [
  ['canvas', ['editor.background']],
  ['chrome', ['titleBar.activeBackground', 'editorGroupHeader.tabsBackground', 'tab.inactiveBackground']],
  ['panel', ['sideBar.background', 'panel.background', 'activityBar.background']],
  ['raised', ['editorWidget.background', 'menu.background', 'dropdown.background', 'quickInput.background']],
  ['tabActive', ['tab.activeBackground']],
  ['field', ['input.background', 'dropdown.background']],
  ['hover', ['list.hoverBackground', 'toolbar.hoverBackground']],
  ['fg', ['editor.foreground', 'foreground', 'sideBar.foreground']],
  ['fgMuted', ['descriptionForeground', 'tab.inactiveForeground', 'sideBarSectionHeader.foreground']],
  ['fgFaint', ['input.placeholderForeground', 'editorLineNumber.foreground']],
  ['line', ['panel.border', 'sideBar.border', 'editorGroup.border', 'contrastBorder']],
  ['lineSoft', ['tab.border', 'editorGroupHeader.tabsBorder']],
  ['scrollbar', ['scrollbarSlider.background']],
  ['accent', ['button.background', 'focusBorder', 'activityBarBadge.background']],
  ['accentFg', ['button.foreground']],
  ['accentSoft', ['list.activeSelectionBackground', 'editor.selectionBackground']],
  ['danger', ['errorForeground', 'editorError.foreground']],
  ['success', ['testing.iconPassed', 'gitDecoration.addedResourceForeground']],
  ['mdCodeBg', ['textCodeBlock.background']],
]

/** `#rrggbbaa` and `#rgba` as rgba(), which every engine reads; other colors as they are. */
function vscodeColor(value: unknown): string | null {
  if (typeof value !== 'string') return null
  const v = value.trim()
  const long = /^#([0-9a-f]{6})([0-9a-f]{2})$/i.exec(v)
  if (long) {
    const [r, g, b] = rgb(`#${long[1]}`)
    return `rgba(${r}, ${g}, ${b}, ${Math.round((parseInt(long[2], 16) / 255) * 100) / 100})`
  }
  if (/^#[0-9a-f]{4}$/i.test(v)) return vscodeColor(`#${[...v.slice(1)].map((c) => c + c).join('')}`)
  return isColor(v) ? v : null
}

function roleColors(value: unknown, what: string): RoleColors {
  if (value === undefined) return {}
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new ThemeFileError(`${what} should be an object of colors.`)
  const out: RoleColors = {}
  for (const [key, color] of Object.entries(value)) {
    if (!isRole(key)) throw new ThemeFileError(`${what}: Writan has no color “${key}”. The colors are ${ROLE_KEYS.join(', ')}.`)
    if (!isColor(color)) throw new ThemeFileError(`${what}.${key}: “${String(color)}” isn’t a color. Use hex like "#1f4e79", or rgb() or hsl().`)
    out[key] = (color as string).trim()
  }
  return out
}

function name(value: unknown, fallback: string) {
  const n = typeof value === 'string' ? value.trim().slice(0, 48) : ''
  return n || fallback
}

function hex6(value: unknown, what: string): string {
  const h = typeof value === 'string' ? toHex6(value) : null
  if (!h) throw new ThemeFileError(`${what} should be a hex color like "#4f8cff".`)
  return h
}

function cssOf(value: unknown): string | undefined {
  if (value === undefined || value === null || value === '') return undefined
  if (typeof value !== 'string') throw new ThemeFileError('css should be a string.')
  if (value.length > THEME_CSS_MAX_LENGTH) throw new ThemeFileError(`css is longer than ${THEME_CSS_MAX_LENGTH} characters.`)
  return value
}

/**
 * Reads a theme from JSON: Writan's own files, T3 Code's (an appearance, a canvas and an accent),
 * or a VS Code color theme. `fallbackName` names one without a name (a file's name).
 */
export function parseThemeFile(text: string | unknown, fallbackName = 'Imported theme'): Omit<ThemeDef, 'id'> {
  let data: unknown = text
  if (typeof text === 'string') {
    try {
      // VS Code themes often have comments and trailing commas.
      data = JSON.parse(text)
    } catch {
      try {
        data = JSON.parse(stripJsonComments(text))
      } catch {
        throw new ThemeFileError('That isn’t JSON.')
      }
    }
  }
  if (!data || typeof data !== 'object' || Array.isArray(data)) throw new ThemeFileError('A theme file is a JSON object.')
  const d = data as Record<string, unknown>

  // Writan's own.
  if (d.light !== undefined || d.dark !== undefined) {
    const accent = hex6(d.accent, 'accent')
    const light = d.light === undefined ? undefined : hex6(d.light, 'light')
    const dark = d.dark === undefined ? undefined : hex6(d.dark, 'dark')
    const v = d.variants
    if (v !== undefined && (!v || typeof v !== 'object' || Array.isArray(v))) throw new ThemeFileError('variants should be an object with light and dark.')
    const variants = { light: roleColors((v as Record<string, unknown> | undefined)?.light, 'variants.light'), dark: roleColors((v as Record<string, unknown> | undefined)?.dark, 'variants.dark') }
    return tidy({
      name: name(d.name, fallbackName),
      accent,
      light: light ?? otherTone(dark as string, 'dark'),
      dark: dark ?? otherTone(light as string, 'light'),
      variants,
      css: cssOf(d.css),
    })
  }

  // VS Code: colors keyed like "editor.background".
  const colors = d.colors
  if (colors && typeof colors === 'object' && !Array.isArray(colors) && Object.keys(colors).some((k) => k.includes('.'))) {
    const c = colors as Record<string, unknown>
    const type = typeof d.type === 'string' ? d.type.toLowerCase() : ''
    const mode: Appearance = type.includes('light') ? 'light' : type.includes('dark') || type.includes('black') ? 'dark' : guessMode(c)
    const mapped: RoleColors = {}
    for (const [role, keys] of VSCODE_ROLES) {
      for (const key of keys) {
        const color = vscodeColor(c[key])
        if (color) {
          mapped[role] = color
          break
        }
      }
    }
    const tone = (mapped.canvas && toHex6(mapped.canvas)) || (mode === 'dark' ? '#1e1e1e' : '#f3f3f3')
    const accent = (mapped.accent && toHex6(mapped.accent)) || '#4f8cff'
    return tidy({
      name: name(d.name, fallbackName),
      accent,
      light: mode === 'light' ? tone : otherTone(tone, 'dark'),
      dark: mode === 'dark' ? tone : otherTone(tone, 'light'),
      variants: { [mode]: mapped },
    })
  }

  // T3 Code: one appearance, a canvas and an accent, and its own color roles.
  if (d.appearance === 'light' || d.appearance === 'dark') {
    const mode = d.appearance
    const t3 = (value: unknown): RoleColors => {
      const out: RoleColors = {}
      if (!value || typeof value !== 'object') return out
      for (const [key, color] of Object.entries(value)) {
        const role = T3_ROLES[key]
        if (role && isColor(color) && !out[role]) out[role] = (color as string).trim()
      }
      return out
    }
    const own = t3(d.colors)
    // T3 Code allows a theme of CSS or a few colors alone; Writan's own colors fill in the rest.
    const canvas = (typeof d.canvas === 'string' && toHex6(d.canvas)) || (own.canvas && toHex6(own.canvas)) || (mode === 'dark' ? PRESET_THEMES[0].dark : PRESET_THEMES[0].light)
    const accent = (typeof d.accent === 'string' && toHex6(d.accent)) || (own.accent && toHex6(own.accent)) || PRESET_THEMES[0].accent
    const vs = (d.variants ?? {}) as Record<string, unknown>
    const other: Appearance = mode === 'dark' ? 'light' : 'dark'
    const otherColors = t3(vs[other])
    const otherCanvas = otherColors.canvas && toHex6(otherColors.canvas)
    return tidy({
      name: name(d.name, fallbackName),
      accent,
      light: mode === 'light' ? canvas : (otherCanvas ?? otherTone(canvas, 'dark')),
      dark: mode === 'dark' ? canvas : (otherCanvas ?? otherTone(canvas, 'light')),
      variants: { [mode]: { ...own, ...t3(vs[mode]) }, [other]: otherColors },
      css: cssOf(d.css),
    })
  }

  throw new ThemeFileError('This isn’t a Writan, T3 Code or VS Code theme. A Writan theme has accent, light and dark colors.')
}

/** Drops empty variants and css. */
function tidy(theme: Omit<ThemeDef, 'id'>): Omit<ThemeDef, 'id'> {
  const variants: Partial<Record<Appearance, RoleColors>> = {}
  for (const mode of ['light', 'dark'] as const) if (theme.variants?.[mode] && Object.keys(theme.variants[mode]).length) variants[mode] = theme.variants[mode]
  const out: Omit<ThemeDef, 'id'> = { name: theme.name, accent: theme.accent, light: theme.light, dark: theme.dark }
  if (Object.keys(variants).length) out.variants = variants
  if (theme.css?.trim()) out.css = theme.css
  return out
}

/** Whether a VS Code theme without a type is dark, from its editor background. */
function guessMode(colors: Record<string, unknown>): Appearance {
  const bg = typeof colors['editor.background'] === 'string' ? toHex6((colors['editor.background'] as string).slice(0, 7)) : null
  if (!bg) return 'dark'
  return toHsl(bg)[2] < 0.5 ? 'dark' : 'light'
}

/** JSON with // and /* comments and trailing commas, as VS Code writes it, made plain. */
export function stripJsonComments(text: string): string {
  let out = ''
  for (let i = 0; i < text.length; i++) {
    const c = text[i]
    if (c === '"') {
      const start = i
      for (i++; i < text.length && text[i] !== '"'; i++) if (text[i] === '\\') i++
      out += text.slice(start, i + 1)
    } else if (c === '/' && text[i + 1] === '/') {
      while (i < text.length && text[i] !== '\n') i++
      out += '\n'
    } else if (c === '/' && text[i + 1] === '*') {
      i = text.indexOf('*/', i + 2)
      if (i === -1) break
      i++
    } else out += c
  }
  return out.replace(/,(\s*[}\]])/g, '$1')
}

// ── For agents and theme authors

/**
 * Selectors theme CSS can rely on. Other class names can change between releases.
 * `tests/stress/ui/themes.mjs` fails when one of these leaves the app.
 */
export const THEME_CSS_HOOKS = [
  { selector: '.app', styles: 'the whole window' },
  { selector: '.titlebar', styles: 'title bar with the tabs' },
  { selector: '.tab', styles: 'a tab; .tab.active is the one you’re on' },
  { selector: '.home', styles: 'Home, the start page' },
  { selector: '.workbench', styles: 'an open document: sidebar, page and inspector' },
  { selector: '.pane.left', styles: 'left sidebar (slides, outline, pages)' },
  { selector: '.pane.right', styles: 'right sidebar, the inspector' },
  { selector: '.canvas', styles: 'the area behind a document’s pages' },
  { selector: '.explorer', styles: 'folder sidebar' },
  { selector: '.btn', styles: 'button; .btn.primary is the main one' },
  { selector: '.field', styles: 'text box or picker' },
  { selector: '.segmented', styles: 'segmented control' },
  { selector: '.switch', styles: 'switch' },
  { selector: '.menu', styles: 'menu' },
  { selector: '.dialog', styles: 'dialog' },
  { selector: '.settings', styles: 'Settings' },
  { selector: '.palette', styles: 'command palette' },
] as const

export const THEME_GUIDE = [
  'Themes restyle Writan: its window, sidebars, menus and dialogs. They don’t change the documents themselves.',
  '',
  'A theme has an `accent` (hex, for buttons, switches and focus rings) and two tones: `light` and `dark` (hex). Each appearance’s surfaces, borders and text are worked out from its tone, so text stays readable. Pick tones that carry the theme’s hue: Writan tints every surface with them. Give only one tone and the other is made from it.',
  '`variants.light` and `variants.dark` set single color roles for one appearance, over what the tones give. Prefer the tones; use roles to fix what they can’t. Values are hex, rgb() or hsl().',
  `Roles: ${ROLES.map((r) => `${r.key} (${r.hint.toLowerCase()})`).join('; ')}.`,
  '',
  'CSS: `css` is applied after the app’s own styles. Use `:root[data-theme="dark"]` or `:root[data-theme="light"]` for one appearance. Every color role is also a custom property (like --canvas, --panel, --fg) you can use.',
  'It can’t load files: no @import, no image-set(), and url() only with data: or #id. Rules that try are dropped.',
  'For a background image, copy a JPEG, PNG or WebP file of up to 16 MB to `<themesDir>/<themeId>.jpg` (or .png, .webp), beside the theme’s file. CSS shows it with `var(--theme-background-image)`, like `.home, .canvas { background: var(--theme-background-image) center / cover no-repeat; }`. Delete the file to remove it.',
  'The app’s own rules can be more specific. If a rule has no effect, add !important.',
  'Icons are `svg.lucide`.',
  '',
  'Selectors to rely on (other class names change between releases):',
  ...THEME_CSS_HOOKS.map((h) => `- ${h.selector}: ${h.styles}`),
  '',
  'You can’t see the result. Make small changes, then ask the person how it looks.',
  `To undo, set the theme "${DEFAULT_THEME_ID}", Writan’s own, or another built-in.`,
].join('\n')
