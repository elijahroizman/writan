// A workbook (or CSV file) open in the engine: its sheets, each sheet's grid geometry, the
// selection, and drawing blocks of cells.

import { host, onFontsChanged, preloadFonts } from '../host'
import { expectKind, loadEngine } from '../load'
import type { PaintItem, PaintStyle } from '../paint'
import { itemsHtml, MediaCache, StyleSheet } from '../paint'
import { BASE_FONTS } from '../preload'
import type { SheetsDoc } from '../wasm/office_wasm'

export interface GridInfo {
  rows: number
  cols: number
  /** The rows and columns in use (cells, merges, charts), from the top left. */
  content: [number, number]
  defaultWidth: number
  defaultHeight: number
  widths: [number, number][]
  heights: [number, number][]
  frozen: [number, number]
}

/** A sheet's column and row positions in pixels, from its grid. */
export class Geometry {
  readonly colX: Float64Array
  readonly rowY: Float64Array
  constructor(readonly info: GridInfo) {
    const w = new Map(info.widths)
    const h = new Map(info.heights)
    this.colX = new Float64Array(info.cols + 1)
    for (let c = 0, x = 0; c <= info.cols; c++) {
      this.colX[c] = x
      x += w.get(c) ?? info.defaultWidth
    }
    this.rowY = new Float64Array(info.rows + 1)
    for (let r = 0, y = 0; r <= info.rows; r++) {
      this.rowY[r] = y
      y += h.get(r) ?? info.defaultHeight
    }
  }
  get width() {
    return this.colX[this.info.cols]
  }
  get height() {
    return this.rowY[this.info.rows]
  }
  x(c: number) {
    return this.colX[Math.min(c, this.info.cols)]
  }
  y(r: number) {
    return this.rowY[Math.min(r, this.info.rows)]
  }
  /** The column at an x, by binary search. */
  colAt(x: number) {
    return search(this.colX, x)
  }
  rowAt(y: number) {
    return search(this.rowY, y)
  }
}

function search(a: Float64Array, v: number) {
  let lo = 0
  let hi = a.length - 2
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1
    if (a[mid] <= v) lo = mid
    else hi = mid - 1
  }
  return Math.max(0, lo)
}

export interface CellInfo {
  address: string
  input: string
  display: string
  numberFormat: string
  font: string | null
  size: number
  bold: boolean
  italic: boolean
  underline: boolean
  strike: boolean
  color: string | null
  fill: string | null
  align: string | null
  valign: string | null
  wrap: boolean
  merged: boolean
}

export type ChartKind = 'column' | 'bar' | 'line' | 'area' | 'pie' | 'doughnut' | 'scatter' | 'other'
export type ChartGrouping = 'standard' | 'stacked' | 'percentStacked'
export type LegendPosition = 'right' | 'bottom' | 'top' | 'left'

/** A chart as drawn: what it plots, worked out from the cells, and its box in grid pixels. */
export interface ChartView {
  kind: ChartKind
  grouping: ChartGrouping
  title: string | null
  legend: LegendPosition | null
  x: number
  y: number
  w: number
  h: number
  categories: string[]
  series: { name: string; color: string | null; values: (number | null)[]; x: (number | null)[] | null }[]
  palette: string[]
  range: string | null
}

export interface SheetState {
  sheet: number
  anchor: [number, number]
  focus: [number, number]
  canUndo: boolean
  canRedo: boolean
  dirty: boolean
  csv: boolean
  /** The formulas in a circular reference, in sheet order. */
  circular: { sheet: number; row: number; col: number }[]
}

let sheets = 0

/** Today, as a spreadsheet serial number (days since 30 December 1899, time as a fraction). */
function serialNow() {
  const d = new Date()
  return (d.getTime() - d.getTimezoneOffset() * 60_000) / 86_400_000 + 25569
}

export class SheetsEngine {
  readonly doc: SheetsDoc
  readonly styles: StyleSheet
  readonly media: MediaCache
  sheets: { name: string; hidden: boolean }[] = []
  state: SheetState = { sheet: 0, anchor: [0, 0], focus: [0, 0], canUndo: false, canRedo: false, dirty: false, csv: false, circular: [] }
  cell: CellInfo | null = null
  revision = 0
  tick = 0
  private geometry = new Map<number, { revision: number; g: Geometry }>()
  private chartCache = new Map<number, { revision: number; charts: ChartView[] }>()
  private listeners = new Set<() => void>()
  private stopFonts: () => void
  private disposed = false

  private constructor(doc: SheetsDoc) {
    this.doc = doc
    this.styles = new StyleSheet(`x${++sheets}s`)
    this.media = new MediaCache(() => null)
    this.stopFonts = onFontsChanged(() => {
      if (this.disposed) return
      this.styles.remeasure()
      this.doc.fontsChanged()
      this.refresh()
    })
  }

  static async open(bytes: Uint8Array | null, name: string): Promise<SheetsEngine> {
    const engine = await loadEngine()
    if (!/\.(csv|tsv)$/i.test(name) || bytes?.length) expectKind(bytes, 'sheets')
    await preloadFonts(BASE_FONTS)
    const doc = new engine.SheetsDoc(bytes ?? new Uint8Array(), name, host)
    doc.setNow(serialNow())
    const e = new SheetsEngine(doc)
    e.refresh()
    if (e.sheets[e.state.sheet]?.hidden) {
      const visible = e.sheets.findIndex((s) => !s.hidden)
      if (visible >= 0) e.run({ cmd: 'select', sheet: visible, anchor: [0, 0], focus: [0, 0] })
    }
    return e
  }

  subscribe = (f: () => void) => {
    this.listeners.add(f)
    return () => {
      this.listeners.delete(f)
    }
  }

  getTick = () => this.tick

  run<T = unknown>(cmd: Record<string, unknown> & { cmd: string }): T {
    const out = JSON.parse(this.doc.command(JSON.stringify(cmd))) as T
    this.refresh()
    return out
  }

  refresh() {
    if (this.disposed) return
    this.sheets = JSON.parse(this.doc.sheets())
    this.state = JSON.parse(this.doc.state())
    this.revision = this.doc.revision()
    const [r, c] = this.state.focus
    this.cell = JSON.parse(this.doc.cell(this.state.sheet, r, c))
    this.tick++
    for (const f of this.listeners) f()
  }

  geometryOf(sheet: number): Geometry {
    const hit = this.geometry.get(sheet)
    if (hit && hit.revision === this.revision) return hit.g
    const g = new Geometry(JSON.parse(this.doc.grid(sheet)))
    this.geometry.set(sheet, { revision: this.revision, g })
    return g
  }

  /** A sheet's charts, worked out once per change. */
  chartsOf(sheet: number): ChartView[] {
    const hit = this.chartCache.get(sheet)
    if (hit && hit.revision === this.revision) return hit.charts
    const charts = JSON.parse(this.doc.charts(sheet)) as ChartView[]
    this.chartCache.set(sheet, { revision: this.revision, charts })
    return charts
  }

  /** A block of cells drawn as HTML, in grid pixels. */
  paint(sheet: number, r0: number, r1: number, c0: number, c1: number) {
    const p = JSON.parse(this.doc.paint(sheet, r0, r1, c0, c1)) as { items: PaintItem[]; styles: PaintStyle[] }
    this.styles.add(p.styles)
    return itemsHtml(p.items, this.styles, this.media.url)
  }

  /** The selection as a range (rows and columns from the top left). */
  get range() {
    const { anchor, focus } = this.state
    return { r0: Math.min(anchor[0], focus[0]), c0: Math.min(anchor[1], focus[1]), r1: Math.max(anchor[0], focus[0]), c1: Math.max(anchor[1], focus[1]) }
  }

  save(): Uint8Array {
    return this.doc.save()
  }

  dispose() {
    this.disposed = true
    this.stopFonts()
    this.styles.dispose()
    this.listeners.clear()
    this.doc.free()
  }
}

/** A column's letters: 0 is A, 26 is AA. */
export function columnName(c: number) {
  let s = ''
  for (let n = c + 1; n > 0; n = Math.floor((n - 1) / 26)) s = String.fromCharCode(65 + ((n - 1) % 26)) + s
  return s
}
