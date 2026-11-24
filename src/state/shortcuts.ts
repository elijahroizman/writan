import { create } from 'zustand'
import { isMac, isTauri, isWindows } from '../lib/platform'

/**
 * Every keyboard shortcut Writan knows about, and the user's changes to them.
 *
 * Keys are written the Mac way, modifiers in Apple's order (⌃⌥⇧⌘) then the key: "⇧⌘N".
 * On Windows ⌘ means Ctrl and ⌥ means Alt, and `shortcut()` in lib/platform shows them so.
 */

/** Where a shortcut works: everywhere, in one kind of editor, or in both documents and presentations. */
export type Scope = 'app' | 'docx' | 'pptx' | 'text'

export interface ShortcutDef {
  id: string
  label: string
  group: string
  scope: Scope
  /** Default keys; '' for none. */
  keys: string
  /** The editor engine already answers the default keys; Writan steps in only once they change. */
  builtIn?: boolean
  /** A built-in shortcut whose engine keys aren't its default: Writan answers `keys`, and the engine must not see these. */
  engineKeys?: string
  /** Shown for reference but can't be changed (the system or the engine owns it). */
  fixed?: boolean
  platform?: 'mac' | 'windows'
}

export const SHORTCUTS: ShortcutDef[] = [
  // ── Files and tabs (the menu bar's items)
  { id: 'new-tab', label: 'New tab', group: 'Files and tabs', scope: 'app', keys: '⌘T' },
  { id: 'new', label: 'New file…', group: 'Files and tabs', scope: 'app', keys: '⌘N' },
  // ⌘O's dialog opens a folder too.
  { id: 'open', label: 'Open…', group: 'Files and tabs', scope: 'app', keys: '⌘O' },
  { id: 'open-folder', label: 'Open folder…', group: 'Files and tabs', scope: 'app', keys: '' },
  { id: 'close-folder', label: 'Close folder', group: 'Files and tabs', scope: 'app', keys: '' },
  { id: 'save', label: 'Save', group: 'Files and tabs', scope: 'app', keys: '⌘S' },
  { id: 'save-as', label: 'Save as…', group: 'Files and tabs', scope: 'app', keys: '⇧⌘S' },
  { id: 'convert-file', label: 'Convert file…', group: 'Files and tabs', scope: 'app', keys: '' },
  { id: 'copy-markdown', label: 'Copy as Markdown (for LLMs)', group: 'Files and tabs', scope: 'app', keys: '' },
  { id: 'export-markdown', label: 'Export as Markdown (.md)', group: 'Files and tabs', scope: 'app', keys: '' },
  { id: 'export-pdf', label: 'Export as PDF', group: 'Files and tabs', scope: 'app', keys: '' },
  { id: 'bundle-pdf', label: 'Bundle open files into one PDF…', group: 'Files and tabs', scope: 'app', keys: '' },
  { id: 'print', label: 'Print…', group: 'Files and tabs', scope: 'app', keys: '⌘P' },
  { id: 'close-tab', label: 'Close tab', group: 'Files and tabs', scope: 'app', keys: '⌘W' },
  { id: 'reopen-tab', label: 'Reopen closed tab', group: 'Files and tabs', scope: 'app', keys: '⇧⌘T' },
  { id: 'version-history', label: 'Version history…', group: 'Files and tabs', scope: 'app', keys: '' },
  { id: 'share', label: 'Share…', group: 'Files and tabs', scope: 'app', keys: '' },
  { id: 'join-shared', label: 'Join a shared document…', group: 'Files and tabs', scope: 'app', keys: '' },
  { id: 'duplicate-tab', label: 'Duplicate tab', group: 'Files and tabs', scope: 'app', keys: '⇧⌘D' },
  { id: 'next-tab', label: 'Next tab', group: 'Files and tabs', scope: 'app', keys: isMac ? '⇧⌘]' : '⌘⇥' },
  { id: 'prev-tab', label: 'Previous tab', group: 'Files and tabs', scope: 'app', keys: isMac ? '⇧⌘[' : '⇧⌘⇥' },
  { id: 'tab-number', label: 'Go to tab 1–9', group: 'Files and tabs', scope: 'app', keys: '⌘1–9', fixed: true },
  { id: 'settings', label: 'Settings', group: 'Files and tabs', scope: 'app', keys: '⌘,' },
  { id: 'install-cli', label: 'Install the writan command in PATH', group: 'Files and tabs', scope: 'app', keys: '', platform: 'mac' },
  { id: 'quit', label: 'Quit Writan', group: 'Files and tabs', scope: 'app', keys: '⌘Q', platform: 'mac' },

  // ── View
  { id: 'command-palette', label: 'Command palette', group: 'View', scope: 'app', keys: '⌘K' },
  { id: 'find', label: 'Find', group: 'View', scope: 'app', keys: '⌘F' },
  // Word's keys; ⌘H on a Mac hides the app.
  { id: 'replace', label: 'Find and replace', group: 'View', scope: 'app', keys: isMac ? '⌃H' : '⌘H' },
  { id: 'toggle-explorer', label: 'Show or hide the folder', group: 'View', scope: 'app', keys: '⇧⌘E' },
  { id: 'split-view', label: 'Split view', group: 'View', scope: 'app', keys: '⇧⌘\\' },
  { id: 'toggle-left', label: 'Show or hide sidebar', group: 'View', scope: 'app', keys: '⌘\\' },
  { id: 'toggle-right', label: 'Show or hide inspector', group: 'View', scope: 'app', keys: '⌥⌘\\' },
  { id: 'zoom-in', label: 'Zoom in', group: 'View', scope: 'app', keys: '⌘=' },
  { id: 'zoom-out', label: 'Zoom out', group: 'View', scope: 'app', keys: '⌘-' },
  { id: 'zoom-reset', label: 'Fit to window', group: 'View', scope: 'app', keys: '⌘0' },
  { id: 'focus-mode', label: 'Focus mode', group: 'View', scope: 'app', keys: '⌥⌘F' },
  { id: 'typewriter', label: 'Typewriter scrolling', group: 'View', scope: 'app', keys: '⌥⌘T' },
  { id: 'focus-dim', label: 'Dim other paragraphs in focus mode', group: 'View', scope: 'app', keys: '' },
  { id: 'toggle-theme', label: 'Switch between light and dark', group: 'View', scope: 'app', keys: '⇧⌘L' },
  // ⌥⌘A is Add comment on a Mac.
  { id: 'change-theme', label: 'Change theme…', group: 'View', scope: 'app', keys: isMac ? '⌃⌘T' : '⌥⌘A' },
  { id: 'toggle-fullscreen', label: 'Full screen', group: 'View', scope: 'app', keys: 'F11', platform: 'windows' },
  { id: 'fullscreen-mac', label: 'Full screen', group: 'View', scope: 'app', keys: '⌃⌘F', fixed: true, platform: 'mac' },
  { id: 'view-read', label: 'Read', group: 'View', scope: 'app', keys: '⌥⌘1' },
  { id: 'view-split', label: 'Edit with preview', group: 'View', scope: 'app', keys: '⌥⌘2' },
  { id: 'view-write', label: 'Edit source', group: 'View', scope: 'app', keys: '⌥⌘3' },
  { id: 'view-cycle', label: 'Next view', group: 'View', scope: 'app', keys: '⌘E' },

  // ── Editing (everywhere)
  { id: 'undo', label: 'Undo', group: 'Editing', scope: 'app', keys: '⌘Z', fixed: true },
  { id: 'redo', label: 'Redo', group: 'Editing', scope: 'app', keys: '⇧⌘Z', fixed: true },
  { id: 'cut', label: 'Cut', group: 'Editing', scope: 'app', keys: '⌘X', fixed: true },
  { id: 'copy', label: 'Copy', group: 'Editing', scope: 'app', keys: '⌘C', fixed: true },
  { id: 'paste', label: 'Paste', group: 'Editing', scope: 'app', keys: '⌘V', fixed: true },
  { id: 'select-all', label: 'Select all', group: 'Editing', scope: 'app', keys: '⌘A', fixed: true },

  // ── Text, in documents and presentations
  { id: 'text.bold', label: 'Bold', group: 'Text (documents and presentations)', scope: 'text', keys: '⌘B', builtIn: true },
  { id: 'text.italic', label: 'Italic', group: 'Text (documents and presentations)', scope: 'text', keys: '⌘I', builtIn: true },
  { id: 'text.underline', label: 'Underline', group: 'Text (documents and presentations)', scope: 'text', keys: '⌘U', builtIn: true },

  // ── Documents (.docx)
  // The engine's own key for a link is ⌘K, which opens the command palette instead.
  { id: 'docx.link', label: 'Link', group: 'Document', scope: 'docx', keys: '⇧⌘K', builtIn: true, engineKeys: '⌘K' },
  // Word's own keys; ⌥⌘M on a Mac is the Window menu's Minimize All.
  { id: 'docx.comment', label: 'Add comment', group: 'Document', scope: 'docx', keys: isMac ? '⌥⌘A' : '⌥⌘M' },
  { id: 'docx.header', label: 'Edit header', group: 'Document', scope: 'docx', keys: '' },
  { id: 'docx.footer', label: 'Edit footer', group: 'Document', scope: 'docx', keys: '' },
  { id: 'docx.math', label: 'Insert LaTeX math', group: 'Document', scope: 'docx', keys: '⌥=' },

  // ── Presentations (.pptx)
  { id: 'pptx.new-slide', label: 'New slide', group: 'Presentation', scope: 'pptx', keys: '⌘M', builtIn: true },
  { id: 'pptx.duplicate', label: 'Duplicate', group: 'Presentation', scope: 'pptx', keys: '⌘D', builtIn: true },
  { id: 'pptx.group', label: 'Group', group: 'Presentation', scope: 'pptx', keys: '⌘G', builtIn: true },
  { id: 'pptx.ungroup', label: 'Ungroup', group: 'Presentation', scope: 'pptx', keys: '⇧⌘G', builtIn: true },
  { id: 'pptx.front', label: 'Bring to front', group: 'Presentation', scope: 'pptx', keys: '⇧⌘F' },
  { id: 'pptx.forward', label: 'Bring forward', group: 'Presentation', scope: 'pptx', keys: '⌥⇧⌘F' },
  { id: 'pptx.backward', label: 'Send backward', group: 'Presentation', scope: 'pptx', keys: '⌥⇧⌘B' },
  { id: 'pptx.back', label: 'Send to back', group: 'Presentation', scope: 'pptx', keys: '⇧⌘B' },
  { id: 'pptx.notes', label: 'Show or hide speaker notes', group: 'Presentation', scope: 'pptx', keys: '' },
  { id: 'pptx.present', label: 'Play from this slide', group: 'Presentation', scope: 'pptx', keys: '⌘↩' },
  { id: 'pptx.present-start', label: 'Play from start', group: 'Presentation', scope: 'pptx', keys: '⇧⌘↩' },
  { id: 'pptx.math', label: 'Insert LaTeX math', group: 'Presentation', scope: 'pptx', keys: '⌥=' },
].filter((s) => !s.platform || (s.platform === 'mac') === isMac) as ShortcutDef[]

const BY_ID = new Map(SHORTCUTS.map((s) => [s.id, s]))

/** Whether two scopes can see the same key press. */
export function scopesOverlap(a: Scope, b: Scope) {
  return a === b || a === 'app' || b === 'app' || a === 'text' || b === 'text'
}

const KEY = 'workspace.shortcuts'

function load(): Record<string, string> {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(KEY) || '{}')
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return {}
    const out: Record<string, string> = {}
    for (const [id, keys] of Object.entries(parsed)) {
      const def = BY_ID.get(id)
      if (def && !def.fixed && typeof keys === 'string' && (keys === '' || isValidKeys(keys))) out[id] = keys
    }
    return out
  } catch {
    return {}
  }
}

interface ShortcutState {
  /** Changed shortcuts by id; '' means the user removed it. */
  custom: Record<string, string>
  /** Set one shortcut's keys, taking them away from anything else they would clash with. Returns what lost them. */
  assign(id: string, keys: string): ShortcutDef[]
  reset(id: string): void
  resetAll(): void
}

export const useShortcuts = create<ShortcutState>((_set, get) => ({
  custom: load(),
  assign: (id, keys) => {
    const def = BY_ID.get(id)
    if (!def || def.fixed) return []
    const custom = { ...get().custom }
    const losers = keys ? clashes(id, keys) : []
    for (const other of losers) custom[other.id] = ''
    custom[id] = keys
    for (const s of [def, ...losers]) if (custom[s.id] === s.keys) delete custom[s.id]
    save(custom)
    return losers
  },
  reset: (id) => {
    const custom = { ...get().custom }
    delete custom[id]
    // Taking the default back may take it from whatever has it now.
    const def = BY_ID.get(id)
    if (def?.keys) for (const other of clashes(id, def.keys)) custom[other.id] = ''
    save(custom)
  },
  resetAll: () => save({}),
}))

function save(custom: Record<string, string>) {
  useShortcuts.setState({ custom })
  localStorage.setItem(KEY, JSON.stringify(custom))
  void syncMenu()
}

/** The keys a shortcut has now, Mac style ('' when it has none). */
export function keysFor(id: string, custom = useShortcuts.getState().custom): string {
  return custom[id] ?? BY_ID.get(id)?.keys ?? ''
}

/** "Label (⌘K)", or just the label when the shortcut has no keys. */
export function withKeys(label: string, id: string): string {
  const keys = keysFor(id)
  return keys ? `${label} (${keys})` : label
}

/** Other shortcuts, seen from the same place, that already use these keys. */
export function clashes(id: string, keys: string): ShortcutDef[] {
  const def = BY_ID.get(id)
  if (!def) return []
  return SHORTCUTS.filter((s) => s.id !== id && scopesOverlap(s.scope, def.scope) && keysFor(s.id) === keys)
}

/** The shortcut the keys run where the user is (app shortcuts first). */
export function shortcutFor(keys: string, scope: Scope | null): ShortcutDef | undefined {
  return SHORTCUTS.find((s) => !s.fixed && keysFor(s.id) === keys && (s.scope === 'app' || (!!scope && scopesOverlap(s.scope, scope))))
}

/** The keys the editor engine itself answers for a built-in shortcut. */
export const engineKeysOf = (def: ShortcutDef) => (def.builtIn ? (def.engineKeys ?? def.keys) : undefined)

/** A built-in engine shortcut whose engine keys these are, now moved elsewhere: the engine must not see them. */
export function movedBuiltIn(keys: string, scope: Scope | null): ShortcutDef | undefined {
  return SHORTCUTS.find((s) => engineKeysOf(s) === keys && keysFor(s.id) !== keys && !!scope && scopesOverlap(s.scope, scope))
}

// ── Reading keys from the keyboard

const CODE_KEYS: Record<string, string> = {
  Comma: ',',
  Period: '.',
  Slash: '/',
  Semicolon: ';',
  Quote: "'",
  Backquote: '`',
  Equal: '=',
  NumpadAdd: '=',
  Minus: '-',
  NumpadSubtract: '-',
  Backslash: '\\',
  BracketRight: ']',
  BracketLeft: '[',
  Enter: '↩',
  NumpadEnter: '↩',
  Backspace: '⌫',
  Delete: '⌦',
  Tab: '⇥',
  Space: 'Space',
  Escape: 'Esc',
  ArrowLeft: '←',
  ArrowRight: '→',
  ArrowUp: '↑',
  ArrowDown: '↓',
  Home: 'Home',
  End: 'End',
  PageUp: 'PageUp',
  PageDown: 'PageDown',
}

/**
 * The key part of a key press. Letters follow the keyboard layout; when the key types something
 * else (⌥ on a Mac, a Cyrillic layout) fall back to the physical key, except for Ctrl+Alt on
 * Windows: that is AltGr, which types characters like ń.
 */
function keyName(e: KeyboardEvent): string | null {
  if (e.code.startsWith('Digit')) return e.code.slice(5)
  if (e.code.startsWith('Numpad') && /^\d$/.test(e.code.slice(6))) return e.code.slice(6)
  if (CODE_KEYS[e.code]) return CODE_KEYS[e.code]
  if (/^F\d{1,2}$/.test(e.key)) return e.key
  if (/^[a-z]$/i.test(e.key)) return e.key.toUpperCase()
  if (e.code.startsWith('Key') && (isMac || !e.altKey)) return e.code.slice(3)
  return null
}

/** A key press written as shortcut keys ("⇧⌘N"), or null for a lone modifier. */
export function keysFromEvent(e: KeyboardEvent): string | null {
  const key = keyName(e)
  if (!key) return null
  const command = isMac ? e.metaKey : e.ctrlKey
  return `${isMac && e.ctrlKey ? '⌃' : ''}${e.altKey ? '⌥' : ''}${e.shiftKey ? '⇧' : ''}${command ? '⌘' : ''}${key}`
}

/** A shortcut needs ⌘ (Ctrl on Windows) or ⌃, or must be a function key; otherwise it would get in the way of typing. */
export function isValidKeys(keys: string): boolean {
  return /^⌃?⌥?⇧?⌘?[^⌃⌥⇧⌘]+$/.test(keys) && (/[⌘⌃]/.test(keys) || /F\d{1,2}$/.test(keys))
}

// ── The Mac menu bar owns app shortcuts; tell it about changes.

const ACCELERATOR_KEYS: Record<string, string> = {
  '↩': 'Enter',
  '⌫': 'Backspace',
  '⌦': 'Delete',
  '⇥': 'Tab',
  Space: 'Space',
  Esc: 'Escape',
  '←': 'Left',
  '→': 'Right',
  '↑': 'Up',
  '↓': 'Down',
}

/** "⇧⌘N" as a Tauri menu accelerator, "CmdOrCtrl+Shift+N". */
export function toAccelerator(keys: string): string {
  const m = /^(⌃?)(⌥?)(⇧?)(⌘?)(.+)$/.exec(keys)
  if (!m) return ''
  const parts = [m[4] && 'CmdOrCtrl', m[1] && 'Ctrl', m[2] && 'Alt', m[3] && 'Shift'].filter(Boolean)
  return [...parts, ACCELERATOR_KEYS[m[5]] ?? m[5]].join('+')
}

/**
 * Rebuild the native menu with the current keys (labels on Windows, real shortcuts on a Mac).
 * `paused` takes every shortcut off the menu while Settings records new keys, so pressing ⌘W
 * to try it out doesn't close the tab.
 */
export async function syncMenu({ paused = false } = {}) {
  if (!isTauri || !(isMac || isWindows)) return
  const custom = useShortcuts.getState().custom
  const accelerators: Record<string, string> = {}
  for (const s of SHORTCUTS) {
    if (s.scope !== 'app' || s.fixed) continue
    if (paused) accelerators[s.id] = ''
    else if (s.id in custom) accelerators[s.id] = custom[s.id] ? toAccelerator(custom[s.id]) : ''
  }
  const { invoke } = await import('@tauri-apps/api/core')
  await invoke('set_menu_shortcuts', { shortcuts: accelerators }).catch((err) => console.warn('Menu shortcuts', err))
}
