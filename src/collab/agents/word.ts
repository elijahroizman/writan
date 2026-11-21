// What an agent can do in a Word document: read it by numbered paragraphs, see its pages, and
// do what people do in the editor: write, delete, style and format paragraphs and text, make
// lists and tables, and put in, resize and crop pictures. Edits answer with the paragraphs as
// they are once everyone has the change.

import type { WordEngine } from '../../engine/word/engine'
import { type Answer, type Args, bool, canvasPicture, clip, doc, grouped, hexColor, int, need, num, plural, str, type Tool, ToolError } from './common'
import { draw } from './picture'
import { pictureBytes, pictureType, scaledPicture } from './pictures'

export const WORD_TOOLS: Tool[] = [
  {
    name: 'insert_paragraphs',
    description: 'Word: puts new paragraphs in after paragraph `after` (0 puts them at the start). Each line of `text` is a paragraph. `style` sets their style, like "Heading 2" or "Normal". Answers with their numbers.',
    inputSchema: {
      type: 'object',
      properties: { document: doc, after: { type: 'integer' }, text: { type: 'string' }, style: { type: 'string' } },
      required: ['document', 'after', 'text'],
    },
  },
  {
    name: 'replace_paragraphs',
    description: 'Word: replaces paragraphs `from` to `to` (just `from` if `to` is left out) with `text`, one paragraph per line. Answers with the new paragraphs.',
    inputSchema: {
      type: 'object',
      properties: { document: doc, from: { type: 'integer' }, to: { type: 'integer' }, text: { type: 'string' } },
      required: ['document', 'from', 'text'],
    },
  },
  {
    name: 'delete_paragraphs',
    description: 'Word: deletes paragraphs `from` to `to` (just `from` if `to` is left out).',
    inputSchema: { type: 'object', properties: { document: doc, from: { type: 'integer' }, to: { type: 'integer' } }, required: ['document', 'from'] },
  },
  {
    name: 'set_style',
    description: 'Word: sets the style of paragraphs `from` to `to` (just `from` if `to` is left out), like "Heading 1", "Title" or "Normal".',
    inputSchema: {
      type: 'object',
      properties: { document: doc, from: { type: 'integer' }, to: { type: 'integer' }, style: { type: 'string' } },
      required: ['document', 'from', 'style'],
    },
  },
  {
    name: 'insert_table',
    description: 'Word: puts a table in after paragraph `after`, `rows` by `columns`, filled from `cells` (a list of rows, each a list of cells’ text) if given. Its cells’ paragraphs then read as “(in a table)”, and the paragraph tools work on them.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        after: { type: 'integer' },
        rows: { type: 'integer' },
        columns: { type: 'integer' },
        cells: { type: 'array', items: { type: 'array', items: { type: 'string' } } },
      },
      required: ['document', 'after'],
    },
  },
  {
    name: 'change_table',
    description:
      'Word: changes the table that paragraph `paragraph` is in, at that paragraph’s cell: `action` is add_row_above, add_row_below, delete_row, add_column_left, add_column_right, delete_column, borders_all, borders_outside, borders_none, fill (with `color`, the cell’s background) or delete_table.',
    inputSchema: {
      type: 'object',
      properties: { document: doc, paragraph: { type: 'integer' }, action: { type: 'string' }, color: { type: 'string' } },
      required: ['document', 'paragraph', 'action'],
    },
  },
  {
    name: 'format_picture',
    description:
      'Word: resizes or crops a picture in paragraph `paragraph` (the first one in it, or number `picture` from 1). width and height in pixels (96 to an inch); giving just one keeps its proportions. left, top, right and bottom are the share of the picture to cut from each side (0 to 1). `reset` puts it back to its own size, uncropped.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        paragraph: { type: 'integer' },
        picture: { type: 'integer' },
        width: { type: 'number' },
        height: { type: 'number' },
        left: { type: 'number' },
        top: { type: 'number' },
        right: { type: 'number' },
        bottom: { type: 'number' },
        reset: { type: 'boolean' },
      },
      required: ['document', 'paragraph'],
    },
  },
]

// ── Reading

interface Para {
  path: number[]
  text: string
}

const paragraphsOf = (e: WordEngine) => JSON.parse(e.doc.paragraphs()) as Para[]
const samePath = (a: number[], b: number[]) => a.length === b.length && a.every((x, i) => x === b[i])
/** Whether two paragraphs are in the same list of blocks (the body, or one table cell). */
const sameList = (a: number[], b: number[]) => a.length === b.length && a.slice(0, -1).every((x, i) => x === b[i])
/** Whether paragraph `b` comes straight after `a` in the same list, with no other block between. */
const nextTo = (a: number[], b: number[]) => sameList(a, b) && b[b.length - 1] === a[a.length - 1] + 1

const shown = (t: string) => t.replace(/￼/g, '[picture]')

function lines(e: WordEngine, from: number, to: number): string[] {
  const paras = paragraphsOf(e)
  const headings = JSON.parse(e.doc.outline()) as { text: string; depth: number; path: number[] }[]
  const out: string[] = []
  for (let i = Math.max(1, from); i <= Math.min(paras.length, to); i++) {
    const p = paras[i - 1]
    const h = headings.find((x) => samePath(x.path, p.path))
    // A table cell's paragraph says which cell: (in a table, row 2, column 3).
    const where = p.path.length >= 4 ? ` (in a table, row ${p.path[p.path.length - 3] + 1}, column ${p.path[p.path.length - 2] + 1})` : p.path.length > 1 ? ' (in a table)' : ''
    // The outline counts levels from 0.
    out.push(`[${i}]${h ? ` (Heading ${h.depth + 1})` : ''}${where} ${shown(p.text)}`)
  }
  return out
}

export function readWord(e: WordEngine, args: Args): string {
  const paras = paragraphsOf(e)
  const from = Math.max(1, int(args, 'from') ?? 1)
  const to = Math.min(paras.length, int(args, 'to') ?? paras.length)
  return [`${plural(paras.length, 'paragraph')} on ${plural(e.state.page.total, 'page')}. Numbers are what the editing tools take. view_document shows a page as a picture.`, ...lines(e, from, to)].join('\n')
}

function para(paras: Para[], n: number | undefined, what = 'paragraph'): Para {
  if (n === undefined || n < 1 || n > paras.length) throw new ToolError(`There’s no ${what} ${n ?? ''}; the document has ${paras.length} paragraphs (1 to ${paras.length}).`)
  return paras[n - 1]
}

/** Selects from (path, offset) to (path, offset), offsets in UTF-16 units of the text. */
function select(e: WordEngine, a: [number[], number], f: [number[], number] = a) {
  e.run({ cmd: 'select', anchor: a, focus: f })
}

function styleId(e: WordEngine, name: string): string {
  const options = JSON.parse(e.doc.styleOptions()) as { styleId?: string; style_id?: string; name: string }[]
  const want = name.trim().toLowerCase().replace(/\s+/g, ' ')
  const hit = options.find((o) => o.name.toLowerCase() === want || (o.styleId ?? o.style_id ?? '').toLowerCase() === want.replace(/ /g, ''))
  if (!hit) throw new ToolError(`No style “${name}”. Styles here: ${options.map((o) => o.name).join(', ')}.`)
  return (hit.styleId ?? hit.style_id)!
}

/** The answer after an edit: paragraphs `from` to `to` as they are now. */
const afterParas = (e: WordEngine, from: number, to: number) => () => {
  const now = lines(e, from, to)
  return now.length ? `\nNow:\n${now.join('\n')}` : ''
}

/** How the selected text looks, from the editor's state. */
function looks(e: WordEngine): string {
  const s = e.state
  const bits = [
    s.style && `style ${s.style}`,
    s.font,
    // The editor's state gives sizes in half-points, as Word stores them.
    s.size ? `${s.size / 2}pt` : null,
    s.bold && 'bold',
    s.italic && 'italic',
    s.underline && 'underlined',
    s.strike && 'struck through',
    s.color && `color ${s.color}`,
    s.highlight && `highlighted ${s.highlight}`,
    s.align && s.align !== 'left' ? `aligned ${s.align}` : null,
    s.list && `${s.list} list`,
    s.lineSpacing && s.lineSpacing !== 1 ? `line spacing ${s.lineSpacing}` : null,
  ]
  return bits.filter(Boolean).join(', ')
}

/**
 * Sets a toggle (bold…) for the whole selection. The state says on only when all of it is, so
 * a mixed selection reads as off: turning it off turns it all on first, then all off.
 */
function toggleTo(e: WordEngine, what: 'bold' | 'italic' | 'underline' | 'strike', on: boolean) {
  if (!e.state[what]) e.run({ cmd: 'toggle', what })
  if (!on) e.run({ cmd: 'toggle', what })
}

const HIGHLIGHTS = ['yellow', 'green', 'cyan', 'magenta', 'blue', 'red', 'darkBlue', 'darkCyan', 'darkGreen', 'darkMagenta', 'darkRed', 'darkYellow', 'darkGray', 'lightGray', 'black', 'none']

/** Applies text and paragraph formatting to the selection. */
function applyFormat(e: WordEngine, args: Args) {
  const font = str(args, 'font')
  if (font) e.run({ cmd: 'font', family: font })
  const size = num(args, 'size')
  if (size !== undefined) e.run({ cmd: 'size', halfPoints: Math.round(size * 2) })
  for (const [k, what] of [
    ['bold', 'bold'],
    ['italic', 'italic'],
    ['underline', 'underline'],
    ['strikethrough', 'strike'],
  ] as const) {
    const v = bool(args, k)
    if (v !== undefined) toggleTo(e, what, v)
  }
  const color = str(args, 'color')
  if (color) e.run({ cmd: 'color', hex: hexColor(color, 'The color') })
  const highlight = str(args, 'highlight')
  if (highlight) e.run({ cmd: 'highlight', name: HIGHLIGHTS.find((h) => h.toLowerCase() === highlight.toLowerCase()) ?? 'yellow' })
  const align = str(args, 'align')
  if (align) e.run({ cmd: 'align', to: align })
  const list = str(args, 'list')
  if (list) {
    const want = list === 'none' ? null : list
    if (e.state.list !== want) e.run({ cmd: 'list', kind: want ?? 'none' })
  }
  const spacing = num(args, 'line_spacing')
  if (spacing !== undefined) e.run({ cmd: 'lineSpacing', value: spacing })
}

function checkFormat(args: Args) {
  const keys = ['font', 'size', 'bold', 'italic', 'underline', 'strikethrough', 'color', 'highlight', 'align', 'list', 'line_spacing']
  if (!keys.some((k) => args[k] !== undefined)) throw new ToolError(`Give what to change: ${keys.join(', ')}.`)
  const size = num(args, 'size')
  if (size !== undefined && (size < 1 || size > 1638)) throw new ToolError('Give a font size in points, from 1 to 1638.')
  const align = str(args, 'align')
  if (align && !['left', 'center', 'right', 'justify'].includes(align)) throw new ToolError('align is left, center, right or justify.')
  const list = str(args, 'list')
  if (list && !['bullet', 'numbered', 'none'].includes(list)) throw new ToolError('list is bullet, numbered or none.')
  const highlight = str(args, 'highlight')
  if (highlight && !HIGHLIGHTS.some((h) => h.toLowerCase() === highlight.toLowerCase())) throw new ToolError(`highlight is one of ${HIGHLIGHTS.join(', ')}.`)
}

function formatWord(e: WordEngine, args: Args): Answer {
  checkFormat(args)
  const paras = paragraphsOf(e)
  const fromN = need(int(args, 'from'), 'from (the first paragraph to format)')
  const toN = int(args, 'to') ?? fromN
  const from = para(paras, fromN)
  const to = para(paras, toN)
  if (toN < fromN) throw new ToolError('to comes before from.')
  const find = str(args, 'find')
  if (!find) {
    select(e, [from.path, 0], [to.path, to.text.length])
    grouped(e, () => applyFormat(e, args))
    return { text: `Formatted ${fromN === toN ? `paragraph ${fromN}` : `paragraphs ${fromN}–${toN}`}: ${looks(e)}.`, after: afterParas(e, fromN, toN) }
  }
  // Only the text found, each time it's there: paragraph formatting (align, lists) goes to its paragraph.
  const q = find.toLowerCase()
  let count = 0
  grouped(e, () => {
    for (let n = fromN; n <= toN; n++) {
      const p = paras[n - 1]
      const low = p.text.toLowerCase()
      for (let at = low.indexOf(q); at >= 0; at = low.indexOf(q, at + q.length)) {
        select(e, [p.path, at], [p.path, at + q.length])
        applyFormat(e, args)
        count++
      }
    }
  })
  if (!count) throw new ToolError(`“${find}” isn’t in ${fromN === toN ? `paragraph ${fromN}` : `paragraphs ${fromN}–${toN}`}.`)
  return { text: `Formatted ${plural(count, 'match', 'matches')}: ${looks(e)}.`, after: afterParas(e, fromN, toN) }
}

/** Fills a table's cells from a grid, from cell (r0, c0). `table` is the table's path. */
function fillTable(e: WordEngine, table: number[], cells: unknown[][], r0 = 0, c0 = 0) {
  cells.forEach((row, ri) =>
    row.forEach((v, ci) => {
      const cell = [...table, r0 + ri, c0 + ci]
      const inCell = paragraphsOf(e).filter((p) => p.path.length === cell.length + 1 && cell.every((x, i) => p.path[i] === x))
      if (!inCell.length) return
      const last = inCell[inCell.length - 1]
      select(e, [inCell[0].path, 0], [last.path, last.text.length])
      e.run({ cmd: 'paste', text: String(v ?? ''), json: null })
    }),
  )
}

const TABLE_ACTIONS: Record<string, string> = {
  add_row_above: 'insertRowAbove',
  add_row_below: 'insertRowBelow',
  delete_row: 'deleteRow',
  add_column_left: 'insertColumnLeft',
  add_column_right: 'insertColumnRight',
  delete_column: 'deleteColumn',
  borders_all: 'bordersAll',
  borders_outside: 'bordersOutside',
  borders_none: 'bordersNone',
  fill: 'fill',
  delete_table: 'delete',
}

/** The nth picture in a paragraph, selected, as the editor's state sees it. */
function selectPicture(e: WordEngine, n: number, which: number) {
  const p = para(paragraphsOf(e), n)
  let at = -1
  for (let k = 0; k < which; k++) {
    at = p.text.indexOf('￼', at + 1)
    if (at < 0) break
  }
  if (at < 0) throw new ToolError(which === 1 ? `Paragraph ${n} has no picture.` : `Paragraph ${n} has fewer than ${which} pictures.`)
  select(e, [p.path, at], [p.path, at + 1])
  const img = e.state.image
  if (!img) throw new ToolError(`That picture in paragraph ${n} can’t be changed here.`)
  return img
}

const EMU = 9525

export async function wordTool(e: WordEngine, name: string, args: Args, local: boolean): Promise<Answer> {
  const paras = paragraphsOf(e)
  switch (name) {
    case 'insert_paragraphs': {
      const after = need(int(args, 'after'), 'after')
      const text = need(str(args, 'text'), 'the text').replace(/\r\n?/g, '\n')
      const count = text.split('\n').length
      const style = str(args, 'style')
      const id = style ? styleId(e, style) : null
      if (after < 0 || after > paras.length) throw new ToolError(`after is a paragraph number from 0 to ${paras.length}.`)
      grouped(e, () => {
        if (after === 0) {
          const first = para(paras, 1)
          select(e, [first.path, 0])
          e.run({ cmd: 'paste', text: `${text}\n`, json: null })
        } else {
          const p = para(paras, after)
          select(e, [p.path, p.text.length])
          e.run({ cmd: 'enter' })
          e.run({ cmd: 'paste', text, json: null })
        }
        if (id) {
          const now = paragraphsOf(e)
          const first = now[after]
          const last = now[after + count - 1]
          if (first && last) {
            select(e, [first.path, 0], [last.path, last.text.length])
            e.run({ cmd: 'style', id })
          }
        }
      })
      return { text: `Put in ${plural(count, 'paragraph')} as ${after + 1}${count > 1 ? `–${after + count}` : ''}.`, after: afterParas(e, after + 1, after + count) }
    }
    case 'replace_paragraphs': {
      const fromN = need(int(args, 'from'), 'from')
      const from = para(paras, fromN)
      const toN = int(args, 'to') ?? fromN
      const to = para(paras, toN)
      const text = need(str(args, 'text'), 'the text').replace(/\r\n?/g, '\n')
      select(e, [from.path, 0], [to.path, to.text.length])
      e.run({ cmd: 'paste', text, json: null })
      return { text: 'Replaced.', after: afterParas(e, fromN, fromN + text.split('\n').length - 1) }
    }
    case 'delete_paragraphs': {
      const fromN = need(int(args, 'from'), 'from')
      const toN = int(args, 'to') ?? fromN
      const from = para(paras, fromN)
      const to = para(paras, toN)
      // Only within one list of paragraphs (the body, or one table cell): a deletion that ran
      // into a table would take the table with it.
      if (toN < fromN || paras.slice(fromN - 1, toN).some((p) => !sameList(p.path, from.path))) {
        throw new ToolError('Those paragraphs aren’t all in one place (a table is in between, or some are in a table cell). Delete them in parts.')
      }
      // Nothing but paragraphs in between: a content control or other block the list skips
      // would go too.
      for (let i = fromN; i < toN; i++) if (!nextTo(paras[i - 1].path, paras[i].path)) throw new ToolError('Something that isn’t a paragraph is among them. Delete the paragraphs around it separately.')
      // The paragraphs go whole, as one change; the ones around them stay exactly as they were.
      const container = from.path.slice(0, -1)
      const result = e.run<string>({ cmd: 'deleteBlocks', container, from: from.path[from.path.length - 1], to: to.path[to.path.length - 1] })
      if (result === 'section') throw new ToolError('One of those paragraphs ends a section (it holds that section’s page setup), so it can’t be deleted. Delete the paragraphs around it.')
      if (result === 'missing') throw new ToolError('Those paragraphs aren’t there any more. Read the document again.')
      if (result === 'all') {
        // They're all there is in that list (a table cell, or the whole body): their text goes
        // and one empty paragraph stays.
        select(e, [from.path, 0], [to.path, to.text.length])
        e.run({ cmd: 'delete', word: false })
      }
      return { text: `Deleted ${plural(toN - fromN + 1, 'paragraph')}.`, after: afterParas(e, fromN - 1, fromN) }
    }
    case 'set_style': {
      const fromN = need(int(args, 'from'), 'from')
      const toN = int(args, 'to') ?? fromN
      const from = para(paras, fromN)
      const to = para(paras, toN)
      const id = styleId(e, need(str(args, 'style'), 'the style'))
      select(e, [from.path, 0], [to.path, to.text.length])
      e.run({ cmd: 'style', id })
      return { text: 'Done.', after: afterParas(e, fromN, toN) }
    }
    case 'format_text':
      return formatWord(e, args)
    case 'insert_table': {
      const after = need(int(args, 'after'), 'after')
      const cells = Array.isArray(args.cells) ? (args.cells as unknown[]).map((r) => (Array.isArray(r) ? r : [r])) : null
      const rows = int(args, 'rows') ?? cells?.length ?? 2
      const cols = int(args, 'columns') ?? Math.max(1, ...(cells ?? [[0, 0]]).map((r) => r.length))
      if (rows < 1 || rows > 200 || cols < 1 || cols > 60) throw new ToolError('A table has 1 to 200 rows and 1 to 60 columns.')
      if (after < 0 || after > paras.length) throw new ToolError(`after is a paragraph number from 0 to ${paras.length}.`)
      let first = 0
      grouped(e, () => {
        if (after === 0) select(e, [para(paras, 1).path, 0])
        else {
          const p = para(paras, after)
          select(e, [p.path, p.text.length])
          e.run({ cmd: 'enter' })
        }
        e.run({ cmd: 'insertTable', rows, cols })
        const now = paragraphsOf(e)
        first = now.findIndex((p, i) => i >= after && p.path.length > 1)
        if (first < 0) throw new ToolError('The table didn’t go in. Read the document again.')
        if (cells) fillTable(e, now[first].path.slice(0, -3), cells)
      })
      return { text: `Put in a ${rows} × ${cols} table.`, after: afterParas(e, first + 1, first + rows * cols) }
    }
    case 'change_table': {
      const n = need(int(args, 'paragraph'), 'paragraph')
      const p = para(paras, n)
      if (p.path.length < 4) throw new ToolError(`Paragraph ${n} isn’t in a table.`)
      const action = need(str(args, 'action'), 'the action')
      const op = TABLE_ACTIONS[action]
      if (!op) throw new ToolError(`action is one of ${Object.keys(TABLE_ACTIONS).join(', ')}.`)
      const color = str(args, 'color')
      if (op === 'fill' && !color) throw new ToolError('Give the fill color, like "#DDEBF7", or "none".')
      select(e, [p.path, 0])
      e.run({ cmd: 'table', op, color: op === 'fill' ? (color!.toLowerCase() === 'none' ? null : hexColor(color!, 'The color')) : null })
      return { text: 'Done.', after: () => `\n${plural(paragraphsOf(e).length, 'paragraph')} now. Read the document again for their numbers.` }
    }
    case 'format_picture': {
      const n = need(int(args, 'paragraph'), 'paragraph')
      const which = int(args, 'picture') ?? 1
      const img = selectPicture(e, n, which)
      if (args.reset === true) {
        const c = img.crop
        grouped(e, () => {
          // Uncropped first, at the scale it shows now; then its own size where Writan knows it.
          if (c.left || c.top || c.right || c.bottom) {
            const w = img.widthEmu / Math.max(0.01, 1 - (c.left + c.right) / 100)
            const h = img.heightEmu / Math.max(0.01, 1 - (c.top + c.bottom) / 100)
            e.run({ cmd: 'cropImage', left: 0, top: 0, right: 0, bottom: 0, widthEmu: Math.round(w), heightEmu: Math.round(h) })
          }
          e.run({ cmd: 'resetImageSize' })
        })
        return { text: 'Uncropped, and back to its own size.', after: () => pictureNow(e, n, which) }
      }
      const w0 = num(args, 'width')
      const h0 = num(args, 'height')
      const sides = ['left', 'top', 'right', 'bottom'].map((k) => num(args, k))
      if (w0 === undefined && h0 === undefined && sides.every((v) => v === undefined)) throw new ToolError('Give width, height, left, top, right, bottom or reset.')
      let w = img.widthEmu
      let h = img.heightEmu
      if (w0 !== undefined) w = w0 * EMU
      if (h0 !== undefined) h = h0 * EMU
      if (w0 !== undefined && h0 === undefined) h = (img.heightEmu * w) / img.widthEmu
      if (h0 !== undefined && w0 === undefined) w = (img.widthEmu * h) / img.heightEmu
      if (sides.every((v) => v === undefined)) {
        if (!img.canResize) throw new ToolError('That picture can’t be resized here.')
        e.run({ cmd: 'imageSize', widthEmu: Math.round(w), heightEmu: Math.round(h) })
      } else {
        if (!img.canCrop) throw new ToolError('That picture can’t be cropped here.')
        const was = [img.crop.left, img.crop.top, img.crop.right, img.crop.bottom].map((v) => v / 100)
        const crop = sides.map((v, i) => {
          if (v !== undefined && (v < 0 || v >= 1)) throw new ToolError('left, top, right and bottom are shares of the picture, from 0 to under 1, like 0.1.')
          return v ?? was[i]
        })
        if (crop[0] + crop[2] > 0.98 || crop[1] + crop[3] > 0.98) throw new ToolError('That would cut away the whole picture.')
        // Unless a size is given, the picture keeps its scale and shows a different part of itself.
        if (w0 === undefined && h0 === undefined) {
          const full = [img.widthEmu / Math.max(0.01, 1 - was[0] - was[2]), img.heightEmu / Math.max(0.01, 1 - was[1] - was[3])]
          w = full[0] * (1 - crop[0] - crop[2])
          h = full[1] * (1 - crop[1] - crop[3])
        }
        e.run({ cmd: 'cropImage', left: crop[0] * 100, top: crop[1] * 100, right: crop[2] * 100, bottom: crop[3] * 100, widthEmu: Math.round(w), heightEmu: Math.round(h) })
      }
      return { text: 'Done.', after: () => pictureNow(e, n, which) }
    }
    case 'add_picture': {
      const after = need(int(args, 'after'), 'after (the paragraph to put it after; 0 for the start)')
      if (after < 0 || after > paras.length) throw new ToolError(`after is a paragraph number from 0 to ${paras.length}.`)
      const got = await pictureBytes(args, local)
      const type = pictureType(got.bytes)
      if (!type) throw new ToolError('That isn’t a picture Writan can put in a document. Give a PNG, JPEG, GIF, WebP or SVG.')
      const pic = await scaledPicture(got.bytes, type, 2400, 2400)
      const now = paragraphsOf(e)
      grouped(e, () => {
        if (after === 0) {
          select(e, [para(now, 1).path, 0])
          e.run({ cmd: 'enter' })
          select(e, [paragraphsOf(e)[0].path, 0])
        } else {
          const p = para(now, after)
          select(e, [p.path, p.text.length])
          e.run({ cmd: 'enter' })
        }
        e.doc.insertImage(pic.bytes, pic.type, pic.width, pic.height)
        e.refresh()
      })
      const n = after + 1
      const w = num(args, 'width')
      const h = num(args, 'height')
      if (w !== undefined || h !== undefined) {
        const img = selectPicture(e, n, 1)
        const ww = w !== undefined ? w * EMU : (img.widthEmu * (h! * EMU)) / img.heightEmu
        const hh = h !== undefined ? h * EMU : (img.heightEmu * (w! * EMU)) / img.widthEmu
        e.run({ cmd: 'imageSize', widthEmu: Math.round(ww), heightEmu: Math.round(hh) })
      }
      return { text: `Put the picture in as paragraph ${n}.`, after: () => pictureNow(e, n, 1) }
    }
  }
  throw new ToolError(`${name} isn’t for Word documents.`)
}

function pictureNow(e: WordEngine, n: number, which: number): string {
  try {
    const img = selectPicture(e, n, which)
    const c = img.crop
    const cropped = [c.left && `${Math.round(c.left)}% left`, c.top && `${Math.round(c.top)}% top`, c.right && `${Math.round(c.right)}% right`, c.bottom && `${Math.round(c.bottom)}% bottom`].filter(Boolean)
    return `\nThe picture in paragraph ${n} is now ${Math.round(img.widthEmu / EMU)} × ${Math.round(img.heightEmu / EMU)} pixels${cropped.length ? `, cropped ${cropped.join(', ')}` : ''}.`
  } catch {
    return ''
  }
}

// ── Seeing

export async function viewPage(e: WordEngine, args: Args): Promise<Answer> {
  const total = e.state.page.total
  const n = int(args, 'page') ?? 1
  if (n < 1 || n > total) throw new ToolError(`There’s no page ${n}; the document has ${plural(total, 'page')}.`)
  const p = e.paint(n - 1, true)
  if (!p) throw new ToolError('That page couldn’t be drawn.')
  const want = Math.min(2000, Math.max(200, num(args, 'width') ?? 1000))
  const canvas = await draw({ width: p.width, height: p.height, items: p.items, styles: e.styles, media: e.media }, want / p.width)
  return { text: `Page ${n} of ${total}, ${canvas.width} × ${canvas.height}.`, pictures: [await canvasPicture(canvas)] }
}

// ── What changed, for wait_for_changes

export const snapshotWord = (e: WordEngine) => paragraphsOf(e).map((p) => p.text)

/** The paragraphs that differ, from the first change to the last. */
export function wordChanges(a: string[], b: string[]): string {
  let start = 0
  while (start < a.length && start < b.length && a[start] === b[start]) start++
  let endA = a.length - 1
  let endB = b.length - 1
  while (endA >= start && endB >= start && a[endA] === b[endB]) {
    endA--
    endB--
  }
  if (start > endB && start > endA) return 'The text is the same as before (the change was to formatting, styles or something else).'
  const out: string[] = []
  const removed = endA - start + 1
  const added = endB - start + 1
  if (added > 0) {
    const shownLines = b.slice(start, endB + 1).map((t, k) => `[${start + k + 1}] ${clip(shown(t), 300)}`)
    out.push(`${added === 1 ? `Paragraph ${start + 1} is` : `Paragraphs ${start + 1}–${endB + 1} are`} now:`, ...shownLines.slice(0, 12))
    if (shownLines.length > 12) out.push(`(and ${shownLines.length - 12} more)`)
  }
  if (removed > added) out.push(`${plural(removed - added, 'paragraph')} fewer than before.`)
  return out.join('\n')
}
