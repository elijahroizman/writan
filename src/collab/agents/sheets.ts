// What an agent can do in a workbook: read cells with their formulas, and do what people do in
// the editor: write and clear cells, format them, size columns and rows, insert and delete rows
// and columns, merge cells, add charts, and add, rename, move and delete sheets.

import type { SheetsEngine } from '../../engine/sheets/engine'
import { type Answer, type Args, bool, doc, hexColor, int, need, num, plural, str, type Tool, ToolError } from './common'

const sheet = { type: 'string', description: 'The sheet’s name (the first one if left out).' }
const range = { type: 'string', description: 'Cells like B3, or a range like A1:F40.' }

export const SHEETS_TOOLS: Tool[] = [
  {
    name: 'write_cells',
    description: 'Excel: writes cells, as `{ "A1": "Total", "B1": "=SUM(B2:B9)", "C1": "42" }`: text, numbers, dates or formulas, as typed in a cell. "" empties a cell. Answers with what they show now.',
    inputSchema: {
      type: 'object',
      properties: { document: doc, sheet, cells: { type: 'object', additionalProperties: { type: 'string' } } },
      required: ['document', 'cells'],
    },
  },
  {
    name: 'format_cells',
    description: 'Excel: formats cells `range`. Give only what should change. Colors like "#1F4E79" ("" for automatic text or no fill).',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        sheet,
        range,
        bold: { type: 'boolean' },
        italic: { type: 'boolean' },
        underline: { type: 'boolean' },
        strikethrough: { type: 'boolean' },
        font: { type: 'string' },
        size: { type: 'number', description: 'Points.' },
        color: { type: 'string', description: 'The text color.' },
        fill: { type: 'string', description: 'The background color.' },
        align: { type: 'string', enum: ['left', 'center', 'right', 'general'] },
        vertical_align: { type: 'string', enum: ['top', 'center', 'bottom'] },
        wrap: { type: 'boolean', description: 'Wraps text onto more lines in the cell.' },
        number_format: { type: 'string', description: 'An Excel number format, like "0.00", "#,##0", "$#,##0.00", "0%", "yyyy-mm-dd" or "General".' },
        borders: { type: 'string', enum: ['all', 'outside', 'bottom', 'none'] },
      },
      required: ['document', 'range'],
    },
  },
  {
    name: 'clear_cells',
    description: 'Excel: empties cells `range`.',
    inputSchema: { type: 'object', properties: { document: doc, sheet, range }, required: ['document', 'range'] },
  },
  {
    name: 'size_cells',
    description: 'Excel: sets the width of `columns` (like "B" or "B:D") and the height of `rows` (like "3" or "3:10"), in pixels. "auto" as the size puts them back to the default.',
    inputSchema: {
      type: 'object',
      properties: { document: doc, sheet, columns: { type: 'string' }, width: { type: ['number', 'string'] }, rows: { type: 'string' }, height: { type: ['number', 'string'] } },
      required: ['document'],
    },
  },
  {
    name: 'insert_cells',
    description: 'Excel: inserts `count` (1 if left out) empty rows before row `row`, or columns before column `column` (like "C"). Formulas that refer past them move along.',
    inputSchema: { type: 'object', properties: { document: doc, sheet, row: { type: 'integer' }, column: { type: 'string' }, count: { type: 'integer' } }, required: ['document'] },
  },
  {
    name: 'delete_cells',
    description: 'Excel: deletes whole rows (like "3" or "3:5") or whole columns (like "C" or "C:E"); the ones after move up or left.',
    inputSchema: { type: 'object', properties: { document: doc, sheet, rows: { type: 'string' }, columns: { type: 'string' } }, required: ['document'] },
  },
  {
    name: 'merge_cells',
    description: 'Excel: merges cells `range` into one, or splits them again if they’re merged.',
    inputSchema: { type: 'object', properties: { document: doc, sheet, range }, required: ['document', 'range'] },
  },
  {
    name: 'add_chart',
    description: 'Excel: adds a chart of cells `range` (labels in the first row and column), of kind column, bar, line, area, pie, doughnut or scatter.',
    inputSchema: { type: 'object', properties: { document: doc, sheet, range, kind: { type: 'string', enum: ['column', 'bar', 'line', 'area', 'pie', 'doughnut', 'scatter'] } }, required: ['document', 'range'] },
  },
  {
    name: 'edit_sheet',
    description: 'Excel: `action` "add" adds a sheet at the end (named `name` if given); "rename" renames `sheet` to `name`; "move" moves `sheet` to position `to` (from 1); "delete" deletes `sheet`.',
    inputSchema: {
      type: 'object',
      properties: { document: doc, action: { type: 'string', enum: ['add', 'rename', 'move', 'delete'] }, sheet, name: { type: 'string' }, to: { type: 'integer' } },
      required: ['document', 'action'],
    },
  },
]

/** `B3` as [row, column], from 0. */
function cellRef(ref: string): [number, number] {
  const m = /^\$?([A-Za-z]{1,3})\$?(\d{1,7})$/.exec(ref.trim())
  if (!m) throw new ToolError(`“${ref}” isn’t a cell like B3.`)
  return [Number(m[2]) - 1, columnIndex(m[1])]
}

function columnIndex(name: string): number {
  if (!/^[A-Za-z]{1,3}$/.test(name.trim())) throw new ToolError(`“${name}” isn’t a column like C.`)
  let col = 0
  for (const ch of name.trim().toUpperCase()) col = col * 26 + (ch.charCodeAt(0) - 64)
  return col - 1
}

export function columnName(c: number) {
  let s = ''
  for (let n = c + 1; n > 0; n = Math.floor((n - 1) / 26)) s = String.fromCharCode(65 + ((n - 1) % 26)) + s
  return s
}

const refName = (r: number, c: number) => `${columnName(c)}${r + 1}`

/** A range like A1:F40 (or one cell), as rows and columns from 0. */
function rangeOf(text: string): { r0: number; c0: number; r1: number; c1: number } {
  const [a, b = a] = text.split(':')
  const [r0, c0] = cellRef(a)
  const [r1, c1] = cellRef(b)
  return { r0: Math.min(r0, r1), c0: Math.min(c0, c1), r1: Math.max(r0, r1), c1: Math.max(c0, c1) }
}

/** "3" or "3:5" as rows from 0; "C" or "C:E" as columns. */
function span(text: string, rows: boolean): [number, number] {
  const [a, b = a] = text.split(':').map((s) => s.trim())
  const one = (s: string) => {
    if (rows) {
      if (!/^\d{1,7}$/.test(s) || Number(s) < 1) throw new ToolError(`“${s}” isn’t a row number like 3.`)
      return Number(s) - 1
    }
    return columnIndex(s)
  }
  const [x, y] = [one(a), one(b)]
  return [Math.min(x, y), Math.max(x, y)]
}

function sheetIndex(e: SheetsEngine, name: string | undefined): number {
  if (name === undefined) return e.sheets.findIndex((s) => !s.hidden)
  const i = e.sheets.findIndex((s) => s.name.toLowerCase() === name.toLowerCase())
  if (i < 0) throw new ToolError(`No sheet “${name}”. Sheets: ${e.sheets.map((s) => s.name).join(', ')}.`)
  return i
}

const cellOf = (e: SheetsEngine, s: number, r: number, c: number) => JSON.parse(e.doc.cell(s, r, c)) as { input: string; display: string }

function cellText(e: SheetsEngine, s: number, r: number, c: number): string | null {
  const cell = cellOf(e, s, r, c)
  if (!cell.input) return null
  return cell.input.startsWith('=') ? `${refName(r, c)}: ${cell.input} → ${cell.display}` : `${refName(r, c)}: ${cell.display}`
}

export function readSheets(e: SheetsEngine, args: Args): string {
  const s = sheetIndex(e, str(args, 'sheet'))
  const g = e.geometryOf(s).info
  let { r0, c0, r1, c1 } = { r0: 0, c0: 0, r1: Math.min(g.content[0], 200) - 1, c1: Math.min(g.content[1], 52) - 1 }
  const want = str(args, 'range')
  if (want) ({ r0, c0, r1, c1 } = rangeOf(want))
  if ((r1 - r0 + 1) * (c1 - c0 + 1) > 20_000) throw new ToolError('That range is too big to read at once. Read up to 20,000 cells at a time, like A1:Z500.')
  const out = [`Sheets: ${e.sheets.map((x) => x.name).join(', ')}. Reading ${e.sheets[s].name}!${refName(r0, c0)}:${refName(Math.max(r0, r1), Math.max(c0, c1))}.`]
  for (let r = r0; r <= r1; r++) {
    const cells: string[] = []
    for (let c = c0; c <= c1; c++) {
      const t = cellText(e, s, r, c)
      if (t) cells.push(t)
    }
    if (cells.length) out.push(cells.join(' | '))
  }
  if (out.length === 1) out.push('(empty)')
  return out.join('\n')
}

/** Shows the agent where it worked, as people see a selection. */
function selectRange(e: SheetsEngine, s: number, r: { r0: number; c0: number; r1: number; c1: number }) {
  e.run({ cmd: 'select', sheet: s, anchor: [r.r0, r.c0], focus: [r.r1, r.c1] })
}

/** What cells show now, for the answer. */
const afterCells = (e: SheetsEngine, s: number, refs: [number, number][]) => () => {
  if (s >= e.sheets.length) return ''
  const shown = refs.slice(0, 40).map(([r, c]) => cellText(e, s, r, c) ?? `${refName(r, c)}: (empty)`)
  return `\nNow: ${shown.join(' | ')}${refs.length > 40 ? ` (and ${refs.length - 40} more)` : ''}`
}

const ALIGN: Record<string, string> = { left: 'left', center: 'center', right: 'right', general: '' }

export function sheetsTool(e: SheetsEngine, name: string, args: Args): Answer {
  if (name === 'edit_sheet') return editSheet(e, args)
  const s = sheetIndex(e, str(args, 'sheet'))
  switch (name) {
    case 'write_cells': {
      const cells = args.cells as Record<string, unknown> | undefined
      if (!cells || typeof cells !== 'object' || Array.isArray(cells)) throw new ToolError('Give cells, like { "A1": "Total" }.')
      const entries = Object.entries(cells).map(([ref, v]) => [cellRef(ref), String(v ?? '')] as const)
      if (!entries.length) return { text: 'Nothing to write.' }
      // Each cell is its own change (workbooks have no grouping), so undo takes back the last one.
      for (const [[row, col], text] of entries) e.run({ cmd: 'input', sheet: s, row, col, text })
      const rows = entries.map(([[r]]) => r)
      const cols = entries.map(([[, c]]) => c)
      selectRange(e, s, { r0: Math.min(...rows), c0: Math.min(...cols), r1: Math.max(...rows), c1: Math.max(...cols) })
      return { text: `Wrote ${plural(entries.length, 'cell')}.`, after: afterCells(e, s, entries.map(([rc]) => rc)) }
    }
    case 'format_cells': {
      const r = rangeOf(need(str(args, 'range'), 'the range'))
      const patch: Record<string, unknown> = {}
      for (const [k, to] of [
        ['bold', 'bold'],
        ['italic', 'italic'],
        ['underline', 'underline'],
        ['strikethrough', 'strike'],
        ['wrap', 'wrap'],
      ] as const) {
        const v = bool(args, k)
        if (v !== undefined) patch[to] = v
      }
      const font = str(args, 'font')
      if (font) patch.font = font
      const size = num(args, 'size')
      if (size !== undefined) {
        if (size < 1 || size > 409) throw new ToolError('Give a font size in points, from 1 to 409.')
        patch.size = size
      }
      for (const k of ['color', 'fill'] as const) {
        const v = str(args, k)
        if (v !== undefined) patch[k] = v.trim() === '' || v.trim().toLowerCase() === 'none' ? '' : hexColor(v, `The ${k}`).toUpperCase()
      }
      const align = str(args, 'align')
      if (align !== undefined) {
        if (!(align in ALIGN)) throw new ToolError('align is left, center, right or general.')
        patch.align = ALIGN[align]
      }
      const valign = str(args, 'vertical_align')
      if (valign !== undefined) {
        if (!['top', 'center', 'bottom'].includes(valign)) throw new ToolError('vertical_align is top, center or bottom.')
        patch.valign = valign
      }
      const nf = str(args, 'number_format')
      if (nf !== undefined) patch.numberFormat = nf
      const borders = str(args, 'borders')
      if (borders !== undefined) {
        if (!['all', 'outside', 'bottom', 'none'].includes(borders)) throw new ToolError('borders is all, outside, bottom or none.')
        patch.borders = borders
      }
      if (!Object.keys(patch).length) throw new ToolError('Give what to change: bold, italic, underline, strikethrough, font, size, color, fill, align, vertical_align, wrap, number_format or borders.')
      e.run({ cmd: 'format', sheet: s, range: r, patch })
      selectRange(e, s, r)
      const refs: [number, number][] = []
      for (let row = r.r0; row <= r.r1 && refs.length < 41; row++) for (let c = r.c0; c <= r.c1 && refs.length < 41; c++) if (cellOf(e, s, row, c).input) refs.push([row, c])
      return { text: `Formatted ${refName(r.r0, r.c0)}${r.r1 > r.r0 || r.c1 > r.c0 ? `:${refName(r.r1, r.c1)}` : ''}.`, after: refs.length && nf !== undefined ? afterCells(e, s, refs) : undefined }
    }
    case 'clear_cells': {
      const r = rangeOf(need(str(args, 'range'), 'the range'))
      e.run({ cmd: 'clear', sheet: s, range: r })
      selectRange(e, s, r)
      return { text: 'Cleared.' }
    }
    case 'size_cells': {
      const cols = str(args, 'columns')
      const rows = str(args, 'rows')
      if (!cols && !rows) throw new ToolError('Give columns (like "B:D") with width, or rows (like "3:10") with height.')
      const px = (k: string) => {
        const v = args[k]
        if (v === 'auto') return null
        const n = num(args, k)
        if (n === undefined) throw new ToolError(`Give ${k} in pixels, or "auto".`)
        if (n < 0 || n > 2000) throw new ToolError(`${k} is from 0 to 2000 pixels.`)
        return n
      }
      if (cols) {
        const [a, b] = span(cols, false)
        const w = px('width')
        for (let c = a; c <= b; c++) e.run({ cmd: 'columnWidth', sheet: s, col: c, px: w })
      }
      if (rows) {
        const [a, b] = span(rows, true)
        const h = px('height')
        for (let r = a; r <= b; r++) e.run({ cmd: 'rowHeight', sheet: s, row: r, px: h })
      }
      return { text: 'Done.' }
    }
    case 'insert_cells': {
      const count = int(args, 'count') ?? 1
      if (count < 1 || count > 1000) throw new ToolError('count is from 1 to 1000.')
      const row = int(args, 'row')
      const col = str(args, 'column')
      if ((row === undefined) === (col === undefined)) throw new ToolError('Give row (a number) or column (a letter), not both.')
      if (row !== undefined) {
        if (row < 1) throw new ToolError('Rows count from 1.')
        e.run({ cmd: 'shiftRows', sheet: s, at: row - 1, n: count })
        return { text: `Inserted ${plural(count, 'row')} at row ${row}.` }
      }
      const c = columnIndex(col!)
      e.run({ cmd: 'shiftCols', sheet: s, at: c, n: count })
      return { text: `Inserted ${plural(count, 'column')} at column ${columnName(c)}.` }
    }
    case 'delete_cells': {
      const rows = str(args, 'rows') ?? (int(args, 'rows') !== undefined ? String(int(args, 'rows')) : undefined)
      const cols = str(args, 'columns')
      if ((rows === undefined) === (cols === undefined)) throw new ToolError('Give rows (like "3:5") or columns (like "C:E"), not both.')
      if (rows !== undefined) {
        const [a, b] = span(rows, true)
        e.run({ cmd: 'shiftRows', sheet: s, at: a, n: -(b - a + 1) })
        return { text: `Deleted ${plural(b - a + 1, 'row')}.` }
      }
      const [a, b] = span(cols!, false)
      e.run({ cmd: 'shiftCols', sheet: s, at: a, n: -(b - a + 1) })
      return { text: `Deleted ${plural(b - a + 1, 'column')}.` }
    }
    case 'merge_cells': {
      const r = rangeOf(need(str(args, 'range'), 'the range'))
      e.run({ cmd: 'merge', sheet: s, range: r })
      selectRange(e, s, r)
      return { text: 'Done.' }
    }
    case 'add_chart': {
      const r = rangeOf(need(str(args, 'range'), 'the range'))
      const kind = str(args, 'kind') ?? 'column'
      if (!['column', 'bar', 'line', 'area', 'pie', 'doughnut', 'scatter'].includes(kind)) throw new ToolError('kind is column, bar, line, area, pie, doughnut or scatter.')
      e.run({ cmd: 'addChart', sheet: s, range: r, kind })
      return { text: `Added a ${kind} chart of ${refName(r.r0, r.c0)}:${refName(r.r1, r.c1)} on ${e.sheets[s].name}.` }
    }
  }
  throw new ToolError(`${name} isn’t for workbooks.`)
}

function editSheet(e: SheetsEngine, args: Args): Answer {
  const action = need(str(args, 'action'), 'the action')
  const name = str(args, 'name')?.trim()
  const list = () => `\nSheets now: ${e.sheets.map((x) => x.name).join(', ')}.`
  if (name !== undefined && (!name || name.length > 31 || /[\\/?*[\]:]/.test(name))) throw new ToolError('A sheet’s name is 1 to 31 characters, without \\ / ? * [ ] or :.')
  switch (action) {
    case 'add': {
      if (name && e.sheets.some((x) => x.name.toLowerCase() === name.toLowerCase())) throw new ToolError(`There’s a sheet “${name}” already.`)
      e.run({ cmd: 'addSheet' })
      if (name) e.run({ cmd: 'renameSheet', sheet: e.sheets.length - 1, name })
      return { text: 'Added.', after: list }
    }
    case 'rename': {
      const s = sheetIndex(e, need(str(args, 'sheet'), 'the sheet'))
      if (!name) throw new ToolError('Give the new name.')
      if (e.sheets.some((x, i) => i !== s && x.name.toLowerCase() === name.toLowerCase())) throw new ToolError(`There’s a sheet “${name}” already.`)
      e.run({ cmd: 'renameSheet', sheet: s, name })
      return { text: 'Renamed.', after: list }
    }
    case 'move': {
      const s = sheetIndex(e, need(str(args, 'sheet'), 'the sheet'))
      const to = need(int(args, 'to'), 'to')
      if (to < 1 || to > e.sheets.length) throw new ToolError(`to is from 1 to ${e.sheets.length}.`)
      e.run({ cmd: 'moveSheet', from: s, to: to - 1 })
      return { text: 'Moved.', after: list }
    }
    case 'delete': {
      const s = sheetIndex(e, need(str(args, 'sheet'), 'the sheet'))
      if (e.sheets.filter((x) => !x.hidden).length <= 1 && !e.sheets[s].hidden) throw new ToolError('That’s the only sheet showing; a workbook needs one.')
      e.run({ cmd: 'deleteSheet', sheet: s })
      return { text: 'Deleted.', after: list }
    }
  }
  throw new ToolError('action is add, rename, move or delete.')
}

/** Where someone's selection is, from their presence. */
export function describeSheetsPlace(at: Record<string, unknown>): string {
  const [ar, ac] = at.a as [number, number]
  const [fr, fc] = at.f as [number, number]
  const a = refName(Math.min(ar, fr), Math.min(ac, fc))
  const b = refName(Math.max(ar, fr), Math.max(ac, fc))
  return `sheet ${at.sheet}, ${a === b ? a : `${a}:${b}`}`
}
