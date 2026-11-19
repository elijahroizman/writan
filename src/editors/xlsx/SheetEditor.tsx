import {
  AlignCenter,
  AlignLeft,
  AlignRight,
  ArrowDownToLine,
  ArrowLeftToLine,
  ArrowRightToLine,
  ArrowUpToLine,
  Bold,
  ChartArea,
  ChartBar,
  ChartColumn,
  ChartLine,
  ChartPie,
  ChartScatter,
  Donut,
  Italic,
  Merge,
  Minus,
  Plus,
  Redo2,
  Strikethrough,
  Trash2,
  Underline,
  Undo2,
  WrapText,
} from 'lucide-react'
import { useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import type { EditorProps } from '../../App'
import { FindBar, InspectorHeader, Workbench, formatZoom } from '../../components/Workbench'
import type { MenuEntry } from '../../components/ui'
import { Button, CenterState, ColorField, Hint, IconButton, NumberField, PopoverMenu, Section, Segmented, SelectField, TextField } from '../../components/ui'
import type { ChartKind, ChartView } from '../../engine/sheets/engine'
import { columnName, SheetsEngine } from '../../engine/sheets/engine'
import { a1, callAt, canPointAt, nameAt, quoteSheet, REF_COLORS, refsIn } from '../../engine/sheets/formula'
import type { SheetFunction } from '../../engine/sheets/functions'
import { COMMON, FUNCTIONS, functionByName } from '../../engine/sheets/functions'
import type { EditorHooks } from '../../engine/sheets/Grid'
import { Grid } from '../../engine/sheets/Grid'
import { showDialog } from '../../lib/dialogs'
import { usePinchZoom } from '../../lib/pinch'
import { FormulaHelp } from './FormulaHelp'
import { registerEditor, useWorkspace } from '../../state/workspace'
import './sheets.css'

const NUMBER_FORMATS = [
  { value: 'General', label: 'General' },
  { value: '0', label: 'Number (1234)' },
  { value: '#,##0.00', label: 'Number (1,234.00)' },
  { value: '"$"#,##0.00', label: 'Currency ($1,234.00)' },
  { value: '0%', label: 'Percent (12%)' },
  { value: '0.00%', label: 'Percent (12.34%)' },
  { value: 'yyyy-mm-dd', label: 'Date (2026-01-31)' },
  { value: 'm/d/yyyy', label: 'Date (1/31/2026)' },
  { value: 'mmm d, yyyy', label: 'Date (Jan 31, 2026)' },
  { value: 'h:mm AM/PM', label: 'Time (9:30 AM)' },
  { value: '0.00E+00', label: 'Scientific (1.23E+03)' },
  { value: '@', label: 'Text' },
]
const FONTS = ['Calibri', 'Arial', 'Helvetica', 'Times New Roman', 'Courier New', 'Georgia', 'Verdana', 'Aptos']
const ZOOM_STEPS = [0.5, 0.67, 0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2]
const CHART_KINDS: { kind: ChartKind; label: string; icon: React.ReactNode }[] = [
  { kind: 'column', label: 'Column', icon: <ChartColumn size={15} /> },
  { kind: 'bar', label: 'Bar', icon: <ChartBar size={15} /> },
  { kind: 'line', label: 'Line', icon: <ChartLine size={15} /> },
  { kind: 'area', label: 'Area', icon: <ChartArea size={15} /> },
  { kind: 'pie', label: 'Pie', icon: <ChartPie size={15} /> },
  { kind: 'doughnut', label: 'Doughnut', icon: <Donut size={15} /> },
  { kind: 'scatter', label: 'Scatter', icon: <ChartScatter size={15} /> },
]
const LEGENDS = [
  { value: 'none', label: 'None' },
  { value: 'right', label: 'Right' },
  { value: 'bottom', label: 'Bottom' },
  { value: 'top', label: 'Top' },
  { value: 'left', label: 'Left' },
]

export default function SheetEditor({ tab, active }: EditorProps) {
  const [engine, setEngine] = useState<SheetsEngine | null>(null)
  const [error, setError] = useState<string | null>(null)
  const name = useRef('')
  name.current = tab.path ?? `${tab.title}.${tab.kind}`
  useEffect(() => {
    let live = true
    let opened: SheetsEngine | null = null
    setEngine(null)
    setError(null)
    SheetsEngine.open(tab.bytes, name.current)
      .then((e) => {
        if (!live) return e.dispose()
        opened = e
        if (import.meta.env.DEV || import.meta.env.VITE_TEST_HOOKS) Object.assign(window, { __sheets: e })
        setEngine(e)
      })
      .catch((e) => { if (live) setError(String(e?.message ?? e)) })
    return () => {
      live = false
      opened?.dispose()
    }
  }, [tab.bytes, tab.kind])
  if (error) {
    return (
      <CenterState title="Couldn’t open this spreadsheet">
        <p className="muted">{error}</p>
      </CenterState>
    )
  }
  if (!engine) return <CenterState spinner />
  return <Book tab={tab} active={active} engine={engine} />
}

function Book({ tab, active, engine }: EditorProps & { engine: SheetsEngine }) {
  useSyncExternalStore(engine.subscribe, engine.getTick)
  const st = engine.state
  const cell = engine.cell
  const sheet = st.sheet
  const markDirty = useWorkspace((s) => s.markDirty)
  const [editing, setEditing] = useState<string | null>(null)
  const draft = useRef<{ text: string; original: string; sheet: number; row: number; col: number } | null>(null)
  const [zoom, setZoom] = useState(1)
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null)
  const [findOpen, setFindOpen] = useState<false | 'find' | 'replace'>(false)
  const [matches, setMatches] = useState<{ sheet: number; row: number; col: number }[]>([])
  const [matchIndex, setMatchIndex] = useState(0)
  const zoomRef = useRef(zoom)
  zoomRef.current = zoom

  useEffect(() => {
    let revision = engine.revision
    let dirty = engine.state.dirty
    return engine.subscribe(() => {
      if (engine.revision !== revision || engine.state.dirty !== dirty) {
        revision = engine.revision
        dirty = engine.state.dirty
        markDirty(tab.id, dirty || !!(draft.current && draft.current.text !== draft.current.original))
      }
    })
  }, [engine, markDirty, tab.id])

  // A formula that comes to depend on itself gets a warning, as in Excel: once when the file
  // opens with one, and again whenever an edit makes a new one.
  const circularSeen = useRef<Set<string> | null>(null)
  useEffect(() => {
    const check = () => {
      const now = new Set(engine.state.circular.map((c) => `${c.sheet}:${c.row}:${c.col}`))
      const fresh = engine.state.circular.find((c) => !circularSeen.current?.has(`${c.sheet}:${c.row}:${c.col}`))
      circularSeen.current = now
      if (fresh) {
        void showDialog({
          kind: 'warning',
          title: 'Circular reference',
          message: `A formula refers to its own cell, directly or through other formulas (${circularName(engine, fresh)}), so it can’t be worked out. It shows 0 until you change or remove it.`,
          buttons: [{ label: 'OK', value: 'ok', primary: true }],
          cancelValue: 'ok',
        })
      }
    }
    check()
    return engine.subscribe(check)
  }, [engine])

  // ── Typing a formula: suggestions, hints, and clicking cells to put their references in.
  const editorEl = useRef<HTMLInputElement | HTMLTextAreaElement | null>(null)
  const formulaInput = useRef<HTMLInputElement>(null)
  const [caret, setCaret] = useState(0)
  /** The reference last put in by clicking a cell, and its first cell: the next click replaces it. */
  const pointer = useRef<{ start: number; end: number; anchor: [number, number]; sheet: number } | null>(null)
  const pendingCaret = useRef<number | null>(null)
  const [pick, setPick] = useState({ index: 0, moved: false })
  const [dismissed, setDismissed] = useState<number | null>(null)

  /** What the person typed (or null to stop editing). */
  const edit = (text: string | null) => {
    pointer.current = null
    pendingCaret.current = null
    setDismissed(null)
    const el = editorEl.current
    if (text !== null && el) setCaret(el.selectionStart ?? text.length)
    setDraft(text)
  }

  const setDraft = (text: string | null) => {
    // Stopping on another sheet (clicked to while pointing) goes back to the cell being edited.
    const d0 = draft.current
    if (text === null && d0 && d0.sheet !== engine.state.sheet) engine.run({ cmd: 'select', sheet: d0.sheet, anchor: [d0.row, d0.col], focus: [d0.row, d0.col] })
    if (text === null) draft.current = null
    else if (draft.current) draft.current.text = text
    else {
      const { sheet, focus: [row, col] } = engine.state
      draft.current = { text, original: engine.cell?.input ?? '', sheet, row, col }
    }
    setEditing(text)
    markDirty(tab.id, engine.state.dirty || !!(draft.current && draft.current.text !== draft.current.original))
  }

  const flushDraft = () => {
    const d = draft.current
    if (!d) return
    draft.current = null
    setEditing(null)
    if (d.text !== d.original) engine.run({ cmd: 'input', sheet: d.sheet, row: d.row, col: d.col, text: d.text })
  }

  const range = engine.range
  const run = <T = unknown,>(cmd: Record<string, unknown> & { cmd: string }) => {
    flushDraft()
    return engine.run<T>(cmd)
  }
  const format = (patch: Record<string, unknown>) => run({ cmd: 'format', sheet, range, patch })

  /** Puts what was typed into the focused cell, then moves as Enter or Tab asks. */
  const commit = (text: string, move: 'down' | 'right' | 'up' | 'left' | 'none') => {
    const d = draft.current
    const [r, c] = d ? [d.row, d.col] : engine.state.focus
    const targetSheet = d?.sheet ?? engine.state.sheet
    draft.current = null
    setEditing(null)
    pointer.current = null
    if (!d || text !== d.original) engine.run({ cmd: 'input', sheet: targetSheet, row: r, col: c, text })
    const next: [number, number] = move === 'down' ? [r + 1, c] : move === 'up' ? [Math.max(0, r - 1), c] : move === 'right' ? [r, c + 1] : move === 'left' ? [r, Math.max(0, c - 1)] : [r, c]
    // Back on the cell's sheet, if a reference took the view to another.
    engine.run({ cmd: 'select', sheet: targetSheet, anchor: next, focus: next })
  }

  useLayoutEffect(() => {
    const p = pendingCaret.current
    const el = editorEl.current
    if (p === null || !el || !el.isConnected) return
    pendingCaret.current = null
    el.focus({ preventScroll: true })
    el.setSelectionRange(p, p)
    setCaret(p)
  }, [editing])

  const token = editing !== null ? nameAt(editing, caret) : null
  const suggestions = useMemo((): SheetFunction[] => {
    if (editing === null || !editing.startsWith('=')) return []
    if (editing === '=' && caret === 1) {
      if (dismissed === 1) return []
      const common = COMMON.map((n) => functionByName.get(n)!).filter(Boolean)
      return [...common, ...FUNCTIONS.filter((f) => !COMMON.includes(f.name)).sort((a, b) => a.name.localeCompare(b.name))]
    }
    if (!token || dismissed === token.start) return []
    const q = token.text.toUpperCase()
    return FUNCTIONS.filter((f) => f.name.startsWith(q)).sort((a, b) => a.name.length - b.name.length || a.name.localeCompare(b.name))
  }, [editing, caret, token?.start, token?.text, dismissed])
  useEffect(() => setPick({ index: 0, moved: false }), [token?.text, editing === '='])
  const call = editing !== null && suggestions.length === 0 ? callAt(editing, caret) : null
  const callFn = call ? functionByName.get(call.name) : undefined

  /** Puts a function's name and its bracket where its name was being typed. */
  const acceptFunction = (fn: SheetFunction) => {
    const text = draft.current?.text ?? editing ?? ''
    const start = token?.start ?? caret
    const tail = text.slice(caret)
    const insert = fn.args.length ? `${fn.name}(` : `${fn.name}()`
    pointer.current = null
    pendingCaret.current = start + insert.length
    setDraft(text.slice(0, start) + insert + tail)
  }

  const assistKey = (e: React.KeyboardEvent): boolean => {
    if (!suggestions.length) return false
    const n = suggestions.length
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      setPick((p) => ({ index: (p.index + (e.key === 'ArrowDown' ? 1 : n - 1)) % n, moved: true }))
      return true
    }
    if ((e.key === 'Tab' && !e.shiftKey) || (e.key === 'Enter' && pick.moved)) {
      e.preventDefault()
      acceptFunction(suggestions[Math.min(pick.index, n - 1)])
      return true
    }
    if (e.key === 'Escape') {
      e.preventDefault()
      setDismissed(token?.start ?? 1)
      return true
    }
    return false
  }

  const editorHooks: EditorHooks = {
    onFocus: (el) => {
      editorEl.current = el
      setCaret(el.selectionStart ?? el.value.length)
    },
    onCaret: (el) => {
      const c = el.selectionStart ?? 0
      if (pointer.current && pendingCaret.current === null && c !== pointer.current.end) pointer.current = null
      setCaret(c)
    },
    onKeyDown: assistKey,
  }

  const canPoint = () => {
    const d = draft.current
    const el = editorEl.current
    if (!d || !el || !el.isConnected || !d.text.startsWith('=')) return false
    if (pointer.current) return true
    return canPointAt(d.text, el.selectionStart ?? d.text.length)
  }

  const onPoint = (from: [number, number], to: [number, number], how: 'replace' | 'extend' | 'add' = 'replace'): [number, number] | undefined => {
    const d = draft.current
    const el = editorEl.current
    if (!d || !el) return
    const here = engine.state.sheet
    const last = pointer.current
    if (how === 'extend' && last?.sheet === here) from = last.anchor
    let ref = a1(from, to)
    if (here !== d.sheet) ref = `${quoteSheet(engine.sheets[here]?.name ?? '')}!${ref}`
    let text = d.text
    let start: number
    let at: number
    if (how === 'add' && last) {
      text = `${text.slice(0, last.end)},${text.slice(last.end)}`
      start = at = last.end + 1
    } else {
      at = last ? last.end : (el.selectionStart ?? text.length)
      start = last ? last.start : at
    }
    pointer.current = { start, end: start + ref.length, anchor: from, sheet: here }
    const next = text.slice(0, start) + ref + text.slice(at)
    // Moving within the same cell doesn't change editing, so its caret effect won't run.
    if (next === d.text) return from
    pendingCaret.current = start + ref.length
    setDraft(next)
    return from
  }

  const refBoxes = useMemo(() => {
    if (editing === null || !editing.startsWith('=')) return []
    const here = engine.sheets[sheet]?.name.toLowerCase()
    const home = draft.current?.sheet ?? sheet
    return refsIn(editing)
      .map((r, i) => ({ ...r, color: REF_COLORS[i % REF_COLORS.length] }))
      .filter((r) => (r.sheet ? r.sheet.toLowerCase() === here : home === sheet))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editing, sheet])

  /** Opens another sheet. While a formula is being typed, it stays open to point at that sheet's cells. */
  const openSheet = (i: number) => {
    if (draft.current && canPoint()) {
      engine.run({ cmd: 'select', sheet: i, anchor: [0, 0], focus: [0, 0] })
      // The cell's own editor is on its sheet; the formula bar carries on here.
      const el = formulaInput.current
      if (el) {
        const at = pointer.current?.end ?? editorEl.current?.selectionStart ?? draft.current.text.length
        editorEl.current = el
        requestAnimationFrame(() => {
          el.focus({ preventScroll: true })
          el.setSelectionRange(at, at)
          setCaret(at)
        })
      }
      return
    }
    run({ cmd: 'select', sheet: i, anchor: [0, 0], focus: [0, 0] })
  }

  // ── Charts
  const charts = engine.chartsOf(sheet)
  const [chartSel, setChartSel] = useState<number | null>(null)
  const [chartNote, setChartNote] = useState<string | null>(null)
  useEffect(() => setChartSel(null), [sheet])
  useEffect(() => {
    if (chartSel !== null && chartSel >= charts.length) setChartSel(null)
  }, [chartSel, charts.length])
  const chart = chartSel !== null ? (charts[chartSel] ?? null) : null
  const setChart = (patch: Record<string, unknown>) => chartSel !== null && run({ cmd: 'setChart', sheet, index: chartSel, patch })
  const insertChart = (kind: ChartKind) => {
    const index = run<number | null>({ cmd: 'addChart', sheet, range: engine.range, kind })
    if (index === null) setChartNote('Select cells with numbers to chart, with their labels if they have any.')
    else {
      setChartNote(null)
      setChartSel(index)
    }
  }
  const chartTypeMenu = (x: number, y: number) =>
    setMenu({ x, y, items: CHART_KINDS.map((k) => ({ label: `${k.label} chart`, icon: k.icon, onSelect: () => insertChart(k.kind) })) })
  const chartMenu = (index: number, x: number, y: number) =>
    setMenu({
      x,
      y,
      items: [{ label: 'Delete chart', icon: <Trash2 size={14} />, danger: true, onSelect: () => run({ cmd: 'deleteChart', sheet, index }) }],
    })

  // A trackpad pinch (or ctrl-wheel) scales the grid while the fingers move, then zooms to it.
  const gridRef = useRef<HTMLDivElement>(null)
  usePinchZoom(gridRef, {
    getZoom: () => zoomRef.current,
    setZoom,
    target: () => gridRef.current?.querySelector<HTMLElement>('.sg-space') ?? null,
    min: ZOOM_STEPS[0],
    max: ZOOM_STEPS[ZOOM_STEPS.length - 1],
  })

  const zoomStep = (dir: 'in' | 'out' | 'fit' | 'reset') => {
    if (dir === 'fit' || dir === 'reset') return setZoom(1)
    const z = zoomRef.current
    const next = dir === 'in' ? ZOOM_STEPS.find((s) => s > z + 0.001) : [...ZOOM_STEPS].reverse().find((s) => s < z - 0.001)
    if (next) setZoom(next)
  }

  // ── Find across every sheet.
  const query = useRef('')
  const runFind = (q: string, from = 0) => {
    query.current = q
    const found = q.trim() ? run<{ sheet: number; row: number; col: number }[]>({ cmd: 'find', query: q }) : []
    setMatches(found)
    const i = Math.min(from, Math.max(0, found.length - 1))
    setMatchIndex(i)
    if (found[i]) goTo(found[i])
  }
  const goTo = (m: { sheet: number; row: number; col: number }) => run({ cmd: 'select', sheet: m.sheet, anchor: [m.row, m.col], focus: [m.row, m.col] })
  const stepFind = (d: number) => {
    if (!matches.length) return
    const i = (matchIndex + d + matches.length) % matches.length
    setMatchIndex(i)
    goTo(matches[i])
  }

  useEffect(
    () =>
      registerEditor(tab.id, {
        prepareSave: flushDraft,
        getRevision: () => engine.revision,
        getBytes: async () => { flushDraft(); return engine.save() },
        busy: () => draft.current !== null,
        markSaved: () => {
          engine.run({ cmd: 'markSaved' })
          markDirty(tab.id, false)
        },
        find: () => setFindOpen('find'),
        replace: () => setFindOpen('replace'),
        zoom: (dir) => zoomStep(dir),
        actions: {
          'text.bold': () => void engine.run({ cmd: 'format', sheet: engine.state.sheet, range: engine.range, patch: { bold: !engine.cell?.bold } }),
          'text.italic': () => void engine.run({ cmd: 'format', sheet: engine.state.sheet, range: engine.range, patch: { italic: !engine.cell?.italic } }),
          'text.underline': () => void engine.run({ cmd: 'format', sheet: engine.state.sheet, range: engine.range, patch: { underline: !engine.cell?.underline } }),
        },
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tab.id, engine, markDirty],
  )

  const sheetMenu = (index: number, x: number, y: number) =>
    setMenu({
      x,
      y,
      items: [
        {
          label: 'Rename…',
          onSelect: () => {
            const name = window.prompt('Sheet name', engine.sheets[index]?.name ?? '')
            if (name) run({ cmd: 'renameSheet', sheet: index, name })
          },
        },
        ...(index > 0 ? [{ label: 'Move up', onSelect: () => run({ cmd: 'moveSheet', from: index, to: index - 1 }) }] : []),
        ...(index < engine.sheets.length - 1 ? [{ label: 'Move down', onSelect: () => run({ cmd: 'moveSheet', from: index, to: index + 1 }) }] : []),
        { label: 'Delete sheet', icon: <Trash2 size={14} />, danger: true, separatorBefore: true, disabled: engine.sheets.length <= 1, onSelect: () => run({ cmd: 'deleteSheet', sheet: index }) },
      ],
    })

  const cellMenu = (x: number, y: number) => {
    const range = engine.range
    const sheet = engine.state.sheet
    const cell = engine.cell
    setMenu({
      x,
      y,
      items: [
        { label: 'Insert row above', icon: <ArrowUpToLine size={14} />, onSelect: () => run({ cmd: 'shiftRows', sheet, at: range.r0, n: 1 }) },
        { label: 'Insert column left', icon: <ArrowLeftToLine size={14} />, onSelect: () => run({ cmd: 'shiftCols', sheet, at: range.c0, n: 1 }) },
        { label: 'Delete rows', icon: <Minus size={14} />, separatorBefore: true, onSelect: () => run({ cmd: 'shiftRows', sheet, at: range.r0, n: -(range.r1 - range.r0 + 1) }) },
        { label: 'Delete columns', icon: <Minus size={14} />, onSelect: () => run({ cmd: 'shiftCols', sheet, at: range.c0, n: -(range.c1 - range.c0 + 1) }) },
        { label: cell?.merged ? 'Unmerge' : 'Merge cells', icon: <Merge size={14} />, separatorBefore: true, onSelect: () => run({ cmd: 'merge', sheet, range }) },
        ...(st.csv ? [] : [{ label: 'Insert chart…', icon: <ChartColumn size={14} />, separatorBefore: true, onSelect: () => setTimeout(() => chartTypeMenu(x, y)) }]),
        { label: 'Clear', icon: <Trash2 size={14} />, danger: true, separatorBefore: true, onSelect: () => run({ cmd: 'clear', sheet, range }) },
      ],
    })
  }

  const left = (
    <nav className="list" aria-label="Sheets">
      {engine.sheets.map((s, i) => !s.hidden && (
        <button
          key={`${i}-${s.name}`}
          type="button"
          className={`list-item ${i === sheet ? 'active' : ''}`}
          onClick={() => openSheet(i)}
          onDoubleClick={() => {
            const name = window.prompt('Sheet name', s.name)
            if (name) run({ cmd: 'renameSheet', sheet: i, name })
          }}
          onContextMenu={(e) => {
            e.preventDefault()
            sheetMenu(i, e.clientX, e.clientY)
          }}
        >
          <span className="label">{s.name}</span>
        </button>
      ))}
      {!st.csv && (
        <button type="button" className="list-item add-sheet" onClick={() => run({ cmd: 'addSheet' })}>
          <Plus size={13} /> <span className="label">Add sheet</span>
        </button>
      )}
    </nav>
  )

  const geometry = engine.geometryOf(sheet)
  const size = range.r1 - range.r0 + 1
  const width = range.c1 - range.c0 + 1

  return (
    <Workbench
      tab={tab}
      left={left}
      leftFooter={<span>{engine.sheets.length === 1 ? '1 sheet' : `${engine.sheets.length} sheets`}</span>}
      right={
        chart ? (
          <ChartPanel
            key={`${sheet}-${chartSel}`}
            chart={chart}
            kind={tab.kind}
            onChange={setChart}
            onDelete={() => {
              run({ cmd: 'deleteChart', sheet, index: chartSel! })
              setChartSel(null)
            }}
          />
        ) : (
        <>
          <InspectorHeader kind={tab.kind}>{st.csv ? 'CSV file' : 'Workbook'}</InspectorHeader>
          <Section title={size * width > 1 ? `${size * width} cells` : `Cell ${cell?.address ?? ''}`}>
            <SelectField label="Number format" value={NUMBER_FORMATS.some((f) => f.value === cell?.numberFormat) ? (cell?.numberFormat ?? null) : null} placeholder={cell?.numberFormat || 'Custom'} options={NUMBER_FORMATS} onChange={(v) => format({ numberFormat: v })} />
          </Section>
          <Section title="Text">
            <div className="docx-font-row">
              <SelectField value={cell?.font ?? 'Calibri'} options={[...new Set([...(cell?.font ? [cell.font] : []), ...FONTS])].map((f) => ({ value: f, label: f }))} onChange={(f) => format({ font: f })} />
              <NumberField label={<span className="faint">pt</span>} value={cell?.size ?? 11} precision={1} step={1} min={1} max={409} onCommit={(v) => format({ size: v })} />
            </div>
            <div className="toggle-row">
              <IconButton label="Bold (⌘B)" active={!!cell?.bold} onClick={() => format({ bold: !cell?.bold })}>
                <Bold size={15} />
              </IconButton>
              <IconButton label="Italic (⌘I)" active={!!cell?.italic} onClick={() => format({ italic: !cell?.italic })}>
                <Italic size={15} />
              </IconButton>
              <IconButton label="Underline (⌘U)" active={!!cell?.underline} onClick={() => format({ underline: !cell?.underline })}>
                <Underline size={15} />
              </IconButton>
              <IconButton label="Strikethrough" active={!!cell?.strike} onClick={() => format({ strike: !cell?.strike })}>
                <Strikethrough size={15} />
              </IconButton>
              <IconButton label="Wrap text" active={!!cell?.wrap} onClick={() => format({ wrap: !cell?.wrap })}>
                <WrapText size={15} />
              </IconButton>
            </div>
            <div className="grid-2">
              <ColorField value={cell?.color ?? null} label="Text" onChange={(hex) => format({ color: hex })} />
              <ColorField value={cell?.fill ?? null} label="Fill" onChange={(hex) => format({ fill: hex })} />
            </div>
            {cell?.fill && (
              <Button block onClick={() => format({ fill: '' })}>
                No fill
              </Button>
            )}
          </Section>
          <Section title="Alignment">
            <Segmented
              value={(cell?.align as 'left' | 'center' | 'right' | null) ?? null}
              onChange={(a) => format({ align: a })}
              options={[
                { value: 'left', icon: <AlignLeft size={15} />, title: 'Align left' },
                { value: 'center', icon: <AlignCenter size={15} />, title: 'Center' },
                { value: 'right', icon: <AlignRight size={15} />, title: 'Align right' },
              ]}
            />
            <Segmented
              value={(cell?.valign as 'top' | 'center' | null) ?? 'bottom'}
              onChange={(a) => format({ valign: a })}
              options={[
                { value: 'top', label: 'Top' },
                { value: 'center', label: 'Middle' },
                { value: 'bottom', label: 'Bottom' },
              ]}
            />
          </Section>
          <Section title="Chart">
            <Button block disabled={st.csv} onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect()
              chartTypeMenu(r.left, r.bottom + 4)
            }}>
              <ChartColumn size={14} /> Insert chart
            </Button>
            <Hint tone="faint" as="p">
              {st.csv ? 'CSV files hold only cells. Save as an Excel workbook to add charts.' : (chartNote ?? 'Charts the selected cells, or the block of cells around the selected one.')}
            </Hint>
          </Section>
          <Section title="Borders" collapsible defaultOpen={false}>
            <div className="grid-2">
              <Button onClick={() => format({ borders: 'all' })}>All</Button>
              <Button onClick={() => format({ borders: 'outside' })}>Outside</Button>
              <Button onClick={() => format({ borders: 'bottom' })}>Bottom</Button>
              <Button onClick={() => format({ borders: 'none' })}>None</Button>
            </div>
          </Section>
          <Section title="Rows and columns">
            <div className="grid-2">
              <NumberField label="Rows" value={geometry.info.rows} min={1} max={1048576} onCommit={(rows) => run({ cmd: 'sheetSize', sheet, rows, cols: geometry.info.cols })} />
              <NumberField label="Columns" value={geometry.info.cols} min={1} max={16384} onCommit={(cols) => run({ cmd: 'sheetSize', sheet, rows: geometry.info.rows, cols })} />
            </div>
            <Hint tone="faint" as="p">
              {st.csv ? 'How many the sheet shows while it’s open. CSV files keep only cells.' : 'How many the sheet has, saved with the file. It never cuts off cells in use.'}
            </Hint>
            <span className="field-caption">Rows</span>
            <div className="toggle-row">
              <IconButton label="Insert row above" onClick={() => run({ cmd: 'shiftRows', sheet, at: range.r0, n: 1 })}>
                <ArrowUpToLine size={15} />
              </IconButton>
              <IconButton label="Insert row below" onClick={() => run({ cmd: 'shiftRows', sheet, at: range.r1 + 1, n: 1 })}>
                <ArrowDownToLine size={15} />
              </IconButton>
              <IconButton label="Delete rows" onClick={() => run({ cmd: 'shiftRows', sheet, at: range.r0, n: -size })}>
                <Minus size={15} />
              </IconButton>
            </div>
            <span className="field-caption">Columns</span>
            <div className="toggle-row">
              <IconButton label="Insert column left" onClick={() => run({ cmd: 'shiftCols', sheet, at: range.c0, n: 1 })}>
                <ArrowLeftToLine size={15} />
              </IconButton>
              <IconButton label="Insert column right" onClick={() => run({ cmd: 'shiftCols', sheet, at: range.c1 + 1, n: 1 })}>
                <ArrowRightToLine size={15} />
              </IconButton>
              <IconButton label="Delete columns" onClick={() => run({ cmd: 'shiftCols', sheet, at: range.c0, n: -width })}>
                <Minus size={15} />
              </IconButton>
            </div>
            <Button block onClick={() => run({ cmd: 'merge', sheet, range })}>
              <Merge size={14} /> {cell?.merged ? 'Unmerge cells' : 'Merge cells'}
            </Button>
            <Hint tone="faint" as="p">Drag a column or row edge in the headers to resize it.</Hint>
          </Section>
          {!st.csv && (
            <Section title="Sheet" collapsible defaultOpen={false}>
              <TextField
                key={`${sheet}-${engine.sheets[sheet]?.name}`}
                defaultValue={engine.sheets[sheet]?.name ?? ''}
                aria-label="Sheet name"
                onKeyDown={(e) => {
                  if (e.key === 'Enter') run({ cmd: 'renameSheet', sheet, name: e.currentTarget.value })
                }}
                onBlur={(e) => run({ cmd: 'renameSheet', sheet, name: e.currentTarget.value })}
              />
              <Button variant="danger" block disabled={engine.sheets.length <= 1} onClick={() => run({ cmd: 'deleteSheet', sheet })}>
                <Trash2 size={14} /> Delete sheet
              </Button>
            </Section>
          )}
          <Section title="File" collapsible defaultOpen={false}>
            <p className="faint" style={{ margin: 0, fontSize: 12 }}>
              {tab.path ? tab.path.replace(/^\/Users\/[^/]+/, '~') : 'Not saved yet'}
            </p>
          </Section>
        </>
        )
      }
    >
      <div className="sheet-stage">
        <div className="formula-bar">
          <span className="formula-address">{cell?.address}</span>
          <span className="formula-fx">fx</span>
          <input
            ref={formulaInput}
            className="formula-input"
            data-engine-input=""
            aria-label="Cell contents"
            value={editing ?? cell?.input ?? ''}
            spellCheck={false}
            onFocus={(e) => editorHooks.onFocus(e.currentTarget)}
            onSelect={(e) => editorHooks.onCaret(e.currentTarget)}
            onChange={(e) => edit(e.target.value)}
            onKeyDown={(e) => {
              if (assistKey(e)) {
                e.stopPropagation()
                return
              }
              if (e.key === 'Enter') {
                e.preventDefault()
                commit(editing ?? cell?.input ?? '', 'down')
              } else if (e.key === 'Escape') {
                e.preventDefault()
                edit(null)
              } else if (e.key === 'Tab') {
                e.preventDefault()
                commit(editing ?? cell?.input ?? '', 'right')
              }
              e.stopPropagation()
            }}
          />
        </div>
        <div className="sheet-grid" ref={gridRef}>
          <Grid
            engine={engine}
            zoom={zoom}
            active={active}
            editing={editing}
            onEdit={edit}
            onCommit={commit}
            onContextMenu={cellMenu}
            editingHere={!draft.current || draft.current.sheet === sheet}
            canPoint={canPoint}
            onPoint={onPoint}
            refBoxes={refBoxes}
            editorHooks={editorHooks}
            charts={charts}
            selectedChart={chartSel}
            onSelectChart={setChartSel}
            onChartMenu={chartMenu}
          />
        </div>
      </div>
      <FindBar
        open={!!findOpen}
        replacing={findOpen === 'replace'}
        onClose={() => setFindOpen(false)}
        onQuery={(q) => runFind(q)}
        onNext={() => stepFind(1)}
        onPrev={() => stepFind(-1)}
        onReplace={(by) => {
          run({ cmd: 'replaceMatch', index: matchIndex, text: by })
          runFind(query.current, matchIndex)
        }}
        onReplaceAll={(by) => {
          run({ cmd: 'replaceAll', text: by })
          runFind(query.current)
        }}
        count={matches.length}
        current={matchIndex}
        placeholder="Find in cells"
      />
      <div className="canvas-bar">
        <IconButton label="Undo (⌘Z)" size="sm" disabled={!st.canUndo} onClick={() => run({ cmd: 'undo' })}>
          <Undo2 size={14} />
        </IconButton>
        <IconButton label="Redo (⇧⌘Z)" size="sm" disabled={!st.canRedo} onClick={() => run({ cmd: 'redo' })}>
          <Redo2 size={14} />
        </IconButton>
        <span className="sep" />
        <span className="readout">{engine.sheets[sheet]?.name}</span>
        <span className="sep" />
        {st.circular.length > 0 && (
          <>
            <button
              type="button"
              className="readout circular"
              title="A formula refers to its own cell. Click to go to it."
              onClick={() => {
                const c = st.circular.find((c) => c.sheet === sheet) ?? st.circular[0]
                engine.run({ cmd: 'select', sheet: c.sheet, anchor: [c.row, c.col], focus: [c.row, c.col] })
              }}
            >
              Circular reference: {circularName(engine, st.circular.find((c) => c.sheet === sheet) ?? st.circular[0])}
            </button>
            <span className="sep" />
          </>
        )}
        <IconButton label="Zoom out (⌘−)" size="sm" onClick={() => zoomStep('out')} disabled={zoom <= ZOOM_STEPS[0] + 0.001}>
          <Minus size={14} />
        </IconButton>
        <IconButton label="Actual size (⌘0)" size="sm" className="readout" aria-label={formatZoom(zoom)} onClick={() => zoomStep('fit')}>
          {formatZoom(zoom)}
        </IconButton>
        <IconButton label="Zoom in (⌘+)" size="sm" onClick={() => zoomStep('in')} disabled={zoom >= ZOOM_STEPS[ZOOM_STEPS.length - 1] - 0.001}>
          <Plus size={14} />
        </IconButton>
      </div>
      {menu && <PopoverMenu anchor={menu} items={menu.items} onClose={() => setMenu(null)} />}
      {editing !== null && active && editorEl.current?.isConnected && (suggestions.length > 0 || callFn) && (
        <FormulaHelp
          anchor={editorEl.current}
          items={suggestions}
          index={Math.min(pick.index, Math.max(0, suggestions.length - 1))}
          call={callFn && call ? { fn: callFn, arg: call.arg } : null}
          onPick={acceptFunction}
          onHover={(index) => setPick({ index, moved: true })}
        />
      )}
    </Workbench>
  )
}

/** A cell in a circular reference as Excel's status bar names it: `A1`, or `Sheet2!A1` on another sheet. */
function circularName(engine: SheetsEngine, c: { sheet: number; row: number; col: number }) {
  const cell = `${columnName(c.col)}${c.row + 1}`
  return c.sheet === engine.state.sheet ? cell : `${quoteSheet(engine.sheets[c.sheet]?.name ?? '')}!${cell}`
}

/** The inspector for a selected chart: its kind, title, cells, colors and legend. */
function ChartPanel({ chart, kind, onChange, onDelete }: { chart: ChartView; kind: string; onChange: (patch: Record<string, unknown>) => void; onDelete: () => void }) {
  const stackable = ['column', 'bar', 'line', 'area'].includes(chart.kind)
  const pie = chart.kind === 'pie' || chart.kind === 'doughnut'
  const commitOnEnter = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') e.currentTarget.blur()
    e.stopPropagation()
  }
  return (
    <>
      <InspectorHeader kind={kind}>Chart</InspectorHeader>
      <Section title="Type">
        <div className="chart-kinds">
          {CHART_KINDS.map((k) => (
            <IconButton key={k.kind} label={k.label} active={chart.kind === k.kind} onClick={() => onChange({ kind: k.kind })}>
              {k.icon}
            </IconButton>
          ))}
        </div>
        {chart.kind === 'other' && <Hint tone="faint" as="p">This kind of chart isn’t drawn here. Picking a type replaces it.</Hint>}
        {stackable && (
          <Segmented
            value={chart.grouping}
            onChange={(g) => onChange({ grouping: g })}
            options={[
              { value: 'standard', label: chart.kind === 'column' || chart.kind === 'bar' ? 'Side by side' : 'Overlapping' },
              { value: 'stacked', label: 'Stacked' },
              { value: 'percentStacked', label: '100%' },
            ]}
          />
        )}
      </Section>
      <Section title="Title">
        <TextField
          key={chart.title ?? ''}
          defaultValue={chart.title ?? ''}
          placeholder="No title"
          aria-label="Chart title"
          onKeyDown={commitOnEnter}
          onBlur={(e) => {
            if (e.currentTarget.value.trim() !== (chart.title ?? '')) onChange({ title: e.currentTarget.value })
          }}
        />
      </Section>
      <Section title="Data">
        <TextField
          key={chart.range ?? ''}
          defaultValue={chart.range ?? ''}
          placeholder="A1:C7"
          aria-label="Cells the chart plots"
          spellCheck={false}
          onKeyDown={commitOnEnter}
          onBlur={(e) => {
            const v = e.currentTarget.value.trim()
            if (v && v !== (chart.range ?? '')) onChange({ range: v })
          }}
        />
        <Hint tone="faint" as="p">
          The cells it plots. A first row and column of labels name the series and categories.
        </Hint>
        {!pie &&
          chart.series.map((s, i) => (
            <div key={i} className="chart-series-row">
              <span title={s.name}>{s.name}</span>
              <ColorField value={(s.color ?? chart.palette[i % chart.palette.length] ?? '').replace('#', '')} onChange={(hex) => onChange({ color: [i, hex] })} />
            </div>
          ))}
      </Section>
      <Section title="Legend">
        <SelectField value={chart.legend ?? 'none'} options={LEGENDS} onChange={(v) => onChange({ legend: v })} />
      </Section>
      <Section title="Chart">
        <Button variant="danger" block onClick={onDelete}>
          <Trash2 size={14} /> Delete chart
        </Button>
        <Hint tone="faint" as="p">Drag a chart to move it, or its handles to resize it.</Hint>
      </Section>
    </>
  )
}
