// The theme editor: a panel that floats over the app while you change a theme, so the whole
// window shows the draft as you go. Settings can stay open beside it or be closed to see the rest
// of the app. Pick (the pointer) finds the color of anything you click in the app. T3 Code's
// theme editor (MIT, © 2026 T3 Tools Inc.) works the same way.

import { ChevronDown, ChevronRight, ChevronUp, Code, Moon, MousePointer2, RotateCcw, Search, Sun, X } from 'lucide-react'
import type { PointerEvent as ReactPointerEvent } from 'react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { create } from 'zustand'
import { cssLoadsFiles } from '../lib/themeCss'
import { inspectRole } from '../lib/themeInspector'
import type { Appearance, Role, ThemeDef } from '../lib/themes'
import { isColor, isHex, ROLES, THEME_CSS_MAX_LENGTH, toHex6 } from '../lib/themes'
import { useAppearance } from '../state/appearance'
import { useWorkspace } from '../state/workspace'
import { Button, IconButton, Segmented, Switch, TextField } from './ui'

interface Session {
  theme: ThemeDef
  /** Changing a saved custom theme, not making a new one. */
  editing: boolean
  key: number
}

export const useThemeEditor = create<{ session: Session | null; open(theme: ThemeDef, editing: boolean): void; close(): void }>((set) => ({
  session: null,
  open: (theme, editing) => set({ session: { theme: structuredClone(theme), editing, key: Date.now() } }),
  close: () => set({ session: null }),
}))

export function ThemeEditorHost() {
  const session = useThemeEditor((s) => s.session)
  return session ? <ThemeEditorPanel key={session.key} session={session} /> : null
}

const GROUPS = [...new Set(ROLES.map((r) => r.group))]

/** "#rrggbb" for any CSS color, as the page draws it, for a color input. */
function swatchHex(color: string): string {
  const hex = toHex6(color)
  if (hex) return hex
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

function ThemeEditorPanel({ session }: { session: Session }) {
  const [draft, setDraft] = useState<ThemeDef>(session.theme)
  const [mode, setMode] = useState<Appearance>(() => useWorkspace.getState().theme)
  const [advanced, setAdvanced] = useState(() => !!(session.theme.variants?.light || session.theme.variants?.dark))
  const [query, setQuery] = useState('')
  const [cssOpen, setCssOpen] = useState(!!session.theme.css)
  const [picking, setPicking] = useState(false)
  const [selected, setSelected] = useState<Role | null>(null)
  const [minimized, setMinimized] = useState(false)
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null)
  const [, setTick] = useState(0)
  const panel = useRef<HTMLDivElement>(null)
  const drag = useRef<{ dx: number; dy: number } | null>(null)
  const close = useThemeEditor((s) => s.close)

  // The whole app shows the draft while the editor is open.
  useLayoutEffect(() => {
    useAppearance.getState().setPreview({ theme: draft, mode })
    // Read the colors the page now has, for the fields that show them.
    setTick((t) => t + 1)
  }, [draft, mode])
  // A layout cleanup, so a panel that replaces this one has set its own preview after it.
  useLayoutEffect(() => () => useAppearance.getState().setPreview(null), [])

  const own = draft.variants?.[mode] ?? {}
  const shown = (css: string) => getComputedStyle(document.documentElement).getPropertyValue(css).trim()
  const setRole = (role: Role, value: string | null) => {
    const colors = { ...own }
    if (value === null) delete colors[role]
    else colors[role] = value
    setDraft({ ...draft, variants: { ...draft.variants, [mode]: colors } })
  }

  const cssTooLong = (draft.css?.length ?? 0) > THEME_CSS_MAX_LENGTH
  const valid = draft.name.trim().length > 0 && isHex(draft.accent) && isHex(draft.light) && isHex(draft.dark) && !cssTooLong
  const cssWarning = draft.css && cssLoadsFiles(draft.css) ? 'Rules that load files (@import, url() other than data:) are left out.' : null

  // Pick: hover outlines what's under the pointer; a click finds its color.
  useEffect(() => {
    if (!picking) return
    const outline = document.createElement('div')
    outline.className = 'theme-pick-outline'
    document.body.appendChild(outline)
    const outside = (e: Event) => !panel.current?.contains(e.target as Node)
    const onMove = (e: PointerEvent) => {
      const el = e.target as Element
      if (!outside(e)) {
        outline.style.display = 'none'
        return
      }
      const r = el.getBoundingClientRect()
      Object.assign(outline.style, { display: 'block', left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px` })
    }
    const swallow = (e: Event) => {
      if (!outside(e)) return
      e.preventDefault()
      e.stopPropagation()
    }
    const onClick = (e: MouseEvent) => {
      if (!outside(e)) return
      swallow(e)
      const role = inspectRole(e.target as Element)
      setPicking(false)
      if (role) {
        setAdvanced(true)
        setQuery('')
        setSelected(role)
        setMinimized(false)
      }
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.preventDefault()
      e.stopPropagation()
      setPicking(false)
    }
    document.addEventListener('pointermove', onMove, true)
    document.addEventListener('pointerdown', swallow, true)
    document.addEventListener('mousedown', swallow, true)
    document.addEventListener('mouseup', swallow, true)
    document.addEventListener('click', onClick, true)
    window.addEventListener('keydown', onKey, true)
    document.documentElement.classList.add('theme-picking')
    return () => {
      outline.remove()
      document.removeEventListener('pointermove', onMove, true)
      document.removeEventListener('pointerdown', swallow, true)
      document.removeEventListener('mousedown', swallow, true)
      document.removeEventListener('mouseup', swallow, true)
      document.removeEventListener('click', onClick, true)
      window.removeEventListener('keydown', onKey, true)
      document.documentElement.classList.remove('theme-picking')
    }
  }, [picking])

  // The picked color's row comes into view.
  useEffect(() => {
    if (selected) panel.current?.querySelector(`[data-role="${selected}"]`)?.scrollIntoView({ block: 'nearest' })
  }, [selected, advanced])

  const save = () => {
    if (!valid) return
    const theme = { ...draft, name: draft.name.trim() }
    const { save: store, use, custom } = useAppearance.getState()
    const isNew = !custom.some((t) => t.id === theme.id)
    store(theme)
    // A new theme is used straight away.
    if (isNew) use(theme.id)
    close()
  }

  const onDragStart = (e: ReactPointerEvent<HTMLDivElement>) => {
    if ((e.target as HTMLElement).closest('button, input')) return
    const r = panel.current?.getBoundingClientRect()
    if (!r) return
    drag.current = { dx: e.clientX - r.left, dy: e.clientY - r.top }
    e.currentTarget.setPointerCapture(e.pointerId)
  }
  const onDragMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (!drag.current || !panel.current) return
    const w = panel.current.offsetWidth
    setPos({ x: Math.max(8 - w + 80, Math.min(window.innerWidth - 80, e.clientX - drag.current.dx)), y: Math.max(8, Math.min(window.innerHeight - 40, e.clientY - drag.current.dy)) })
  }

  const q = query.trim().toLowerCase()
  // The accent has its own row at the top.
  const roles = ROLES.filter((r) => r.key !== 'accent' && (!q || `${r.label} ${r.hint} ${r.key}`.toLowerCase().includes(q)))
  // One accent for both appearances, unless this one has its own (an imported theme's can).
  const accentRow = (
    <ColorRow
      role="accent"
      label="Accent"
      hint="Buttons, switches and focus rings"
      value={own.accent ?? draft.accent}
      selected={selected === 'accent'}
      onChange={(v) => {
        const hex = toHex6(v)
        if (!hex) return
        if (own.accent !== undefined) setRole('accent', hex)
        else setDraft({ ...draft, accent: hex })
      }}
    />
  )
  const toneKey = mode === 'dark' ? 'dark' : 'light'
  const selectedRole = ROLES.find((r) => r.key === selected)

  return (
    <div
      ref={panel}
      className={`theme-panel ${minimized ? 'minimized' : ''}`}
      role="dialog"
      aria-label={session.editing ? 'Edit theme' : 'Create theme'}
      data-theme-editor
      style={pos ? { left: pos.x, top: pos.y, right: 'auto', bottom: 'auto' } : undefined}
      onKeyDown={(e) => {
        if (e.key === 'Escape' && !picking) {
          e.stopPropagation()
          close()
        }
      }}
    >
      <div className="theme-panel-head" onPointerDown={onDragStart} onPointerMove={onDragMove} onPointerUp={() => (drag.current = null)}>
        <div className="theme-panel-title">
          <h3>{session.editing ? 'Edit theme' : 'Create theme'}</h3>
          {!minimized && <span>{picking ? 'Click anything in Writan · Esc to stop' : selectedRole ? `${selectedRole.label}: ${selectedRole.hint.toLowerCase()}` : 'Changes show everywhere as you make them'}</span>}
        </div>
        <IconButton label={picking ? 'Stop picking' : 'Pick a color from the app'} size="sm" active={picking} onClick={() => setPicking(!picking)}>
          <MousePointer2 size={14} />
        </IconButton>
        <IconButton label={minimized ? 'Expand' : 'Minimize'} size="sm" onClick={() => setMinimized(!minimized)}>
          {minimized ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
        </IconButton>
        <IconButton label="Close without saving" size="sm" onClick={close}>
          <X size={14} />
        </IconButton>
      </div>

      {!minimized && (
        <>
          <div className="theme-panel-body">
            <TextField label="Name" autoFocus value={draft.name} maxLength={48} placeholder="Theme name" onChange={(e) => setDraft({ ...draft, name: e.currentTarget.value })} />
            <div className="theme-panel-row">
              <span>Editing</span>
              <Segmented
                value={mode}
                onChange={setMode}
                options={[
                  { value: 'light', label: 'Light', icon: <Sun size={13} /> },
                  { value: 'dark', label: 'Dark', icon: <Moon size={13} /> },
                ]}
              />
            </div>

            <div className="theme-panel-row">
              <span>Every color</span>
              <Switch label="Show every color" checked={advanced} onChange={setAdvanced} />
            </div>

            {!advanced ? (
              <div className="theme-panel-colors">
                {accentRow}
                <ColorRow
                  label={mode === 'dark' ? 'Dark tone' : 'Light tone'}
                  hint="Tints every surface, border and text"
                  value={draft[toneKey]}
                  onChange={(v) => isHex(v) && setDraft({ ...draft, [toneKey]: v })}
                />
              </div>
            ) : (
              <>
                <TextField icon={<Search size={13} className="faint" />} value={query} placeholder="Find a color" onChange={(e) => setQuery(e.currentTarget.value)} />
                <div className="theme-panel-colors">
                  {accentRow}
                  <ColorRow label={mode === 'dark' ? 'Dark tone' : 'Light tone'} hint="What the rest is worked out from" value={draft[toneKey]} onChange={(v) => isHex(v) && setDraft({ ...draft, [toneKey]: v })} />
                  {GROUPS.map((group) => {
                    const inGroup = roles.filter((r) => r.group === group)
                    if (!inGroup.length) return null
                    return (
                      <section key={group} className="theme-panel-group">
                        <h4>{group}</h4>
                        {inGroup.map((r) => (
                          <ColorRow
                            key={r.key}
                            role={r.key}
                            label={r.label}
                            hint={r.hint}
                            value={own[r.key] ?? shown(r.css)}
                            changed={own[r.key] !== undefined}
                            selected={selected === r.key}
                            onFocus={() => setSelected(r.key)}
                            onChange={(v) => isColor(v) && setRole(r.key, v)}
                            onReset={() => setRole(r.key, null)}
                          />
                        ))}
                      </section>
                    )
                  })}
                  {!roles.length && <p className="settings-hint">No color matches “{query}”.</p>}
                </div>
              </>
            )}

            <div className="theme-panel-css">
              <button type="button" className="section-title" aria-expanded={cssOpen} onClick={() => setCssOpen(!cssOpen)}>
                {cssOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                <Code size={13} /> Custom CSS
              </button>
              {cssOpen && (
                <>
                  <p className="settings-hint">For what colors can’t change. Applied after Writan’s own styles and saved with the theme. Use :root[data-theme='dark'] for one appearance.</p>
                  <textarea className="theme-css" spellCheck={false} value={draft.css ?? ''} placeholder={'.titlebar {\n  border-bottom: 2px solid var(--accent);\n}'} onChange={(e) => setDraft({ ...draft, css: e.currentTarget.value })} />
                  {cssWarning && <p className="settings-hint warn">{cssWarning}</p>}
                  {cssTooLong && <p className="settings-hint warn">Custom CSS can be {THEME_CSS_MAX_LENGTH.toLocaleString()} characters at most.</p>}
                </>
              )}
            </div>
          </div>
          <div className="theme-panel-foot">
            <Button onClick={close}>Cancel</Button>
            <Button variant="primary" disabled={!valid} onClick={save}>
              {session.editing ? 'Save changes' : 'Save theme'}
            </Button>
          </div>
        </>
      )}
    </div>
  )
}

/** One color: a swatch to pick it, and its value to type. */
function ColorRow({
  role,
  label,
  hint,
  value,
  changed,
  selected,
  onChange,
  onReset,
  onFocus,
}: {
  role?: Role
  label: string
  hint: string
  value: string
  changed?: boolean
  selected?: boolean
  onChange: (value: string) => void
  onReset?: () => void
  onFocus?: () => void
}) {
  const [text, setText] = useState(value)
  useEffect(() => setText(value), [value])
  return (
    <div className={`theme-color-row ${selected ? 'on' : ''}`} data-role={role} onFocus={onFocus}>
      <input type="color" aria-label={label} value={swatchHex(value || '#000000')} onChange={(e) => onChange(e.currentTarget.value)} />
      <span className="theme-color-text">
        <span>{label}</span>
        <span className="faint">{hint}</span>
      </span>
      <input
        className="theme-color-value"
        aria-label={`${label} value`}
        spellCheck={false}
        value={text}
        onChange={(e) => {
          setText(e.currentTarget.value)
          if (isColor(e.currentTarget.value)) onChange(e.currentTarget.value.trim())
        }}
        onBlur={() => setText(value)}
      />
      {onReset && (
        <IconButton label="Back to the worked-out color" size="sm" disabled={!changed} onClick={onReset}>
          <RotateCcw size={12} />
        </IconButton>
      )}
    </div>
  )
}
