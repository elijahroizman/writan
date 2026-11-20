// Writan's design system: every shared piece of UI lives here, so screens look alike. Build a
// screen out of these before writing new markup and CSS; if something is missing, add it here.
// Colours, radii and type come from the tokens at the top of styles/app.css (the brand board's
// light and dark palettes), which themes override.

import { ChevronDown, FileText } from 'lucide-react'
import type { ButtonHTMLAttributes, CSSProperties, InputHTMLAttributes, ReactNode } from 'react'
import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import type { DocKind } from '../lib/kinds'
import { KINDS } from '../lib/kinds'
import { shortcut } from '../lib/platform'

/* ───────────────────────── Brand ───────────────────────── */

/**
 * The mark: a W made of three blue sheets of paper on a dark tile. The tile stays dark in both
 * themes; a faint white ring keeps it from sinking into dark backgrounds. Readable down to 16 px.
 */
/** The app icon's three looks, from the brand board: dark tile (the default), accent blue, light tile. */
export type LogoVariant = 'dark' | 'blue' | 'light'

const LOGO_COLORS: Record<LogoVariant, { tile: string; left: string; right: string; front: string; back: string; edge?: string }> = {
  dark: { tile: '#1C1C1C', left: '#7FAAFF', right: '#2F66D8', front: '#4F8CFF', back: '#3B74EA' },
  blue: { tile: '#4F8CFF', left: '#FFFFFF', right: '#D5E3FF', front: '#F3F7FF', back: '#BFD3FF' },
  light: { tile: '#F5F5F7', left: '#7FAAFF', right: '#2F66D8', front: '#4F8CFF', back: '#3B74EA', edge: '#CFCFD4' },
}

/** The app icon: a W made of three sheets of paper. */
export function Logo({ size = 24, label, className = '', variant = 'dark' }: { size?: number; label?: string; className?: string; variant?: LogoVariant }) {
  const c = LOGO_COLORS[variant]
  return (
    <svg
      className={`logo ${className}`}
      width={size}
      height={size}
      viewBox="0 0 200 200"
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      {c.edge ? <rect x="1" y="1" width="198" height="198" rx="43" fill={c.tile} stroke={c.edge} strokeWidth="2" /> : <rect width="200" height="200" rx="44" fill={c.tile} />}
      <g transform="translate(100 102) scale(1.12) translate(-100 -102)" strokeLinejoin="round">
        <path d="M45 56 H67 L93 148 H71 Z" fill={c.left} stroke={c.left} strokeWidth="4" />
        <path d="M133 56 H155 L129 148 H107 Z" fill={c.right} stroke={c.right} strokeWidth="4" />
        <path d="M71 148 L89 88 H111 L129 148 H107 L100 124.7 L93 148 Z" fill={c.tile} stroke={c.tile} strokeWidth="9" />
        <path d="M71 148 L89 88 H100 V124.7 L93 148 Z" fill={c.front} stroke={c.front} strokeWidth="3" />
        <path d="M100 88 H111 L129 148 H107 L100 124.7 Z" fill={c.back} stroke={c.back} strokeWidth="3" />
      </g>
      {variant === 'dark' && <rect x="0.5" y="0.5" width="199" height="199" rx="43.5" fill="none" stroke="#FFFFFF" strokeOpacity="0.12" />}
    </svg>
  )
}

/** The logo beside the name, as on Home and the welcome screen. `size` is the logo's. */
export function Wordmark({ size = 32, variant }: { size?: number; variant?: LogoVariant }) {
  return (
    <span className="wordmark" style={{ '--wordmark-size': `${size}px` } as CSSProperties}>
      <Logo size={size} variant={variant} />
      <span className="wordmark-name">Writan</span>
    </span>
  )
}

/** A file type's icon, as drawn in tabs and lists. */
export function KindIcon({ kind, size = 14 }: { kind: DocKind; size?: number }) {
  const Icon = KINDS[kind]?.icon ?? FileText
  return <Icon size={size} strokeWidth={1.75} />
}

/** A file type's icon on its colour, as on Home's cards and in pickers. */
export function DocBadge({ kind, size = 22 }: { kind: DocKind; size?: number }) {
  return (
    <span className="doc-icon" style={{ background: KINDS[kind]?.color ?? 'var(--kind-txt)', width: size, height: size }}>
      <KindIcon kind={kind} size={Math.round(size * 0.62)} />
    </span>
  )
}

/* ───────────────────────── Buttons ───────────────────────── */

type ButtonVariant = 'default' | 'primary' | 'danger' | 'primary-danger' | 'quiet'

/** A labelled button. `primary` for the one action a screen leads to; `quiet` has no fill until hovered. */
export function Button({
  variant = 'default',
  block,
  className = '',
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant; block?: boolean }) {
  const variantClass = variant === 'default' ? '' : variant === 'primary-danger' ? 'primary danger' : variant
  return (
    <button type="button" className={`btn ${variantClass} ${block ? 'block' : ''} ${className}`} {...rest}>
      {children}
    </button>
  )
}

/** A button that reads as a link inside text. */
export function LinkButton({ className = '', children, ...rest }: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button type="button" className={`link-btn ${className}`} {...rest}>
      {children}
    </button>
  )
}

/** A tall button in an insert or annotate grid: an icon with a short label under it. `active` marks the selected tool. */
export function ToolButton({
  icon,
  label,
  active,
  className = '',
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { icon: ReactNode; label: ReactNode; active?: boolean }) {
  return (
    <button type="button" className={`tool-btn ${active ? 'on' : ''} ${className}`} {...rest}>
      {icon}
      <span>{label}</span>
    </button>
  )
}

/** An icon-only button. `label` is its tooltip and accessible name (written Mac-style; shown in this platform's keys). */
export function IconButton({
  label,
  active,
  size,
  children,
  className = '',
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { label: string; active?: boolean; size?: 'sm' }) {
  const text = shortcut(label)
  return (
    <button
      type="button"
      aria-label={text}
      title={text}
      aria-pressed={active}
      className={`icon-btn ${active ? 'active' : ''} ${size ?? ''} ${className}`}
      {...rest}
    >
      {children}
    </button>
  )
}

export function Section({
  title,
  actions,
  children,
  collapsible = false,
  defaultOpen = true,
  flush = false,
}: {
  title: ReactNode
  actions?: ReactNode
  children?: ReactNode
  collapsible?: boolean
  defaultOpen?: boolean
  flush?: boolean
}) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <section className={`section ${open ? '' : 'collapsed'}`}>
      <header className="section-header">
        {collapsible ? (
          // The chevron follows the title, so collapsible titles line up with the others.
          <button type="button" className="section-title" onClick={() => setOpen(!open)} aria-expanded={open}>
            {title}
            <ChevronDown size={14} className="section-chevron" aria-hidden />
          </button>
        ) : (
          <span className="section-title">{title}</span>
        )}
        {actions}
      </header>
      {open && children && <div className={`section-body ${flush ? 'flush' : ''}`}>{children}</div>}
    </section>
  )
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  disabled,
}: {
  value: T | null
  options: { value: T; label?: ReactNode; icon?: ReactNode; title?: string }[]
  onChange: (value: T) => void
  disabled?: boolean
}) {
  return (
    <div className="segmented" role="radiogroup">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          title={shortcut(o.title ?? (typeof o.label === 'string' ? o.label : ''))}
          aria-label={shortcut(o.title ?? (typeof o.label === 'string' ? o.label : o.value))}
          className={value === o.value ? 'on' : ''}
          disabled={disabled}
          onClick={() => onChange(o.value)}
        >
          {o.icon}
          {o.label}
        </button>
      ))}
    </div>
  )
}

/** Illustrated choices with native radio-button keyboard navigation. */
export function ChoiceTiles<T extends string>({ label, value, options, onChange }: {
  label: string
  value: T
  options: { value: T; label: string; icon: ReactNode }[]
  onChange: (value: T) => void
}) {
  const name = useId()
  return (
    <div className="choice-tiles" role="radiogroup" aria-label={label}>
      {options.map((option) => (
        <label className="choice-tile" key={option.value}>
          <input type="radio" name={name} value={option.value} checked={value === option.value} onChange={() => onChange(option.value)} />
          <span className="choice-tile-content">
            {option.icon}
            <span>{option.label}</span>
          </span>
        </label>
      ))}
    </div>
  )
}

/** Numeric input that commits on Enter/blur, with arrow-key nudging (⇧ for ×10). */
export function NumberField({
  label,
  value,
  onCommit,
  suffix,
  step = 1,
  precision = 0,
  disabled,
  min,
  max,
}: {
  label: ReactNode
  value: number | null
  onCommit: (value: number) => void
  suffix?: string
  step?: number
  precision?: number
  disabled?: boolean
  min?: number
  max?: number
}) {
  const format = (v: number | null) => (v == null || Number.isNaN(v) ? '' : String(Number(v.toFixed(precision))))
  const [text, setText] = useState(format(value))
  const focused = useRef(false)

  useEffect(() => {
    if (!focused.current) setText(format(value))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value])

  const clamp = (v: number) => Math.min(max ?? Infinity, Math.max(min ?? -Infinity, v))
  const commit = (raw: string) => {
    const parsed = Number(raw.replace(/[^0-9.+-]/g, ''))
    if (raw.trim() === '' || Number.isNaN(parsed)) {
      setText(format(value))
      return
    }
    const next = clamp(parsed)
    setText(format(next))
    if (next !== value) onCommit(next)
  }

  return (
    <label className="field">
      <span className="field-label">{label}</span>
      <input
        value={text}
        disabled={disabled}
        inputMode="decimal"
        onFocus={(e) => {
          focused.current = true
          e.currentTarget.select()
        }}
        onBlur={(e) => {
          focused.current = false
          commit(e.currentTarget.value)
        }}
        onChange={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            commit(e.currentTarget.value)
            e.currentTarget.blur()
          } else if (e.key === 'Escape') {
            setText(format(value))
            e.currentTarget.blur()
          } else if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
            e.preventDefault()
            const delta = (e.key === 'ArrowUp' ? step : -step) * (e.shiftKey ? 10 : 1)
            const next = clamp((value ?? 0) + delta)
            setText(format(next))
            onCommit(next)
          }
        }}
      />
      {suffix && <span className="field-suffix">{suffix}</span>}
    </label>
  )
}

export function SelectField({
  label,
  value,
  options,
  onChange,
  disabled,
  placeholder,
}: {
  label?: ReactNode
  value: string | null
  options: { value: string; label: string }[]
  onChange: (value: string) => void
  disabled?: boolean
  placeholder?: string
}) {
  const known = value != null && options.some((o) => o.value === value)
  const select = useRef<HTMLSelectElement>(null)
  return (
    <label
      className="field"
      onMouseDown={(e) => {
        // A click on the label beside the list should open it, not just focus it.
        const el = select.current
        if (!el || disabled || e.target === el) return
        e.preventDefault()
        el.focus()
        try {
          el.showPicker()
        } catch {
          // Older WebKit has no showPicker for selects; the list stays focused for the keyboard.
        }
      }}
    >
      {label && <span className="field-label">{label}</span>}
      <select ref={select} value={known ? value! : ''} disabled={disabled} onChange={(e) => onChange(e.currentTarget.value)}>
        {!known && <option value="">{value ?? placeholder ?? '—'}</option>}
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      <ChevronDown size={14} className="chevron" />
    </label>
  )
}

export function ColorField({
  value,
  onChange,
  label,
  disabled,
}: {
  value: string | null
  onChange: (hex: string) => void
  label?: string
  disabled?: boolean
}) {
  const hex = normalizeHex(value)
  // Shown with its #, as hex colors are written everywhere else.
  const shown = hex ? `#${hex}` : ''
  const [text, setText] = useState(shown)
  useEffect(() => setText(shown), [shown])
  return (
    <label className="field">
      <span className="color-input" style={{ background: hex ? `#${hex}` : 'transparent' }}>
        <input
          type="color"
          disabled={disabled}
          value={`#${hex ?? '000000'}`}
          onChange={(e) => onChange(e.currentTarget.value.slice(1).toUpperCase())}
        />
      </span>
      <input
        value={text}
        disabled={disabled}
        placeholder={label ?? 'Mixed'}
        spellCheck={false}
        onChange={(e) => setText(e.currentTarget.value)}
        onBlur={(e) => {
          const next = normalizeHex(e.currentTarget.value)
          if (next && next !== hex) onChange(next)
          else setText(shown)
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') e.currentTarget.blur()
        }}
      />
    </label>
  )
}

export function normalizeHex(value: string | null | undefined): string | null {
  if (!value) return null
  let v = value.trim().replace(/^#/, '')
  if (/^[0-9a-f]{3}$/i.test(v)) v = v.replace(/./g, (c) => c + c)
  return /^[0-9a-f]{6}$/i.test(v) ? v.toUpperCase() : null
}

export interface MenuEntry {
  label: string
  icon?: ReactNode
  shortcut?: string
  onSelect: () => void
  separatorBefore?: boolean
  danger?: boolean
  disabled?: boolean
}

/** A small popover menu anchored to a point or element. With `align: 'end'` its right edge sits at `x`. */
export function PopoverMenu({
  anchor,
  items,
  onClose,
}: {
  anchor: { x: number; y: number; align?: 'start' | 'end' }
  items: MenuEntry[]
  onClose: () => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState(anchor)

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const { innerWidth, innerHeight } = window
    const rect = el.getBoundingClientRect()
    setPos({
      x: Math.max(8, Math.min(anchor.align === 'end' ? anchor.x - rect.width : anchor.x, innerWidth - rect.width - 8)),
      y: anchor.y + rect.height > innerHeight - 8 ? Math.max(8, anchor.y - rect.height) : anchor.y,
    })
  }, [anchor])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  return createPortal(
    <>
      <div className="menu-backdrop" onMouseDown={onClose} onContextMenu={(e) => (e.preventDefault(), onClose())} />
      <div className="menu" ref={ref} style={{ left: pos.x, top: pos.y }} role="menu">
        {items.map((item, i) => (
          <div key={i}>
            {item.separatorBefore && <div className="menu-sep" />}
            <button
              type="button"
              role="menuitem"
              className="menu-item"
              disabled={item.disabled}
              style={item.danger ? { color: 'var(--danger)' } : undefined}
              onClick={() => {
                onClose()
                item.onSelect()
              }}
            >
              {item.icon}
              <span className="label">{item.label}</span>
              <Kbd keys={item.shortcut} />
            </button>
          </div>
        ))}
      </div>
    </>,
    document.body,
  )
}

/**
 * Insert-table size picker. The grid grows as the pointer nears its edge, up to `max`,
 * so a big table is one sweep away without a huge empty grid up front.
 */
export function TableSizePicker({ onPick, max = { rows: 20, cols: 10 } }: { onPick: (rows: number, cols: number) => void; max?: { rows: number; cols: number } }) {
  const [hover, setHover] = useState({ rows: 0, cols: 0 })
  const rows = Math.min(max.rows, Math.max(6, hover.rows + 1))
  const cols = Math.min(max.cols, Math.max(6, hover.cols + 1))
  return (
    <div className="table-picker" onMouseLeave={() => setHover({ rows: 0, cols: 0 })}>
      <div className="table-picker-grid" style={{ gridTemplateColumns: `repeat(${cols}, 18px)` }}>
        {Array.from({ length: rows * cols }, (_, i) => {
          const r = Math.floor(i / cols) + 1
          const c = (i % cols) + 1
          return (
            <button
              key={`${r}-${c}`}
              type="button"
              aria-label={`${r} by ${c}`}
              className={r <= hover.rows && c <= hover.cols ? 'on' : ''}
              onMouseEnter={() => setHover({ rows: r, cols: c })}
              onFocus={() => setHover({ rows: r, cols: c })}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => onPick(r, c)}
            />
          )
        })}
      </div>
      <span className="table-picker-caption">
        {hover.rows ? `${hover.rows} ${hover.rows === 1 ? 'row' : 'rows'} × ${hover.cols} ${hover.cols === 1 ? 'column' : 'columns'}` : 'Choose a size'}
      </span>
    </div>
  )
}

/** An on/off switch, for settings. */
export function Switch({ checked, onChange, label, disabled }: { checked: boolean; onChange: (checked: boolean) => void; label: string; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={`switch ${checked ? 'on' : ''}`}
      disabled={disabled}
      onClick={() => onChange(!checked)}
    >
      <span className="switch-knob" />
    </button>
  )
}

/* ───────────────────────── Text ───────────────────────── */

/** A shortcut, written Mac-style (⌘⇧K) and shown in this platform's keys. Renders nothing for an empty one. */
export function Kbd({ keys, className = '' }: { keys: string | null | undefined; className?: string }) {
  if (!keys) return null
  return <kbd className={`kbd ${className}`}>{shortcut(keys)}</kbd>
}

/** Secondary text under a label: “muted” for something to read, “faint” for something to skim. */
export function Hint({ tone = 'muted', as: Tag = 'span', className = '', children }: { tone?: 'muted' | 'faint'; as?: 'span' | 'p'; className?: string; children: ReactNode }) {
  return <Tag className={`hint ${tone} ${className}`}>{children}</Tag>
}

/** A dashed box saying there's nothing here yet. */
export function EmptyState({ children }: { children: ReactNode }) {
  return <div className="empty-state">{children}</div>
}

/* ───────────────────────── Surfaces ───────────────────────── */

/**
 * A dimmed layer over the app holding one panel (`children`): Settings, the setup screen, message
 * boxes, pickers. Clicking the dim area calls `onClose`; Esc does too when `closeOnEscape` is set
 * (leave it off if the panel handles its own keys). `placement: 'top'` sits a picker high up, like
 * the command palette; `dim: 'blur'` hides the app almost completely, for the welcome screen.
 * `layer` orders stacked modals: sheet (Settings) < picker < dialog (message boxes).
 */
export function Modal({
  onClose,
  closeOnEscape = false,
  placement = 'center',
  dim = 'normal',
  layer = 'sheet',
  className = '',
  children,
}: {
  onClose?: () => void
  closeOnEscape?: boolean
  placement?: 'center' | 'top'
  dim?: 'normal' | 'light' | 'blur'
  layer?: 'sheet' | 'picker' | 'welcome' | 'dialog'
  className?: string
  children: ReactNode
}) {
  const close = useRef(onClose)
  close.current = onClose
  useEffect(() => {
    if (!closeOnEscape) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || !close.current) return
      e.preventDefault()
      close.current()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [closeOnEscape])
  return (
    <div
      className={`modal-backdrop modal-${placement} modal-dim-${dim} modal-layer-${layer} ${className}`}
      onMouseDown={(e) => e.target === e.currentTarget && onClose?.()}
    >
      {children}
    </div>
  )
}

/** A raised, bordered surface for grouping (a card on Home, a preview in the welcome tour). */
export function Card({ className = '', children, style }: { className?: string; children: ReactNode; style?: CSSProperties }) {
  return (
    <div className={`card ${className}`} style={style}>
      {children}
    </div>
  )
}

/** A big clickable card with an icon, a title and a hint: Home's Open… and New … cards. */
export function ActionCard({
  icon,
  title,
  hint,
  keys,
  className = '',
  onClick,
}: {
  icon: ReactNode
  title: ReactNode
  hint?: ReactNode
  keys?: string | null
  className?: string
  onClick: () => void
}) {
  return (
    <button type="button" className={`action-card ${className}`} onClick={onClick}>
      {icon}
      <span className="action-card-text">
        <strong>{title}</strong>
        {hint && <span className="muted">{hint}</span>}
      </span>
      <Kbd keys={keys} />
    </button>
  )
}

/** A compact action in a list of them: icon, title and hint on one line (Home's New panel). */
export function ActionRow({ icon, title, hint, label, onClick }: { icon: ReactNode; title: ReactNode; hint?: ReactNode; label?: string; onClick: () => void }) {
  return (
    <button type="button" className="action-row" aria-label={label} onClick={onClick}>
      {icon}
      <span className="action-row-text">
        <strong>{title}</strong>
        {hint && <span className="muted">{hint}</span>}
      </span>
    </button>
  )
}

/* ───────────────────────── Settings & forms ───────────────────────── */

/** One setting: its name and a hint on the left, the control on the right (or below with `column`). `bare` leaves children unwrapped, for several blocks under the label. */
export function SettingsRow({
  label,
  hint,
  children,
  column,
  bare,
  className = '',
}: {
  label: ReactNode
  hint?: ReactNode
  children?: ReactNode
  column?: boolean
  bare?: boolean
  className?: string
}) {
  return (
    <div className={`settings-row ${column ? 'column' : ''} ${className}`}>
      <div className="settings-row-text">
        <span className="settings-label">{label}</span>
        {hint && <span className="settings-hint">{hint}</span>}
      </div>
      {children && (bare ? children : <div className="settings-row-control">{children}</div>)}
    </div>
  )
}

/** A checkbox with its label. */
export function Checkbox({
  checked,
  onChange,
  label,
  disabled,
  className = '',
}: {
  checked: boolean
  onChange: (checked: boolean) => void
  label: ReactNode
  disabled?: boolean
  className?: string
}) {
  return (
    <label className={`check ${className}`}>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.currentTarget.checked)} />
      {label}
    </label>
  )
}

/** A text input in the field style, with an optional leading icon (a search box, a name). */
export function TextField({ icon, label, className = '', ...rest }: InputHTMLAttributes<HTMLInputElement> & { icon?: ReactNode; label?: ReactNode }) {
  return (
    <label className={`field ${className}`}>
      {icon}
      {label && <span className="field-label">{label}</span>}
      <input spellCheck={false} {...rest} />
    </label>
  )
}

/**
 * A row of pill tabs that filter what's below (Home's Recent filter). Each option can carry a
 * colour dot, e.g. its file type's colour.
 */
export function PillTabs<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T
  options: { value: T; label: ReactNode; color?: string }[]
  onChange: (value: T) => void
  label: string
}) {
  return (
    <div className="pill-tabs" role="tablist" aria-label={label}>
      {options.map((o) => (
        <button key={o.value} type="button" role="tab" aria-selected={value === o.value} className={value === o.value ? 'on' : ''} onClick={() => onChange(o.value)}>
          {o.color && <span className="pill-dot" style={{ background: o.color }} />}
          {o.label}
        </button>
      ))}
    </div>
  )
}

/** Steps as dots: where you are in a short sequence (the welcome tour). */
export function StepDots({ count, current, onPick, label = 'Steps' }: { count: number; current: number; onPick?: (index: number) => void; label?: string }) {
  const id = useId()
  return (
    <div className="step-dots" role="tablist" aria-label={label}>
      {Array.from({ length: count }, (_, i) => (
        <button
          key={`${id}-${i}`}
          type="button"
          role="tab"
          aria-selected={i === current}
          aria-label={`Step ${i + 1} of ${count}`}
          className={i === current ? 'on' : i < current ? 'past' : ''}
          disabled={!onPick}
          onClick={() => onPick?.(i)}
        />
      ))}
    </div>
  )
}

export function Spinner({ size = 18 }: { size?: number }) {
  return (
    <svg className="spin" width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden>
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeOpacity="0.2" strokeWidth="2.5" />
      <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" />
    </svg>
  )
}

export function CenterState({ title, children, spinner }: { title?: string; children?: ReactNode; spinner?: boolean }) {
  return (
    <div className="center-state">
      <div className="box">
        {spinner && <Spinner size={22} />}
        {title && <h3>{title}</h3>}
        {children}
      </div>
    </div>
  )
}
