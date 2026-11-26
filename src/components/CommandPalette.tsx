import type { LucideIcon } from 'lucide-react'
import {
  ArrowDown,
  ArrowRight,
  ArrowRightLeft,
  ArrowUp,
  BookOpen,
  Bold,
  BringToFront,
  ChevronLeft,
  ChevronRight,
  ClipboardCopy,
  Clock,
  Code,
  Columns2,
  Copy,
  CopyPlus,
  Eye,
  FileOutput,
  FilePlus,
  FileStack,
  FileText,
  Focus,
  FolderOpen,
  FolderTree,
  FolderX,
  Fullscreen,
  Group,
  History,
  LogIn,
  Italic,
  LayoutGrid,
  Link,
  LogOut,
  MessageSquarePlus,
  Palette,
  Monitor,
  Moon,
  PanelBottom,
  PanelLeft,
  PanelRight,
  PanelTop,
  Play,
  Presentation,
  Printer,
  Replace,
  Rocket,
  RotateCcw,
  RotateCw,
  Save,
  ScanText,
  ScrollText,
  Search,
  SendToBack,
  Settings,
  SlidersHorizontal,
  SquareChevronRight,
  SquarePlus,
  StickyNote,
  Sun,
  SunMoon,
  Terminal,
  Trash2,
  Underline,
  Ungroup,
  Users,
  X,
  ZoomIn,
  ZoomOut,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { commandAvailable, runCommand } from '../App'
import { usePaletteSourceRev } from '../lib/folderFiles'
import { KINDS } from '../lib/kinds'
import { dirname, tildify } from '../lib/platform'
import { usePalette, sourceItems, type PaletteItem, type Picker } from '../state/modals'
import { enabledKinds, usePlugins } from '../state/plugins'
import type { SettingsPage } from '../state/settings'
import { useSettings } from '../state/settings'
import type { Scope } from '../state/shortcuts'
import { SHORTCUTS, keysFor } from '../state/shortcuts'
import { getEditor, HOME_TAB_ID, useWorkspace } from '../state/workspace'
import { orbBackground } from '../lib/themes'
import { allThemes, useAppearance } from '../state/appearance'
import { recentItems } from './paletteItems'
import { Kbd, KindIcon, Modal } from './ui'

export { recentItems, sourceItems, usePalette, type PaletteItem, type Picker }

/**
 * ⌘K: one box to find anything. Type to search commands, open tabs and recent files; start
 * with “>” for commands only. Arrows and return pick, esc closes.
 */
/** Change theme…: every theme, the one in use first. Picking one uses it for light and dark. */
export function openThemePicker() {
  usePalette.getState().pick({
    placeholder: 'Pick a theme',
    items: () => {
      const { light, dark } = useAppearance.getState()
      const mode = useWorkspace.getState().theme
      const current = mode === 'dark' ? dark : light
      const themes = allThemes()
      return [...themes.filter((t) => t.id === current), ...themes.filter((t) => t.id !== current)].map((t) => ({
        id: `theme-pick:${t.id}`,
        label: t.name,
        detail: t.id === current ? 'In use' : t.published ? 'Themes folder' : t.custom ? 'Yours' : undefined,
        group: 'Commands' as const,
        icon: <span className="palette-theme-dot" style={{ background: orbBackground(t, mode) }} />,
        keywords: 'theme',
        run: () => useAppearance.getState().use(t.id),
      }))
    },
  })
}

/** Each command's icon. Commands without one get a plain prompt arrow, which reads the same on every system. */
const COMMAND_ICONS: Record<string, LucideIcon> = {
  'new-tab': SquarePlus,
  new: FilePlus,
  open: FolderOpen,
  'open-folder': FolderTree,
  'close-folder': FolderX,
  save: Save,
  'save-as': Save,
  revert: RotateCw,
  'convert-file': ArrowRightLeft,
  'ocr-document': ScanText,
  'copy-markdown': ClipboardCopy,
  'export-markdown': FileText,
  'export-pdf': FileOutput,
  'bundle-pdf': FileStack,
  print: Printer,
  'close-tab': X,
  'reopen-tab': RotateCcw,
  'version-history': History,
  share: Users,
  'join-shared': LogIn,
  'duplicate-tab': Copy,
  'next-tab': ChevronRight,
  'prev-tab': ChevronLeft,
  settings: Settings,
  'install-cli': Terminal,
  quit: LogOut,
  find: Search,
  replace: Replace,
  'toggle-explorer': FolderTree,
  'split-view': Columns2,
  'toggle-left': PanelLeft,
  'toggle-right': PanelRight,
  'zoom-in': ZoomIn,
  'zoom-out': ZoomOut,
  'zoom-reset': Fullscreen,
  'focus-mode': Focus,
  typewriter: ScrollText,
  'focus-dim': SunMoon,
  'toggle-theme': SunMoon,
  'change-theme': Palette,
  'toggle-fullscreen': Fullscreen,
  'view-read': Eye,
  'view-split': Columns2,
  'view-write': Code,
  'view-cycle': BookOpen,
  'text.bold': Bold,
  'text.italic': Italic,
  'text.underline': Underline,
  'docx.link': Link,
  'docx.comment': MessageSquarePlus,
  'docx.header': PanelTop,
  'docx.footer': PanelBottom,
  'pptx.new-slide': SquarePlus,
  'pptx.duplicate': CopyPlus,
  'pptx.group': Group,
  'pptx.ungroup': Ungroup,
  'pptx.front': BringToFront,
  'pptx.forward': ArrowUp,
  'pptx.backward': ArrowDown,
  'pptx.back': SendToBack,
  'pptx.notes': StickyNote,
  'pptx.present': Play,
  'pptx.present-start': Presentation,
}

const commandIcon = (id: string) => {
  const Icon = COMMAND_ICONS[id]
  return Icon ? <Icon size={14} /> : undefined
}

const THEME_ICONS = { light: <Sun size={14} />, dark: <Moon size={14} />, system: <Monitor size={14} /> }

/** Commands that make no sense in a list you pick from (they're keys, or they're this, or Open… does it too). */
const HIDDEN = new Set(['command-palette', 'tab-number', 'fullscreen-mac', 'open-folder'])

/** Other words people search for a command by. */
const COMMAND_KEYWORDS: Record<string, string> = {
  open: 'file folder',
  'zoom-reset': 'zoom actual size',
  'toggle-theme': 'dark light theme appearance',
  'change-theme': 'theme color colour look style appearance',
  find: 'search',
  replace: 'search',
  'split-view': 'side by side',
  'version-history': 'versions restore',
  revert: 'revert refresh saved discard changes',
  share: 'collaborate invite network lan co-edit',
  'join-shared': 'collaborate invite network lan co-edit link',
  'focus-mode': 'distraction free',
  'view-read': 'preview',
  'view-split': 'preview source',
  'view-write': 'source',
  'convert-file': 'export transform docx markdown md pdf pptx xlsx csv llm',
  'ocr-document': 'recognize scan searchable text pdf image offline',
  'copy-markdown': 'llm ai prompt markdown md docx clipboard export',
  'export-markdown': 'llm ai markdown md docx convert',
  'export-pdf': 'convert print render document',
  'bundle-pdf': 'merge combine join tabs pdf all open files',
}

/** Commands that only work on a document. From Home they show greyed out, so searching for one explains itself. */
const NEEDS_DOCUMENT = new Set(['share', 'save', 'save-as', 'revert', 'convert-file', 'copy-markdown', 'export-markdown', 'export-pdf', 'bundle-pdf', 'print', 'close-tab', 'duplicate-tab', 'version-history', 'find', 'replace', 'split-view', 'toggle-left', 'toggle-right', 'zoom-in', 'zoom-out', 'zoom-reset', 'focus-mode', 'typewriter', 'focus-dim'])

const SETTINGS_PAGES: { page: SettingsPage; label: string }[] = [
  { page: 'general', label: 'General' },
  { page: 'autosave', label: 'Autosave' },
  { page: 'appearance', label: 'Appearance' },
  { page: 'shortcuts', label: 'Keyboard Shortcuts' },
]

const sentence = (s: string) => s.charAt(0).toUpperCase() + s.slice(1)

function commandItems(): PaletteItem[] {
  const ws = useWorkspace.getState()
  const tab = ws.tabs.find((t) => t.id === ws.activeId)
  const scope: Scope | null = tab?.kind === 'docx' || tab?.kind === 'pptx' ? tab.kind : null
  const editor = getEditor(ws.activeId)
  const items: PaletteItem[] = []

  for (const def of SHORTCUTS) {
    if (def.fixed || HIDDEN.has(def.id)) continue
    const keys = keysFor(def.id)
    const keywords = COMMAND_KEYWORDS[def.id]
    if (def.scope === 'app') {
      if (commandAvailable(def.id)) {
        items.push({ id: `cmd:${def.id}`, label: sentence(def.label), group: 'Commands', icon: commandIcon(def.id), keys, keywords, run: () => runCommand(def.id) })
      } else if (!tab && NEEDS_DOCUMENT.has(def.id)) {
        items.push({ id: `cmd:${def.id}`, label: sentence(def.label), detail: 'Open a document first', group: 'Commands', icon: commandIcon(def.id), keys, keywords, disabled: true, run: () => {} })
      }
      continue
    }
    // Editor commands, for the kind of document in front of you.
    const fits = def.scope === 'text' ? scope !== null : def.scope === scope
    const action = editor?.actions?.[def.id]
    if (!fits || !action) continue
    items.push({ id: `cmd:${def.id}`, label: sentence(def.label), group: 'Commands', icon: commandIcon(def.id), keys, keywords, run: () => void action() })
  }

  for (const kind of enabledKinds(usePlugins.getState().enabled)) {
    if (!kind.create) continue
    items.push({
      id: `new:${kind.id}`,
      label: sentence(kind.create.title),
      detail: kind.create.hint,
      group: 'Commands',
      icon: <KindIcon kind={kind.id} />,
      keywords: `new create ${kind.label}`,
      run: () => ws.newDocument(kind.id),
    })
  }

  for (const mode of ['light', 'dark', 'system'] as const) {
    items.push({
      id: `theme:${mode}`,
      label: `Appearance: ${mode === 'system' ? 'Match the system' : sentence(mode)}`,
      group: 'Commands',
      icon: THEME_ICONS[mode],
      keywords: 'theme color mode',
      run: () => ws.setThemeMode(mode),
    })
  }
  for (const { page, label } of SETTINGS_PAGES) {
    items.push({ id: `settings:${page}`, label: `Settings: ${label}`, group: 'Commands', icon: <SlidersHorizontal size={14} />, keywords: 'preferences options', run: () => useSettings.getState().setOpen(true, page) })
  }
  items.push({ id: 'check-updates', label: 'Check for updates…', group: 'Commands', icon: <Rocket size={14} />, keywords: 'version upgrade', run: () => runCommand('check-updates') })
  if (ws.recents.length) items.push({ id: 'clear-recents', label: 'Clear recent files', group: 'Commands', icon: <Trash2 size={14} />, keywords: 'history', run: () => ws.clearRecents() })
  return items
}

function tabItems(): PaletteItem[] {
  const ws = useWorkspace.getState()
  const items: PaletteItem[] = [{ id: `tab:${HOME_TAB_ID}`, label: 'Home', group: 'Tabs', icon: <LayoutGrid size={14} />, run: () => ws.activate(HOME_TAB_ID) }]
  for (const t of ws.tabs) {
    items.push({
      id: `tab:${t.id}`,
      label: t.title,
      detail: t.path && !t.path.startsWith('browser:') ? tildify(dirname(t.path)) : t.dirty ? 'Not saved' : undefined,
      group: 'Tabs',
      icon: <KindIcon kind={t.kind} />,
      run: () => ws.activate(t.id),
    })
  }
  return items.filter((i) => i.id !== `tab:${ws.activeId}`)
}

// recentItems, sourceItems and addPaletteSource live in paletteItems.tsx / state/modals.ts.

const BOUNDARY = /[\s\-_./:(]/

/**
 * How well `query` matches `text`: its letters in order, scoring runs, word starts and an early
 * start. Null when they aren't all there. `at` lists the matched positions, for bold.
 * Scattered letters must start at a word and only jump to the start of another word, so
 * “open” finds Open… but not the o, p, e, n spread through “New presentation (PowerPoint)”.
 */
export function fuzzy(query: string, text: string): { score: number; at: number[] } | null {
  const q = query.toLowerCase()
  const t = text.toLowerCase()
  if (!q) return { score: 0, at: [] }
  // A plain substring beats any scattered match.
  const sub = t.indexOf(q)
  if (sub !== -1) {
    const wordStart = sub === 0 || BOUNDARY.test(t[sub - 1])
    return { score: 1000 - sub + (wordStart ? 200 : 0) - t.length * 0.1, at: Array.from({ length: q.length }, (_, i) => sub + i) }
  }
  const chars = q.replace(/ /g, '')
  // O(N) subsequence fast-reject before recursive word-boundary matching.
  for (let qi = 0, ti = -1; qi < chars.length; qi++) {
    ti = t.indexOf(chars[qi], ti + 1)
    if (ti === -1) return null
  }
  const wordStart = (i: number) => i === 0 || BOUNDARY.test(t[i - 1])
  const failed = new Set<number>()
  // The earliest placement that follows the rules, backing up when a greedy pick leads nowhere.
  const place = (qi: number, prev: number): number[] | null => {
    if (qi === chars.length) return []
    const key = qi * (t.length + 1) + prev + 1
    if (failed.has(key)) return null
    for (let i = t.indexOf(chars[qi], prev + 1); i !== -1; i = t.indexOf(chars[qi], i + 1)) {
      const sameWord = prev !== -1 && !BOUNDARY.test(t.slice(prev + 1, i))
      if (!wordStart(i) && !sameWord) continue
      const rest = place(qi + 1, i)
      if (rest) return [i, ...rest]
    }
    failed.add(key)
    return null
  }
  const at = place(0, -1)
  if (!at) return null
  let score = 0
  at.forEach((found, i) => {
    const prev = i ? at[i - 1] : -1
    if (prev !== -1 && found === prev + 1) score += 15
    if (wordStart(found)) score += 10
    score -= Math.min(found - prev - 1, 10)
  })
  return { score: score - t.length * 0.1, at }
}

function Highlight({ text, at }: { text: string; at: number[] }) {
  if (!at.length) return <>{text}</>
  const marks = new Set(at)
  const parts: ReactNode[] = []
  let run = ''
  let bold = false
  const flush = (i: number) => {
    if (run) parts.push(bold ? <b key={i}>{run}</b> : run)
    run = ''
  }
  for (let i = 0; i < text.length; i++) {
    const m = marks.has(i)
    if (m !== bold) {
      flush(i)
      bold = m
    }
    run += text[i]
  }
  flush(text.length)
  return <>{parts}</>
}

const GROUP_ICON = { Tabs: <ArrowRight size={14} />, Recent: <Clock size={14} />, Folder: <FileText size={14} />, Open: <FolderOpen size={14} />, Commands: <SquareChevronRight size={14} /> }

export function CommandPalette() {
  const initial = usePalette((s) => s.query)
  const picker = usePalette((s) => s.picker)
  const [query, setQuery] = useState(initial)
  const [index, setIndex] = useState(0)
  const input = useRef<HTMLInputElement>(null)
  const list = useRef<HTMLDivElement>(null)
  const close = () => usePalette.getState().setOpen(false)

  const sourceRev = usePaletteSourceRev((s) => s.rev)
  // Gathered once per opening (and refreshed if lazy folder walk finishes while open).
  const all = useMemo(() => (picker ? picker.items() : [...tabItems(), ...recentItems(), ...sourceItems(), ...commandItems()]), [picker, sourceRev])

  const onlyCommands = !picker && query.startsWith('>')
  const q = (onlyCommands ? query.slice(1) : query).trim()
  const shown = useMemo(() => {
    const pool = onlyCommands ? all.filter((i) => i.group === 'Commands') : all
    if (!q) {
      // Nothing typed: tabs, then a few recent files, then every command (a folder's files wait for a search, except when picking).
      let recent = 0
      let folder = 0
      return pool
        .filter((i) => !i.disabled && (i.group === 'Folder' ? !!picker && folder++ < 8 : i.group !== 'Recent' || recent++ < 5))
        .map((item) => ({ item, at: [] as number[] }))
    }
    const scored: { item: PaletteItem; at: number[]; score: number }[] = []
    for (const item of pool) {
      // A greyed-out command's detail only says why; it isn't something to search.
      const detail = item.disabled ? '' : (item.detail ?? '')
      // And it goes below everything that can run.
      const penalty = item.disabled ? 400 : 0
      const m = fuzzy(q, item.label)
      const extra = !m && item.keywords ? fuzzy(q, `${item.keywords} ${detail}`) : null
      if (m) scored.push({ item, at: m.at, score: m.score + (item.group === 'Tabs' ? 30 : 0) - penalty })
      else if (extra) scored.push({ item, at: [], score: extra.score - 300 - penalty })
      else if (detail) {
        const d = fuzzy(q, detail)
        if (d && d.score > 0) scored.push({ item, at: [], score: d.score - 500 })
      }
    }
    return scored.sort((a, b) => b.score - a.score).slice(0, 80)
  }, [all, q, onlyCommands, picker])

  useEffect(() => {
    input.current?.focus()
    input.current?.select()
  }, [])
  useEffect(() => setIndex(0), [query])
  useEffect(() => {
    list.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [index, shown])

  const pick = (item: PaletteItem | undefined) => {
    if (!item || item.disabled) return
    close()
    // After the palette is gone, so the command's own focus (a find box, a dialog) sticks.
    requestAnimationFrame(() => void item.run())
  }

  const keyStateRef = useRef({ shown, index, pick })
  keyStateRef.current = { shown, index, pick }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const { shown: s, index: idx, pick: doPick } = keyStateRef.current
      if (e.key === 'Escape') close()
      else if (e.key === 'ArrowDown' || (e.ctrlKey && e.key === 'n')) setIndex((i) => Math.min(s.length - 1, i + 1))
      else if (e.key === 'ArrowUp' || (e.ctrlKey && e.key === 'p')) setIndex((i) => Math.max(0, i - 1))
      else if (e.key === 'PageDown') setIndex((i) => Math.min(s.length - 1, i + 8))
      else if (e.key === 'PageUp') setIndex((i) => Math.max(0, i - 8))
      else if (e.key === 'Enter') doPick(s[idx]?.item)
      else return
      e.preventDefault()
      e.stopPropagation()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [])

  return (
    <Modal placement="top" dim="light" layer="picker" onClose={close}>
      <div className="palette" role="dialog" aria-modal="true" aria-label={picker ? picker.placeholder : 'Command palette'}>
        <input
          ref={input}
          className="palette-input"
          value={query}
          placeholder={picker?.placeholder ?? 'Search tabs, recent files and commands (> for commands only)'}
          spellCheck={false}
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-list"
          aria-activedescendant={shown[index] ? `palette-${index}` : undefined}
          onChange={(e) => setQuery(e.currentTarget.value)}
        />
        <div className="palette-list" id="palette-list" role="listbox" ref={list}>
          {shown.length === 0 && <div className="palette-empty">Nothing matches “{q}”.</div>}
          {shown.map(({ item, at }, i) => {
            const first = !q && (i === 0 || shown[i - 1].item.group !== item.group)
            return (
              <div key={item.id} role="presentation">
                {first && <div className="palette-group">{item.group}</div>}
                <button
                  id={`palette-${i}`}
                  type="button"
                  role="option"
                  aria-selected={i === index}
                  aria-disabled={item.disabled || undefined}
                  className={`palette-item ${i === index ? 'on' : ''} ${item.disabled ? 'disabled' : ''}`}
                  onMouseMove={() => i !== index && setIndex(i)}
                  onClick={() => pick(item)}
                >
                  <span className="palette-icon">{item.icon ?? (item.kind && KINDS[item.kind] ? <KindIcon kind={item.kind} /> : GROUP_ICON[item.group])}</span>
                  <span className="palette-label">
                    <Highlight text={item.label} at={at} />
                  </span>
                  {item.detail && <span className="palette-detail">{item.detail}</span>}
                  {q && <span className="palette-kind">{item.group === 'Commands' ? 'Command' : item.group === 'Tabs' ? 'Tab' : item.group}</span>}
                  <Kbd keys={item.keys} />
                </button>
              </div>
            )
          })}
        </div>
      </div>
    </Modal>
  )
}
