// What an agent can do in a Word document: read it by numbered paragraphs, see its pages, and
// do what people do in the editor: write, delete, style and format paragraphs and text, make
// lists and tables, and put in, resize and crop pictures. Edits answer with the paragraphs as
// they are once everyone has the change.

import type { WordEngine } from '../../engine/word/engine'
import type { Said } from '../session'
import { ID, IDS_IN, idsOf } from './ids'
import { type Answer, type Args, bool, canvasPicture, clip, doc, found, foundAnswer, grouped, hexColor, int, need, num, plural, str, type Tool, ToolError } from './common'
import { draw } from './picture'
import { pictureBytes, pictureType, scaledPicture } from './pictures'

/** A paragraph, by number or id. */
export const paraRef = (what = 'A paragraph') => ({ type: ['integer', 'string'], description: `${what}: its number, or its id from read_document like "#k3f9x2", which stays the same while others add or delete paragraphs.` })

export const WORD_TOOLS: Tool[] = [
  {
    name: 'insert_paragraphs',
    description:
      'Word: puts new paragraphs in after paragraph `after` (0 puts them at the start), or after the last one with `at_end`, which stays right even if others add paragraphs meanwhile. Each line of `text` is a paragraph; or give `paragraphs`, each with its own style. `style` sets the style of those without one, like "Heading 2" or "Normal"; otherwise they take the style Word gives a new paragraph after the one before (body text after a heading or title). Answers with their numbers.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        after: { ...paraRef('The paragraph to put them after'), description: 'The paragraph to put them after: its number (0 for the start) or its id, like "#k3f9x2", which stays right while others add or delete paragraphs.' },
        at_end: { type: 'boolean', description: 'After the last paragraph, in place of after.' },
        text: { type: 'string' },
        paragraphs: {
          type: 'array',
          items: { type: 'object', properties: { text: { type: 'string' }, style: { type: 'string' } }, required: ['text'] },
          description: 'In place of text: each paragraph with its own style, like [{"text": "Results", "style": "Heading 1"}, {"text": "…"}].',
        },
        style: { type: 'string' },
      },
      required: ['document'],
    },
  },
  {
    name: 'replace_paragraphs',
    description:
      'Word: replaces paragraphs `from` to `to` (just `from` if `to` is left out) with `text`, one paragraph per line, or with `paragraphs`, each with its own style. The first keeps the style of paragraph `from`; the rest take the style Word gives a new paragraph after it (body text after a heading or title), unless they name one. Someone else’s paragraphs need `confirm`. Given by id, a paragraph someone else has changed since is refused, so you don’t write over what you haven’t read. Answers with the new paragraphs.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        from: paraRef('The first paragraph'),
        to: paraRef('The last paragraph'),
        text: { type: 'string' },
        paragraphs: {
          type: 'array',
          items: { type: 'object', properties: { text: { type: 'string' }, style: { type: 'string' } }, required: ['text'] },
          description: 'In place of text: each paragraph with its own style, like [{"text": "Results", "style": "Heading 1"}, {"text": "…"}].',
        },
        confirm: { type: 'boolean', description: 'Replace paragraphs someone else wrote, once they or your person agreed.' },
      },
      required: ['document', 'from'],
    },
  },
  {
    name: 'delete_paragraphs',
    description: 'Word: deletes paragraphs `from` to `to` (just `from` if `to` is left out). Someone else’s paragraphs need `confirm`. Given by id, a paragraph someone else has changed since is refused.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        from: paraRef('The first paragraph'),
        to: paraRef('The last paragraph'),
        confirm: { type: 'boolean', description: 'Delete paragraphs someone else wrote, once they or your person agreed.' },
      },
      required: ['document', 'from'],
    },
  },
  {
    name: 'set_style',
    description: 'Word: sets the style of paragraphs `from` to `to` (just `from` if `to` is left out), like "Heading 1", "Title" or "Normal".',
    inputSchema: {
      type: 'object',
      properties: { document: doc, from: paraRef('The first paragraph'), to: paraRef('The last paragraph'), style: { type: 'string' } },
      required: ['document', 'from', 'style'],
    },
  },
  {
    name: 'insert_table',
    description: 'Word: puts a table in after paragraph `after` (or after the last with `at_end`), `rows` by `columns`, filled from `cells` (a list of rows, each a list of cells’ text) if given. Its cells’ paragraphs then read as “(in a table)”, and the paragraph tools work on them.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        after: paraRef('The paragraph to put it after (0 for the start)'),
        at_end: { type: 'boolean' },
        rows: { type: 'integer' },
        columns: { type: 'integer' },
        cells: { type: 'array', items: { type: 'array', items: { type: 'string' } } },
      },
      required: ['document'],
    },
  },
  {
    name: 'change_table',
    description:
      'Word: changes the table that paragraph `paragraph` is in, at that paragraph’s cell: `action` is add_row_above, add_row_below, delete_row, add_column_left, add_column_right, delete_column, borders_all, borders_outside, borders_none, fill (with `color`, the cell’s background) or delete_table.',
    inputSchema: {
      type: 'object',
      properties: { document: doc, paragraph: paraRef('A paragraph in the table'), action: { type: 'string' }, color: { type: 'string' } },
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
        paragraph: paraRef('The paragraph with the picture'),
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
  /** Its style's name; none for Normal. */
  style?: string | null
  /** The style id Enter gives the paragraph after it, when that's another. */
  next?: string | null
}

const paragraphsOf = (e: WordEngine) => JSON.parse(e.doc.paragraphs()) as Para[]
const samePath = (a: number[], b: number[]) => a.length === b.length && a.every((x, i) => x === b[i])
/** Whether two paragraphs are in the same list of blocks (the body, or one table cell). */
const sameList = (a: number[], b: number[]) => a.length === b.length && a.slice(0, -1).every((x, i) => x === b[i])
/** Whether paragraph `b` comes straight after `a` in the same list, with no other block between. */
const nextTo = (a: number[], b: number[]) => sameList(a, b) && b[b.length - 1] === a[a.length - 1] + 1

const shown = (t: string) => t.replace(/￼/g, '[picture]')

/** `[7 #k3f9x2]`, or `[7]` for an empty paragraph. */
const label = (n: number, id: string | null | undefined) => (id ? `[${n} ${id}]` : `[${n}]`)

function lines(e: WordEngine, from: number, to: number, history?: History, notes?: Map<number, string[]>): string[] {
  const paras = paragraphsOf(e)
  const ids = idsOf(paras.map((p) => p.text))
  const headings = JSON.parse(e.doc.outline()) as { text: string; depth: number; path: number[] }[]
  const out: string[] = []
  for (let i = Math.max(1, from); i <= Math.min(paras.length, to); i++) {
    const p = paras[i - 1]
    // Its style, or for one that's a heading by its own outline level, which heading. The
    // outline counts levels from 0.
    const h = p.style ? null : headings.find((x) => samePath(x.path, p.path))
    const style = p.style ?? (h ? `Heading ${h.depth + 1}` : null)
    // A table cell's paragraph says which cell: (in a table, row 2, column 3).
    const where = p.path.length >= 4 ? ` (in a table, row ${p.path[p.path.length - 3] + 1}, column ${p.path[p.path.length - 2] + 1})` : p.path.length > 1 ? ' (in a table)' : ''
    const by = p.text.trim() ? history?.label(i) : null
    history?.shown(ids[i - 1])
    out.push(`${label(i, ids[i - 1])}${style ? ` (${style})` : ''}${where}${by ? ` {by ${by}}` : ''} ${shown(p.text)}`)
    for (const n of notes?.get(i) ?? []) out.push(`    ↳ ${n}`)
  }
  return out
}

/** What read_document's Word answer needs beside the document. */
export interface ReadContext {
  history?: History
  /** The chat this agent has seen and said, for the messages that name a paragraph. */
  chat?: Said[]
  /** The relay's number of the last change this copy has. */
  version?: number
}

export function readWord(e: WordEngine, args: Args, ctx: ReadContext = {}): string {
  const paras = paragraphsOf(e)
  const { history, version } = ctx
  const notes: string[] = []
  const at = (k: string) => paraNumber(paras, args, k, history, notes)
  const since = int(args, 'since')
  const v = version !== undefined ? `, version ${version}` : ''
  const head = `${plural(paras.length, 'paragraph')} on ${plural(e.state.page.total, 'page')}${v}. Each starts [number #id]: the tools take either. An id stays the same while others add or delete paragraphs, so use ids in messages too; a message that names one shows under its paragraph (↳). The style is in brackets, like (Title) or (Heading 1); none is Normal. {by …} says who last changed a paragraph’s text; one without was there before anyone joined. view_document shows a page as a picture.`
  const said = ctx.chat ? notesOn(paras, ctx.chat, history) : undefined
  if (since !== undefined) {
    if (!history) throw new ToolError('This document can’t say what changed since.')
    const changed = history.changedSince(since)
    const out = [`${plural(paras.length, 'paragraph')}${v}. Since version ${since}:`]
    for (const r of history.removedSince(since)) out.push(`- ${r.by ?? 'Someone'} deleted ${plural(r.count, 'paragraph')}.`)
    if (!changed.length && out.length === 1) return `Nothing changed since version ${since}${v ? `; this is${v.slice(1)}` : ''}.`
    if (changed.length) out.push(`${changed.length === 1 ? 'This paragraph is' : 'These paragraphs are'} new or changed (the rest are as they were):`)
    for (const n of changed) out.push(...lines(e, n, n, history, said))
    return [...notes, ...out].join('\n')
  }
  const from = Math.max(1, at('from') ?? 1)
  const to = Math.min(paras.length, at('to') ?? paras.length)
  const whole = from === 1 && to === paras.length
  const who = whole && history ? history.summary() : ''
  return [...notes, head, ...(who ? [who] : []), ...lines(e, from, to, history, said)].join('\n')
}

/** For each paragraph, by number, the messages that name its id, latest last; at most three each. */
function notesOn(paras: Para[], chat: Said[], history?: History): Map<number, string[]> {
  const ids = idsOf(paras.map((p) => p.text))
  const out = new Map<number, string[]>()
  for (const m of chat) {
    const hit = new Set<number>()
    for (const id of m.text.match(IDS_IN) ?? []) {
      const hit2 = history ? history.find(ids, id) : null
      const n = history ? (hit2 && !hit2.taken ? hit2.n : undefined) : ids.indexOf(id) + 1 || undefined
      if (n) hit.add(n)
    }
    for (const n of hit) {
      const list = out.get(n) ?? []
      list.push(`${m.mine ? 'You' : m.name}: ${clip(m.text.replace(/\s+/g, ' '), 240)}`)
      out.set(n, list.slice(-3))
    }
  }
  return out
}

/**
 * Paragraph number `args[k]`, given as a number or an id, or undefined if it isn't given. An id
 * that changed since is followed to what it is now, and `notes` says so; with `strict`, one that
 * someone else changed since is refused, so an edit doesn't write over what its agent hasn't read.
 */
function paraNumber(paras: Para[], args: Args, k: string, history: History | undefined, notes: string[], strict = false): number | undefined {
  const v = args[k]
  if (v === undefined || v === null || v === '') return undefined
  if (typeof v === 'number') return Math.trunc(v)
  if (typeof v !== 'string') throw new ToolError(`${k} is a paragraph’s number or id.`)
  const t = v.trim()
  if (/^\d+$/.test(t)) return Number(t)
  const m = ID.exec(t.toLowerCase())
  if (!m) throw new ToolError(`${k} is a paragraph’s number, or its id like "#k3f9x2"; “${t}” is neither.`)
  const id = `#${m[1]}`
  const ids = idsOf(paras.map((p) => p.text))
  const found = history ? history.find(ids, id) : ids.indexOf(id) >= 0 ? { n: ids.indexOf(id) + 1, by: [] } : null
  if (!found) throw new ToolError(`No paragraph ${id} now: it was deleted, or changed in a way Writan couldn’t follow. Read the document again (find_text finds a paragraph by its words).`)
  if (found.taken) {
    const other = found.taken.other
    const theirs = other ? `paragraph ${other}, ${ids[other - 1]} now, had it before` : 'one next to it has the same text, and which is which can’t be told'
    throw new ToolError(`Not done: ${id} has meant two paragraphs since you last read it. Paragraph ${found.n} has it now, and ${theirs}. Read the document again, then use the id it gives.`)
  }
  const now = ids[found.n - 1]
  if (now !== id) {
    const others = found.by.filter((c) => c !== 'you')
    const what = `${id} has changed since${others.length ? ` (${others.join(' and ')} changed it)` : ''}: it’s paragraph ${found.n}${now ? `, ${now},` : ''} now: “${clip(shown(paras[found.n - 1].text), 160)}”.`
    if (strict && others.length) throw new ToolError(`Not done: ${what} Read it, then call again with ${now ?? `paragraph ${found.n}`} if it’s still what you mean.`)
    notes.push(what)
  }
  return found.n
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
const afterParas = (e: WordEngine, from: number, to: number, history?: History) => () => {
  const now = lines(e, from, to, history)
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

/** Applies text and paragraph formatting to the selection; `toggles: false` leaves bold and the like. */
function applyFormat(e: WordEngine, args: Args, toggles = true) {
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
    if (v !== undefined && toggles) toggleTo(e, what, v)
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

/** Paragraph number `k` of a tool's arguments, given by number or id (see `paraNumber`), in `paras` if given. */
type At = (k: string, strict?: boolean, paras?: Para[]) => number | undefined

function formatWord(e: WordEngine, args: Args, at: At, history?: History): Answer {
  checkFormat(args)
  const paras = paragraphsOf(e)
  const fromN = need(at('from'), 'from (the first paragraph to format)')
  const toN = at('to') ?? fromN
  const from = para(paras, fromN)
  const to = para(paras, toN)
  if (toN < fromN) throw new ToolError('to comes before from.')
  const find = str(args, 'find')
  if (!find) {
    grouped(e, () => {
      select(e, [from.path, 0], [to.path, to.text.length])
      applyFormat(e, args, false)
      // Bold and the like a paragraph at a time: turning one off across paragraphs of different
      // styles would follow only the first one's style.
      const toggles = ['bold', 'italic', 'underline', 'strikethrough'].some((k) => args[k] !== undefined)
      for (let n = fromN; toggles && n <= toN; n++) {
        const p = paras[n - 1]
        select(e, [p.path, 0], [p.path, p.text.length])
        applyFormat(e, { bold: args.bold, italic: args.italic, underline: args.underline, strikethrough: args.strikethrough })
      }
      select(e, [from.path, 0], [to.path, to.text.length])
    })
    return { text: `Formatted ${fromN === toN ? `paragraph ${fromN}` : `paragraphs ${fromN}–${toN}`}: ${looks(e)}.`, after: afterParas(e, fromN, toN, history) }
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
  return { text: `Formatted ${plural(count, 'match', 'matches')}: ${looks(e)}.`, after: afterParas(e, fromN, toN, history) }
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

/** Where insert_paragraphs and insert_table go: after paragraph `after`, or the last. */
function afterOf(args: Args, paras: Para[], at: At): number {
  const after = bool(args, 'at_end') === true || str(args, 'after')?.trim().toLowerCase() === 'end' ? paras.length : need(at('after'), 'after (a paragraph number or id, 0 for the start), or at_end')
  if (after < 0 || after > paras.length) throw new ToolError(`after is a paragraph number from 0 to ${paras.length}.`)
  return after
}

/** The paragraphs to write, from `paragraphs` or the lines of `text`, each with its style id if it names one (or `style` does). */
function newParagraphs(e: WordEngine, args: Args): { text: string; style: string | null }[] {
  const fallback = str(args, 'style')
  const list = args.paragraphs
  let out: { text: string; style?: string }[]
  if (Array.isArray(list) && list.length) {
    out = list.flatMap((p) => {
      const item = typeof p === 'string' ? { text: p } : p && typeof p === 'object' ? (p as Args) : {}
      const text = str(item, 'text')
      if (text === undefined) throw new ToolError('Each of paragraphs is like {"text": "…", "style": "Heading 1"}, with text.')
      const style = str(item, 'style') ?? fallback
      return text.replace(/\r\n?/g, '\n').split('\n').map((t) => ({ text: t, style }))
    })
  } else {
    out = need(str(args, 'text'), 'the text, or paragraphs')
      .replace(/\r\n?/g, '\n')
      .split('\n')
      .map((t) => ({ text: t, style: fallback }))
  }
  const ids = new Map<string, string>()
  return out.map((p) => {
    if (!p.style) return { text: p.text, style: null }
    if (!ids.has(p.style)) ids.set(p.style, styleId(e, p.style))
    return { text: p.text, style: ids.get(p.style)! }
  })
}

/** Sets the style of the paragraphs from number `first` on, each to its own: `styles[k]` for paragraph `first + k`, or leaves it where that's null. */
function styleEach(e: WordEngine, first: number, styles: (string | null)[]) {
  const now = paragraphsOf(e)
  styles.forEach((id, k) => {
    const p = now[first - 1 + k]
    if (!id || !p) return
    select(e, [p.path, 0], [p.path, p.text.length])
    e.run({ cmd: 'style', id })
  })
}

/**
 * For each of `b`, the index in `a` of the same paragraph if it was kept, else -1: the longest
 * common subsequence. Too long to compare (a big replace_text), nothing counts as kept.
 */
function keptFrom(a: string[], b: string[]): number[] {
  const out = b.map(() => -1)
  if (!a.length || !b.length || a.length * b.length > 4_000_000) return out
  const n = a.length
  const m = b.length
  // len[i][j]: the longest common run of a[i..] and b[j..].
  const len = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1))
  for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) len[i][j] = a[i] === b[j] ? len[i + 1][j + 1] + 1 : Math.max(len[i + 1][j], len[i][j + 1])
  for (let i = 0, j = 0; i < n && j < m; ) {
    if (a[i] === b[j]) {
      out[j] = i
      i++
      j++
    } else if (len[i + 1][j] >= len[i][j + 1]) i++
    else j++
  }
  return out
}

/** Who changed paragraphs in `History`, when it isn't known. */
type Client = string | null

/** Who wrote a paragraph that can't be told apart from the one next to it. */
const UNCLEAR = '?'

/**
 * Where a run of paragraphs `from` to `to` that was put in (or taken out of) `texts` could as well
 * have been, among neighbours with the same text: the first and last index it could cover.
 */
function slide(texts: string[], from: number, to: number): [number, number] {
  let s = from
  let e = to
  while (s > 0 && texts[s - 1] === texts[e]) {
    s--
    e--
  }
  const lo = s
  s = from
  e = to
  while (e + 1 < texts.length && texts[e + 1] === texts[s]) {
    s++
    e++
  }
  return [lo, e]
}

/**
 * What an agent's copy knows of each paragraph's past, from the changes it has seen: those made
 * before it joined too, which the relay's welcome brings. Who last changed its text (their client,
 * or null for what was there before anyone joined), the relay's number for that change, and what
 * each id became when its paragraph's text changed. The texts alone tell, by the paragraphs that
 * differ after each change, so it's a good guess, not a record.
 */
export class History {
  private texts: string[]
  private by: Client[]
  /** The relay's number of the change that last changed each paragraph's text; 0 for none. */
  private at: number[]
  /** Paragraphs deleted, with when and by whom. */
  private removed: { seq: number; client: Client; count: number }[] = []
  /** What an id became: the next id, and who changed the text (undefined when only the "-2" moved). */
  private next = new Map<string, { id: string; client?: Client }>()
  /**
   * Ids another paragraph has taken over since this agent was last shown them (the one that had it
   * has another now, or can't be told apart from its neighbours): which one the agent means can't
   * be told, so they're refused until it's shown them again.
   */
  private taken = new Set<string>()
  /** Stops following the document. */
  stop: () => void = () => {}
  constructor(
    private readonly e: WordEngine,
    /** This agent's copy, and its person's tab: what they wrote is the agent's and its person's. */
    private readonly me: string,
    private readonly mine: string | null,
    /** The name people here know a client by, here or gone. */
    private readonly nameOf: (client: string) => string | null,
  ) {
    this.texts = snapshotWord(e)
    this.by = this.texts.map(() => null)
    this.at = this.texts.map(() => 0)
  }

  /** The document changed, in change number `seq`: what differs from before is `client`'s. */
  saw(client: Client, seq: number) {
    const a = this.texts
    const b = snapshotWord(this.e)
    let start = 0
    while (start < a.length && start < b.length && a[start] === b[start]) start++
    let endA = a.length - 1
    let endB = b.length - 1
    while (endA >= start && endB >= start && a[endA] === b[endB]) {
      endA--
      endB--
    }
    if (start > endA && start > endB) return
    // In between, a paragraph that's still there as it was keeps its writer (the longest run of
    // them in order); the rest are new or changed, and `client`'s.
    const oldMid = a.slice(start, endA + 1)
    const newMid = b.slice(start, endB + 1)
    const kept = keptFrom(oldMid, newMid)
    const by = newMid.map((_, j) => (kept[j] >= 0 ? this.by[start + kept[j]] : client))
    const at = newMid.map((_, j) => (kept[j] >= 0 ? this.at[start + kept[j]] : seq))
    // What each id became. Between two kept paragraphs, the changed ones pair up in order (typing
    // in one, Enter splitting it, replacing three with three); one more of the old ones than the
    // new went into the last of them (two joined into one).
    const oldIds = idsOf(a)
    const newIds = idsOf(b)
    const now = new Set(newIds)
    const became = (i: number, j: number, changed: boolean) => {
      const from = oldIds[i]
      const to = newIds[j]
      if (!from || !to || from === to) return
      this.alias(from, to, changed ? client : undefined)
      if (now.has(from)) this.taken.add(from)
    }
    for (let i = 0; i < start; i++) became(i, i, false)
    for (let k = 0; k < a.length - 1 - endA; k++) became(endA + 1 + k, endB + 1 + k, false)
    let i0 = 0
    let j0 = 0
    const gap = (i1: number, j1: number) => {
      for (let i = i0; i < i1; i++) if (j0 < j1) became(start + i, start + Math.min(j0 + (i - i0), j1 - 1), true)
    }
    kept.forEach((i, j) => {
      if (i < 0) return
      gap(i, j)
      became(start + i, start + j, false)
      i0 = i + 1
      j0 = j + 1
    })
    gap(oldMid.length, newMid.length)
    const gone = oldMid.length - kept.filter((i) => i >= 0).length - newMid.filter((_, j) => kept[j] < 0).length
    if (gone > 0) {
      this.removed.push({ seq, client, count: gone })
      if (this.removed.length > 1000) this.removed.splice(0, this.removed.length - 1000)
    }
    this.by = [...this.by.slice(0, start), ...by, ...this.by.slice(endA + 1)]
    this.at = [...this.at.slice(0, start), ...at, ...this.at.slice(endA + 1)]
    this.texts = b
    // Paragraphs put in or deleted next to ones with the same text could as well have been the
    // others: which of them is whose, and which old id is which, can't be told.
    const [lo, hi] = !oldMid.length ? slide(b, start, endB) : !newMid.length ? slide(a, start, endA) : [0, -1]
    const span = !oldMid.length ? newMid.length : oldMid.length
    if (hi - lo + 1 > span) {
      const last = !oldMid.length ? hi : hi - span
      const unsure = new Set(newIds.slice(lo, last + 1))
      for (let i = lo; i <= last; i++) this.by[i] = UNCLEAR
      for (const [from, to] of [...this.next]) if (unsure.has(to.id)) this.next.delete(from)
      // The old ids of those it could have been: none of them can be followed.
      for (let i = lo; i <= (!oldMid.length ? hi - span : hi); i++) {
        const id = oldIds[i]
        if (!id) continue
        this.next.delete(id)
        if (now.has(id)) this.taken.add(id)
      }
    }
  }

  private alias(from: string, to: string, client?: Client) {
    this.next.delete(from)
    this.next.set(from, client === undefined ? { id: to } : { id: to, client })
    // The oldest go first: an id from long ago is unlikely to come up again.
    if (this.next.size > 50_000) this.next.delete(this.next.keys().next().value!)
  }

  /**
   * The paragraph an id is now, given every paragraph's id (`ids`): its number, and who changed its
   * text since the id was its ("you" for this agent). Null if none is.
   */
  find(ids: (string | null)[], id: string): { n: number; by: string[]; taken?: { other: number | null } } | null {
    if (this.taken.has(id) && ids.includes(id)) {
      const step = this.next.get(id)
      const other = step ? this.find(ids, step.id) : null
      return { n: ids.indexOf(id) + 1, by: [], taken: { other: other?.n ?? null } }
    }
    const by = new Set<string>()
    const seen = new Set<string>()
    for (let at: string | undefined = id; at && !seen.has(at); ) {
      const n = ids.indexOf(at)
      if (n >= 0) return { n: n + 1, by: [...by] }
      seen.add(at)
      const step = this.next.get(at)
      if (step && step.client !== undefined) by.add(step.client === this.me ? 'you' : this.who(step.client))
      at = step?.id
    }
    return null
  }

  private who(c: Client): string {
    if (!c) return 'someone'
    if (c === UNCLEAR) return 'unclear: one next to it has the same text'
    if (c === this.me) return 'you'
    const name = this.nameOf(c)
    if (c === this.mine) return name ? `${name}, your person` : 'your person'
    return name ?? 'someone who has left'
  }

  /** The agent has been shown paragraph id `id` as it is now. */
  shown(id: string | null | undefined) {
    if (id) this.taken.delete(id)
  }

  /** Who last changed paragraph `n`'s text, as read_document says it; null for nobody since it was shared. */
  label(n: number): string | null {
    const c = this.by[n - 1]
    return c ? this.who(c) : null
  }

  /** Who wrote how many paragraphs, for the top of a whole document's reading. */
  summary(): string {
    const count = new Map<string, number>()
    let before = 0
    this.by.forEach((c, i) => {
      if (!this.texts[i]?.trim()) return
      if (!c) before++
      else count.set(this.who(c), (count.get(this.who(c)) ?? 0) + 1)
    })
    if (!count.size) return ''
    const parts = [...count].map(([who, n]) => `${who} ${n}`)
    return `Who last changed each paragraph: ${parts.join(', ')}${before ? `; ${before} as they were before anyone joined` : ''}.`
  }

  /** The numbers of the paragraphs whose text changed after change number `seq`. */
  changedSince(seq: number): number[] {
    return this.at.flatMap((at, i) => (at > seq ? [i + 1] : []))
  }

  /** Paragraphs deleted after change number `seq`, by whom. */
  removedSince(seq: number): { by: string | null; count: number }[] {
    const out = new Map<string | null, number>()
    for (const r of this.removed) if (r.seq > seq) out.set(r.client ? this.who(r.client) : null, (out.get(r.client ? this.who(r.client) : null) ?? 0) + r.count)
    return [...out].map(([by, count]) => ({ by, count }))
  }

  /** Who wrote paragraph `n`, when it's someone else. An empty one has nothing to keep. */
  private other(n: number): string | null {
    const c = this.by[n - 1]
    return c && c !== this.me && c !== this.mine && this.texts[n - 1]?.trim() ? c : null
  }

  /** Whether this agent wrote paragraph `n`. */
  wrote(n: number): boolean {
    return this.by[n - 1] === this.me
  }

  /** Refuses to change paragraphs `from` to `to` without `confirm` when someone else wrote any of them. */
  check(from: number, to: number, args: Args, what: string) {
    if (bool(args, 'confirm') === true) return
    const who = new Set<string>()
    const which: number[] = []
    for (let n = from; n <= to; n++) {
      const c = this.other(n)
      if (c) {
        who.add(this.who(c))
        which.push(n)
      }
    }
    if (!which.length) return
    const list = which.length > 6 ? `${which.slice(0, 6).join(', ')}…` : which.join(', ')
    const unclear = which.filter((n) => this.by[n - 1] === UNCLEAR)
    const known = which.filter((n) => this.by[n - 1] !== UNCLEAR)
    const sameText = unclear.length ? ` ${unclear.length === 1 ? `Paragraph ${unclear[0]} has` : `Paragraphs ${unclear.slice(0, 6).join(', ')} have`} the same text as one next to it, so Writan can’t tell whose ${unclear.length === 1 ? 'it is' : 'each is'}.` : ''
    if (!known.length) throw new ToolError(`Not ${what}:${sameText} Check it’s what you mean, then call again with confirm: true.`)
    who.delete(this.who(UNCLEAR))
    const knownList = known.length > 6 ? `${known.slice(0, 6).join(', ')}…` : known.join(', ')
    if (unclear.length) throw new ToolError(
      `Not ${what}: ${known.length === 1 ? `paragraph ${knownList} is` : `paragraphs ${knownList} are`} ${[...who].join(' and ')}’s.${sameText} Ask them in the chat first (send_message), or your person; once they agree, call again with confirm: true.`,
    )
    throw new ToolError(
      `Not ${what}: ${which.length === 1 ? `paragraph ${list} is` : `paragraphs ${list} are`} ${[...who].join(' and ')}’s. Ask them in the chat first (send_message), or your person; once they agree, call again with confirm: true.`,
    )
  }
}

export async function wordTool(e: WordEngine, name: string, args: Args, local: boolean, history?: History): Promise<Answer> {
  const paras = paragraphsOf(e)
  // What an id that changed since is now, for the answer.
  const notes: string[] = []
  const at: At = (k, strict = false, now = paras) => paraNumber(now, args, k, history, notes, strict)
  const answer = await wordEdit(e, paras, at, name, args, local, history)
  return notes.length ? { ...answer, text: `${[...new Set(notes)].join('\n')}\n${answer.text}` } : answer
}

async function wordEdit(e: WordEngine, paras: Para[], at: At, name: string, args: Args, local: boolean, history?: History): Promise<Answer> {
  switch (name) {
    case 'insert_paragraphs': {
      const after = afterOf(args, paras, at)
      const lines = newParagraphs(e, args)
      const count = lines.length
      // They go in as paragraphs of their own, next to the one before without changing it, so
      // someone else adding paragraphs at the same place at the same moment keeps theirs too.
      // Without a style of their own, they're what Enter after the paragraph before gives: Normal
      // after a title. At the start there's none before, so it's what follows the first.
      // At the end, after everything in the body, even a table it ends with, like the last
      // paragraph that isn't in one.
      const atEnd = after === paras.length && after > 0 && (bool(args, 'at_end') === true || str(args, 'after')?.trim().toLowerCase() === 'end')
      const like = atEnd ? ([...paras].reverse().find((p) => p.path.length === 1) ?? para(paras, after)) : para(paras, Math.max(1, after))
      // The engine takes an index past the body's end as its end (there may be blocks after the
      // last paragraph that the list of paragraphs doesn't show).
      const where = atEnd
        ? [0xffffffff]
        : after === 0
          ? like.path
          : [...like.path.slice(0, -1), like.path[like.path.length - 1] + 1]
      const result = e.run<string>({
        cmd: 'insertParagraphs',
        container: where.slice(0, -1),
        index: where[where.length - 1],
        like: like.path,
        paras: lines.map((l) => ({ text: l.text, style: l.style ?? like.next ?? null })),
      })
      if (result !== 'ok') throw new ToolError('That paragraph isn’t there any more. Read the document again.')
      // Someone else's may have gone in at the same place first: say where these are once the
      // relay has ordered them.
      const placed = (): number | null => {
        const texts = paragraphsOf(e).map((p) => p.text)
        const fits = (i: number) => lines.every((l, k) => texts[i + k] === l.text)
        // Nobody else added or removed paragraphs meanwhile: where they went.
        if (texts.length === paras.length + count && fits(after)) return after + 1
        // Someone did: the place with that text, if there's only one (that this agent wrote, when
        // there's more than one); else it can't tell which.
        const starts = texts.flatMap((_, i) => (fits(i) ? [i] : []))
        const mine = starts.filter((i) => history?.wrote(i + 1))
        const only = starts.length === 1 ? starts : mine
        return only.length === 1 ? only[0] + 1 : null
      }
      return {
        text: `Put in ${plural(count, 'paragraph')}.`,
        after: () => {
          const n = placed()
          if (n === null) return '\nSomeone else changed the document at the same time, and paragraphs with the same text are elsewhere too: read it again to see where these went.'
          return `\n${count > 1 ? `They’re paragraphs ${n}–${n + count - 1}` : `It’s paragraph ${n}`}.${afterParas(e, n, n + count - 1, history)()}`
        },
      }
    }
    case 'replace_paragraphs': {
      const fromN = need(at('from', true), 'from')
      const from = para(paras, fromN)
      const toN = at('to', true) ?? fromN
      const to = para(paras, toN)
      if (toN < fromN) throw new ToolError('to comes before from.')
      // Within one list of paragraphs, as for delete_paragraphs: a range into a table would
      // take the table with it.
      if (paras.slice(fromN - 1, toN).some((p) => !sameList(p.path, from.path))) {
        throw new ToolError('Those paragraphs aren’t all in one place (a table is in between, or some are in a table cell). Replace them in parts.')
      }
      for (let i = fromN; i < toN; i++) if (!nextTo(paras[i - 1].path, paras[i].path)) throw new ToolError('Something that isn’t a paragraph is among them. Replace the paragraphs around it separately.')
      const lines = newParagraphs(e, args)
      history?.check(fromN, toN, args, 'replaced')
      // The first keeps its paragraph's style; the rest are what Enter after it gives, as typing
      // them would make them: body text after a title, not more title.
      grouped(e, () => {
        select(e, [from.path, 0], [to.path, to.text.length])
        e.run({ cmd: 'paste', text: lines.map((l) => l.text).join('\n'), json: null })
        styleEach(e, fromN, lines.map((l, k) => l.style ?? (k > 0 ? (from.next ?? null) : null)))
      })
      return { text: 'Replaced.', after: afterParas(e, fromN, fromN + lines.length - 1, history) }
    }
    case 'delete_paragraphs': {
      const fromN = need(at('from', true), 'from')
      const toN = at('to', true) ?? fromN
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
      history?.check(fromN, toN, args, 'deleted')
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
      return { text: `Deleted ${plural(toN - fromN + 1, 'paragraph')}.`, after: afterParas(e, fromN - 1, fromN, history) }
    }
    case 'set_style': {
      const fromN = need(at('from'), 'from')
      const toN = at('to') ?? fromN
      const from = para(paras, fromN)
      const to = para(paras, toN)
      const id = styleId(e, need(str(args, 'style'), 'the style'))
      select(e, [from.path, 0], [to.path, to.text.length])
      e.run({ cmd: 'style', id })
      return { text: 'Done.', after: afterParas(e, fromN, toN, history) }
    }
    case 'format_text':
      return formatWord(e, args, at, history)
    case 'insert_table': {
      const after = afterOf(args, paras, at)
      const cells = Array.isArray(args.cells) ? (args.cells as unknown[]).map((r) => (Array.isArray(r) ? r : [r])) : null
      const rows = int(args, 'rows') ?? cells?.length ?? 2
      const cols = int(args, 'columns') ?? Math.max(1, ...(cells ?? [[0, 0]]).map((r) => r.length))
      if (rows < 1 || rows > 200 || cols < 1 || cols > 60) throw new ToolError('A table has 1 to 200 rows and 1 to 60 columns.')
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
      return { text: `Put in a ${rows} × ${cols} table.`, after: afterParas(e, first + 1, first + rows * cols, history) }
    }
    case 'change_table': {
      const n = need(at('paragraph'), 'paragraph')
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
      const n = need(at('paragraph'), 'paragraph')
      const which = int(args, 'picture') ?? 1
      const img = selectPicture(e, n, which)
      if (args.reset === true) {
        const c = img.crop
        let uncropped = { w: img.widthEmu, h: img.heightEmu }
        grouped(e, () => {
          // Uncropped first, at the scale it shows now; then its own size where Writan knows it.
          if (c.left || c.top || c.right || c.bottom) {
            const w = img.widthEmu / Math.max(0.01, 1 - (c.left + c.right) / 100)
            const h = img.heightEmu / Math.max(0.01, 1 - (c.top + c.bottom) / 100)
            e.run({ cmd: 'cropImage', left: 0, top: 0, right: 0, bottom: 0, widthEmu: Math.round(w), heightEmu: Math.round(h) })
            uncropped = { w: Math.round(w), h: Math.round(h) }
          }
          e.run({ cmd: 'resetImageSize' })
        })
        // Only a size that changed is known to be the picture's own (Writan can't read some kinds').
        const now = e.state.image
        const sized = now && (Math.abs(now.widthEmu - uncropped.w) > 1 || Math.abs(now.heightEmu - uncropped.h) > 1)
        const text = sized ? 'Uncropped, and back to its own size.' : 'Uncropped. Its size is as below; give width or height to change it.'
        return { text, after: () => pictureNow(e, n, which) }
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
      const what = 'after (the paragraph to put it after, by number or id; 0 for the start)'
      need(at('after'), what)
      const got = await pictureBytes(args, local)
      const type = pictureType(got.bytes)
      if (!type) throw new ToolError('That isn’t a picture Writan can put in a document. Give a PNG, JPEG, GIF, WebP or SVG.')
      const pic = await scaledPicture(got.bytes, type, 2400, 2400)
      // Where it goes once the picture is ready: others may have changed the document meanwhile.
      const now = paragraphsOf(e)
      const after = need(at('after', false, now), what)
      if (after < 0 || after > now.length) throw new ToolError(`after is a paragraph number from 0 to ${now.length}.`)
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

/** find_text in a Word document: each paragraph with a match, by number. */
export function findInWord(e: WordEngine, find: string, matchCase: boolean, history?: History): string {
  let total = 0
  const paras = paragraphsOf(e)
  const ids = idsOf(paras.map((p) => p.text))
  const places = paras.flatMap((p, i) => {
    const hit = found(shown(p.text), find, matchCase)
    if (!hit) return []
    total += hit.count
    history?.shown(ids[i])
    return [`${label(i + 1, ids[i])}${hit.count > 1 ? ` (${hit.count} times)` : ''} ${hit.around}`]
  })
  return foundAnswer(find, places, total)
}

export const snapshotWord = (e: WordEngine) => paragraphsOf(e).map((p) => p.text)

/** The paragraphs that differ, from the first change to the last; `full` gives them whole, however many. */
export function wordChanges(a: string[], b: string[], full = false): string {
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
    const ids = idsOf(b)
    const most = full ? 500 : 12
    const shownLines = b.slice(start, endB + 1).map((t, k) => `${label(start + k + 1, ids[start + k])} ${full ? shown(t) : clip(shown(t), 300)}`)
    out.push(`${added === 1 ? `Paragraph ${start + 1} is` : `Paragraphs ${start + 1}–${endB + 1} are`} now:`, ...shownLines.slice(0, most))
    if (shownLines.length > most) out.push(`(and ${shownLines.length - most} more${full ? '' : '; full: true gives up to 500, whole'})`)
  }
  if (removed > added) out.push(`${plural(removed - added, 'paragraph')} fewer than before.`)
  return out.join('\n')
}
