// What an agent can do in a presentation: read it, see its slides, and do what people do in the
// editor: write and format text, move, resize, crop and restack shapes and pictures, add and
// delete them, and add, move, copy and hide slides. Each edit is the editor's own command, and
// answers with the shape or slide as it is once everyone has the change.

import type { Element, SlidesEngine } from '../../engine/slides/engine'
import { type Answer, type Args, bool, BOX, canvasPicture, clip, doc, found, foundAnswer, grouped, hexColor, int, need, num, plural, str, TEXT_FORMAT, type Tool, ToolError } from './common'
import { draw, type Mark, naturalSize } from './picture'
import { pictureBytes, pictureType, scaledPicture } from './pictures'

const slide = { type: 'integer', description: 'From 1.' }
const shape = { type: 'string', description: 'Its id, like "4".' }
const shapes = { type: 'array', items: { type: 'string' }, description: 'Ids, like ["4", "7"].' }
const fill = { type: 'string', description: 'Like "#1F4E79", or "none".' }
const outline = { type: 'string', description: 'Like "#1F4E79", or "none".' }
const cropSides = {
  left: { type: 'number', description: 'The share to crop, 0 to under 1.' },
  top: { type: 'number', description: 'The share to crop.' },
  right: { type: 'number', description: 'The share to crop.' },
  bottom: { type: 'number', description: 'The share to crop.' },
}
const box = { x: BOX.x, y: BOX.y, width: BOX.width, height: BOX.height }

export const SLIDES_TOOLS: Tool[] = [
  {
    name: 'add_slide',
    description:
      'PowerPoint: adds a slide after `after`, or at the end, and returns its number and shapes. With `copy`, copies that slide and its notes to right after it instead.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        after: { type: 'integer', description: '0 for the start. Default: the end.' },
        layout: { type: 'string', description: 'As read_document lists them.' },
        title: { type: 'string', description: 'For the title placeholder.' },
        body: { type: 'string', description: 'For the body placeholder.' },
        copy: { type: 'integer', description: 'A slide to copy instead.' },
      },
      required: ['document'],
    },
  },
  {
    name: 'edit_slide',
    description: 'PowerPoint: changes a slide’s layout, whether the slideshow skips it, its notes, or its position. Give only what changes.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        slide,
        layout: { type: 'string', description: 'As read_document lists them.' },
        hidden: { type: 'boolean', description: 'Skipped in the slideshow.' },
        notes: { type: 'string', description: 'Replaces the speaker notes.' },
        to: { type: 'integer', description: 'Its new number.' },
      },
      required: ['document', 'slide'],
    },
  },
  {
    name: 'delete_slide',
    description: 'PowerPoint: deletes a slide.',
    inputSchema: { type: 'object', properties: { document: doc, slide }, required: ['document', 'slide'] },
  },
  {
    name: 'add_shape',
    description:
      'PowerPoint: adds a text box, shape or table in front of the others, and returns its id. It goes in the middle unless x and y are given.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        slide,
        kind: { type: 'string', enum: ['text box', 'rectangle', 'rounded rectangle', 'oval', 'triangle', 'diamond', 'star', 'arrow', 'line', 'table'] },
        text: { type: 'string', description: 'Not for a line or table.' },
        rows: { type: 'integer', description: 'Table: 1 to 50. Default 2.' },
        columns: { type: 'integer', description: 'Table: 1 to 20. Default 2.' },
        ...box,
        ...TEXT_FORMAT,
        fill,
        outline,
      },
      required: ['document', 'slide', 'kind'],
    },
  },
  {
    name: 'set_shape_text',
    description: 'PowerPoint: replaces a shape’s text, keeping its formatting.',
    inputSchema: { type: 'object', properties: { document: doc, slide, shape, text: { type: 'string', description: 'One paragraph per line.' } }, required: ['document', 'slide', 'shape', 'text'] },
  },
  {
    name: 'set_table_text',
    description: 'PowerPoint: writes cells of table `shape` from `row` and `column` on. The table grows to fit.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        slide,
        shape,
        row: { type: 'integer', description: 'Default 1.' },
        column: { type: 'integer', description: 'Default 1.' },
        cells: { type: 'array', items: { type: 'array', items: { type: 'string' } }, description: 'Rows of cell text, like [["Dish", "Price"]].' },
      },
      required: ['document', 'slide', 'shape', 'cells'],
    },
  },
  {
    name: 'edit_shape',
    description:
      'PowerPoint: moves, resizes, rotates, flips or restacks a shape, sets its fill and outline, or sets its slideshow link. Give only what changes. Returns where it is and what it overlaps.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        slide,
        shape,
        ...box,
        keep_proportions: { type: 'boolean', description: 'With only width or height, scale the other. Default true.' },
        rotation: { type: 'number', description: 'Degrees clockwise.' },
        flip_horizontal: { type: 'boolean', description: 'Mirrored left to right.' },
        flip_vertical: { type: 'boolean', description: 'Upside down.' },
        layer: { type: 'string', enum: ['front', 'forward', 'backward', 'back'], description: 'forward and backward move one step.' },
        fill,
        outline,
        outline_width: { type: 'number', description: 'In points.' },
        link: { type: 'string', description: 'A web address, or "" for none.' },
      },
      required: ['document', 'slide', 'shape'],
    },
  },
  {
    name: 'crop_picture',
    description:
      'PowerPoint: crops a picture. `fit: "fill"` fills its frame without stretching. `fit: "fit"` shows it whole and shrinks the frame to match. Or give left, top, right and bottom.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        slide,
        shape,
        fit: { type: 'string', enum: ['fill', 'fit'] },
        keep: { type: 'string', enum: ['center', 'top', 'bottom', 'left', 'right'], description: 'With fit "fill": the part to keep.' },
        ...cropSides,
      },
      required: ['document', 'slide', 'shape'],
    },
  },
  {
    name: 'copy_shapes',
    description: 'PowerPoint: copies shapes to `to_slide`, or onto the same slide, and returns the copies’ ids.',
    inputSchema: {
      type: 'object',
      properties: { document: doc, slide, shapes, to_slide: { type: 'integer', description: 'Default: the same slide.' } },
      required: ['document', 'slide', 'shapes'],
    },
  },
  {
    name: 'group_shapes',
    description: 'PowerPoint: groups shapes so they move together, and returns the group’s id.',
    inputSchema: { type: 'object', properties: { document: doc, slide, shapes: { ...shapes, description: 'At least two.' } }, required: ['document', 'slide', 'shapes'] },
  },
  {
    name: 'ungroup_shapes',
    description: 'PowerPoint: splits a group into its shapes and returns their ids.',
    inputSchema: { type: 'object', properties: { document: doc, slide, group: { type: 'string', description: 'The group’s id.' } }, required: ['document', 'slide', 'group'] },
  },
  {
    name: 'delete_shapes',
    description: 'PowerPoint: deletes shapes.',
    inputSchema: { type: 'object', properties: { document: doc, slide, shapes }, required: ['document', 'slide', 'shapes'] },
  },
]

// ── Reading

const elementsOf = (e: SlidesEngine, i: number) => JSON.parse(e.doc.elements(i)) as Element[]

const r = (v: number) => Math.round(v)
const pct = (v: number) => `${Math.round(v * 100)}%`

const KINDS: Record<string, string> = { text: 'text', shape: 'shape', connector: 'line', image: 'picture', table: 'table', group: 'group', other: 'object' }

/** One shape, as read_document and the edits' answers show it. */
export function shapeLine(el: Element, depth = 1): string {
  const bits: string[] = [`${'  '.repeat(depth)}- shape "${el.id}" ${KINDS[el.type] ?? el.type}${el.placeholder ? ` (${el.placeholder} placeholder)` : ''} “${el.name}”`]
  let where = `at x ${r(el.x)}, y ${r(el.y)}, ${r(el.width)} × ${r(el.height)}`
  if (el.rotation) where += `, turned ${r(el.rotation)}°`
  if (el.flipHorizontal || el.flipVertical) where += `, flipped ${[el.flipHorizontal && 'horizontally', el.flipVertical && 'vertically'].filter(Boolean).join(' and ')}`
  const looks: string[] = [where]
  if (el.crop && el.crop.some((v) => v > 0.0005)) {
    const [l, t, rr, b] = el.crop
    looks.push(`cropped ${[l && `${pct(l)} left`, t && `${pct(t)} top`, rr && `${pct(rr)} right`, b && `${pct(b)} bottom`].filter(Boolean).join(', ')}`)
  }
  const s = el.shapeStyle
  if (s && el.type !== 'text') {
    if (s.fillMode === 'solid' && s.fillColor) looks.push(`fill ${s.fillColor}`)
    if (s.strokeWidth > 0 && s.strokeColor) looks.push(`outline ${s.strokeColor} ${s.strokeWidth}pt`)
  }
  const t = el.textStyle ?? el.table?.textStyle
  if (t && (el.text || el.table)) {
    const f = [t.fontFamily, t.fontSize ? `${t.fontSize}pt` : null, t.bold && 'bold', t.italic && 'italic', t.underline && 'underlined', t.color, t.align !== 'left' ? t.align : null].filter(Boolean)
    looks.push(`text ${f.join(' ')}`)
  }
  if (el.table) looks.push(`${el.table.rows} rows × ${el.table.columns} columns`)
  if (el.link?.url) looks.push(`links to ${el.link.url}`)
  bits.push(`: ${looks.join('; ')}`)
  if (el.text) bits.push(` — “${el.text.replace(/\n/g, ' / ')}”`)
  return bits.join('')
}

function slideLines(e: SlidesEngine, i: number): string[] {
  const s = e.slides[i]
  const out = [`Slide ${i + 1}${s.layoutName ? ` (layout ${s.layoutName})` : ''}${s.hidden ? ' (hidden)' : ''}`]
  const walk = (els: Element[], depth: number) => {
    for (const el of els) {
      out.push(shapeLine(el, depth))
      if (el.children) walk(el.children, depth + 1)
    }
  }
  walk(elementsOf(e, i), 1)
  if (s.notes) out.push(`  notes: ${s.notes.replace(/\n/g, ' / ')}`)
  return out
}

export function readSlides(e: SlidesEngine, args: Args): string {
  const one = int(args, 'slide')
  if (one !== undefined) slideIndex(e, one)
  const out = [
    `${plural(e.slides.length, 'slide')}, each ${r(e.width)} × ${r(e.height)} pixels (96 to an inch). Layouts: ${e.layouts.map((l) => l.name).join(', ')}.`,
    'Shapes are listed back to front: a later one covers an earlier one where they overlap. view_document shows a slide as a picture.',
  ]
  e.slides.forEach((_, i) => {
    if (one !== undefined && one !== i + 1) return
    out.push('', ...slideLines(e, i))
  })
  return out.join('\n')
}

/** find_text in a presentation: each shape or slide's notes with a match. */
export function findInSlides(e: SlidesEngine, find: string, matchCase: boolean): string {
  let total = 0
  const places: string[] = []
  const add = (where: string, text: string) => {
    const hit = found(text, find, matchCase)
    if (!hit) return
    total += hit.count
    places.push(`- ${where}${hit.count > 1 ? ` (${hit.count} times)` : ''}: ${hit.around}`)
  }
  e.slides.forEach((s, i) => {
    const walk = (els: Element[]) => {
      for (const el of els) {
        if (el.text) add(`slide ${i + 1}, shape "${el.id}" “${el.name}”`, el.text)
        if (el.children) walk(el.children)
      }
    }
    walk(elementsOf(e, i))
    if (s.notes) add(`slide ${i + 1}, notes`, s.notes)
  })
  return foundAnswer(find, places, total)
}

export function slideIndex(e: SlidesEngine, n: number | undefined): number {
  if (n === undefined || n < 1 || n > e.slides.length) throw new ToolError(`There’s no slide ${n ?? ''}; the presentation has ${e.slides.length} (1 to ${e.slides.length}).`)
  return n - 1
}

/** A shape on a slide, anywhere in its groups, and the group it's in. */
function findShape(e: SlidesEngine, i: number, id: string): { el: Element; group: Element | null } {
  const walk = (els: Element[], group: Element | null): { el: Element; group: Element | null } | null => {
    for (const el of els) {
      if (el.id === id) return { el, group }
      const hit = el.children && walk(el.children, el)
      if (hit) return hit
    }
    return null
  }
  const hit = walk(elementsOf(e, i), null)
  if (!hit) throw new ToolError(`Slide ${i + 1} has no shape "${id}". read_document lists them.`)
  return hit
}

/** A shape that isn't inside a group: the ones that move, restack and get deleted on their own. */
function topShape(e: SlidesEngine, i: number, id: string, doing: string): Element {
  const { el, group } = findShape(e, i, id)
  if (group) throw new ToolError(`Shape "${id}" is inside group "${group.id}". To ${doing} it on its own, ungroup the group first (ungroup_shapes), or ${doing} the whole group.`)
  return el
}

const shapeIds = (args: Args): string[] => {
  const v = args.shapes
  const list = Array.isArray(v) ? v.map(String) : typeof v === 'string' ? v.split(',').map((s) => s.trim()) : []
  if (!list.length) throw new ToolError('Give shapes, a list of ids like ["4", "7"].')
  return list
}

/** What a shape overlaps, and whether it's off the slide: for the answer after moving it. */
function surroundings(e: SlidesEngine, i: number, el: Element): string {
  const notes: string[] = []
  const right = el.x + el.width
  const bottom = el.y + el.height
  if (el.x < -1 || el.y < -1 || right > e.width + 1 || bottom > e.height + 1) {
    const inside = el.x < e.width && right > 0 && el.y < e.height && bottom > 0
    notes.push(inside ? 'It runs off the edge of the slide.' : 'It’s off the slide, so it won’t show.')
  }
  const covers: string[] = []
  for (const other of elementsOf(e, i)) {
    if (other.id === el.id) continue
    const w = Math.min(right, other.x + other.width) - Math.max(el.x, other.x)
    const h = Math.min(bottom, other.y + other.height) - Math.max(el.y, other.y)
    // A shape that covers most of the slide is a background, which everything overlaps.
    if (w > 2 && h > 2 && other.width * other.height < e.width * e.height * 0.8) covers.push(`"${other.id}" “${other.name}”`)
  }
  if (covers.length) notes.push(`It overlaps ${covers.join(', ')}.`)
  return notes.length ? `\n${notes.join(' ')}` : ''
}

/** The answer after a change to a shape: the shape as it is once everyone has the change. */
const afterShape = (e: SlidesEngine, i: number, id: string, surround = false) => () => {
  if (i >= e.slides.length) return ''
  const hit = findHit(e, i, id)
  if (!hit) return `\nShape "${id}" isn’t on slide ${i + 1} any more; someone else changed it. Read the slide again.`
  const order = elementsOf(e, i).map((el) => el.id)
  const at = order.indexOf(id)
  const layer = surround && at >= 0 ? `\nIt’s ${at + 1} of ${order.length} from the back.` : ''
  return `\nSlide ${i + 1}, now:\n${shapeLine(hit)}${surround ? surroundings(e, i, hit) : ''}${layer}`
}

function findHit(e: SlidesEngine, i: number, id: string): Element | null {
  try {
    return findShape(e, i, id).el
  } catch {
    return null
  }
}

// ── Seeing

/** A slide as a picture, each shape outlined with its id; or just one shape, closer up. */
export async function viewSlide(e: SlidesEngine, args: Args): Promise<Answer> {
  const i = slideIndex(e, int(args, 'slide'))
  const labels = args.labels !== false
  const els = elementsOf(e, i)
  const marks: Mark[] = []
  const walk = (list: Element[]) => {
    for (const el of list) {
      marks.push({ label: el.id, x: el.x, y: el.y, w: el.width, h: el.height, rot: el.rotation })
      if (el.children) walk(el.children)
    }
  }
  if (labels) walk(els)
  const id = str(args, 'shape')
  let view: { x: number; y: number; w: number; h: number } | undefined
  if (id) {
    const el = findShape(e, i, id).el
    const pad = 12
    view = { x: Math.max(0, el.x - pad), y: Math.max(0, el.y - pad), w: Math.min(e.width, el.x + el.width + pad) - Math.max(0, el.x - pad), h: Math.min(e.height, el.y + el.height + pad) - Math.max(0, el.y - pad) }
    if (view.w < 4 || view.h < 4) throw new ToolError(`Shape "${id}" is off the slide, so there’s nothing to show.`)
  }
  const want = Math.min(2000, Math.max(200, num(args, 'width') ?? 1280))
  // A small shape close up is drawn at most three times its size, unless a width asks for more.
  const scale = Math.min(want / (view?.w ?? e.width), view && num(args, 'width') === undefined ? 3 : Infinity)
  const canvas = await draw({ width: e.width, height: e.height, items: e.paintItems(i), styles: e.styles, media: e.media }, scale, marks, view)
  const picture = await canvasPicture(canvas)
  const text = [
    `Slide ${i + 1}${id ? `, shape "${id}"` : ''}, ${canvas.width} × ${canvas.height} (the slide is ${r(e.width)} × ${r(e.height)} pixels).${labels ? ' Each shape is outlined in pink with its id.' : ''}`,
    ...slideLines(e, i).slice(1),
  ].join('\n')
  return { text, pictures: [picture] }
}

// ── Editing

/** Types `text` into a shape on the active slide, replacing what's there. */
function typeInto(e: SlidesEngine, id: string, text: string) {
  e.run({ cmd: 'beginEdit', id: Number(id), x: null, y: null })
  if (e.state.editing !== id) throw new ToolError(`Shape "${id}" doesn’t hold text.`)
  e.run({ cmd: 'selectAllText' })
  typeLines(e, text)
}

function typeLines(e: SlidesEngine, text: string) {
  text
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .forEach((line, i) => {
      if (i) e.run({ cmd: 'enter' })
      if (line) e.run({ cmd: 'insertText', text: line })
      // An empty shape needs the selection gone before the first line.
      else if (i === 0) e.run({ cmd: 'deleteText', word: false })
    })
}

interface TextPatch {
  fontFamily?: string
  fontSize?: number
  bold?: boolean
  italic?: boolean
  underline?: boolean
  strikethrough?: boolean
  color?: string
  align?: string
}

function textPatch(args: Args): TextPatch {
  const p: TextPatch = {}
  const font = str(args, 'font')
  if (font) p.fontFamily = font
  const size = num(args, 'size')
  if (size !== undefined) {
    if (size < 1 || size > 400) throw new ToolError('Give a font size in points, from 1 to 400.')
    p.fontSize = size
  }
  for (const k of ['bold', 'italic', 'underline', 'strikethrough'] as const) {
    const v = bool(args, k)
    if (v !== undefined) p[k] = v
  }
  const color = str(args, 'color')
  if (color) p.color = `#${hexColor(color, 'The color')}`
  const align = str(args, 'align')
  if (align) {
    if (!['left', 'center', 'right', 'justify'].includes(align)) throw new ToolError('align is left, center, right or justify.')
    p.align = align
  }
  return p
}

const SHAPES: Record<string, string> = {
  rectangle: 'rect',
  rect: 'rect',
  square: 'rect',
  'rounded rectangle': 'roundRect',
  roundrect: 'roundRect',
  oval: 'ellipse',
  ellipse: 'ellipse',
  circle: 'ellipse',
  triangle: 'triangle',
  diamond: 'diamond',
  star: 'star5',
  star5: 'star5',
  arrow: 'rightArrow',
  'right arrow': 'rightArrow',
  rightarrow: 'rightArrow',
  line: 'line',
}

/** Width and height the agent gave, checked before anything changes. A line can be 0 one way. */
function checkSize(args: Args) {
  for (const k of ['width', 'height']) {
    const v = num(args, k)
    if (v !== undefined && v < 1) throw new ToolError('Width and height must be at least 1 pixel.')
  }
}

/** A color an agent gave, or "none" (null); checked before anything changes. */
const colorArg = (args: Args, k: string, what: string) => {
  const v = str(args, k)
  if (v === undefined) return undefined
  return v.trim().toLowerCase() === 'none' ? null : hexColor(v, what)
}

/** Where a new or moved shape goes: what's given, the rest from `from`, keeping proportions when one side is given. */
function boxFrom(args: Args, from: { x: number; y: number; width: number; height: number }, keep = true) {
  const w0 = num(args, 'width')
  const h0 = num(args, 'height')
  let w = w0 ?? from.width
  let h = h0 ?? from.height
  if (keep && from.width > 0 && from.height > 0) {
    if (w0 !== undefined && h0 === undefined) h = (from.height * w0) / from.width
    if (h0 !== undefined && w0 === undefined) w = (from.width * h0) / from.height
  }
  return { x: num(args, 'x') ?? from.x, y: num(args, 'y') ?? from.y, w, h }
}

/** Shows the agent on the shapes it changed, without leaving it typing in one. */
function pick(e: SlidesEngine, ids: string[]) {
  if (e.state.editing) e.run({ cmd: 'endEdit' })
  e.run({ cmd: 'selectFrames', ids: ids.map(Number) })
}

/** Selects text in a shape (on the active slide) from (paragraph, offset) to (paragraph, offset), offsets in UTF-16 units. */
function selectIn(e: SlidesEngine, id: string, a: [number[], number], f: [number[], number]) {
  if (!e.run<boolean>({ cmd: 'selectText', id: Number(id), anchor: a, focus: f })) throw new ToolError(`Shape "${id}" doesn’t hold text.`)
}

interface ParaText {
  frame: string
  path: number[]
  text: string
}

const textsOf = (e: SlidesEngine, id: string) => (JSON.parse(e.doc.slideText()) as ParaText[]).filter((t) => t.frame === id)

function formatText(e: SlidesEngine, args: Args): Answer {
  const wordOnly = ['style', 'highlight', 'list', 'line_spacing'].filter((k) => args[k] !== undefined)
  if (wordOnly.length) throw new ToolError(`${wordOnly.join(', ')} ${wordOnly.length === 1 ? 'is' : 'are'} for Word documents. In a presentation, format_text sets font, size, bold, italic, underline, strikethrough, color and align.`)
  const i = slideIndex(e, int(args, 'slide'))
  const id = need(str(args, 'shape'), 'the shape')
  e.run({ cmd: 'setActive', index: i })
  const el = topShape(e, i, id, 'format')
  const patch = textPatch(args)
  if (!Object.keys(patch).length) throw new ToolError('Give what to change: font, size, bold, italic, underline, strikethrough, color or align.')
  const from = int(args, 'from')
  const to = int(args, 'to')
  const find = str(args, 'find')
  const isTable = el.type === 'table'
  if (from === undefined && to === undefined && !find) {
    // All its text, as the inspector does with the shape picked.
    pick(e, [id])
    e.run({ cmd: isTable ? 'tableTextStyle' : 'textStyle', patch })
    return { text: 'Formatted.', after: afterShape(e, i, id) }
  }
  const paras = textsOf(e, id)
  if (!paras.length) throw new ToolError(`Shape "${id}" doesn’t hold text.`)
  const a = from ?? 1
  const b = to ?? (from !== undefined ? from : paras.length)
  if (a < 1 || b > paras.length || a > b) throw new ToolError(`Shape "${id}" has ${plural(paras.length, 'paragraph')} (1 to ${paras.length}).`)
  let count = 0
  grouped(e, () => {
    if (find) {
      for (let p = a - 1; p < b; p++) {
        const t = paras[p].text
        const low = t.toLowerCase()
        const q = find.toLowerCase()
        for (let at = low.indexOf(q); at >= 0 && q; at = low.indexOf(q, at + q.length)) {
          selectIn(e, id, [paras[p].path, at], [paras[p].path, at + q.length])
          e.run({ cmd: 'textStyle', patch })
          count++
        }
      }
    } else {
      selectIn(e, id, [paras[a - 1].path, 0], [paras[b - 1].path, paras[b - 1].text.length])
      e.run({ cmd: 'textStyle', patch })
      count = 1
    }
    pick(e, [id])
  })
  if (find && !count) throw new ToolError(`“${find}” isn’t in ${a === 1 && b === paras.length ? `shape "${id}"` : `those paragraphs of shape "${id}"`}.`)
  return { text: find ? `Formatted ${plural(count, 'match', 'matches')}.` : `Formatted ${a === b ? `paragraph ${a}` : `paragraphs ${a}–${b}`}.`, after: afterShape(e, i, id) }
}

async function cropPicture(e: SlidesEngine, args: Args): Promise<Answer> {
  let i = slideIndex(e, int(args, 'slide'))
  const id = need(str(args, 'shape'), 'the shape')
  e.run({ cmd: 'setActive', index: i })
  const el = topShape(e, i, id, 'crop')
  if (el.type !== 'image' || !el.media) throw new ToolError(`Shape "${id}" isn’t a picture.`)
  const fit = str(args, 'fit')
  const [l0, t0, r0, b0] = el.crop ?? [0, 0, 0, 0]
  const { x, y, width: w, height: h } = el
  let crop: [number, number, number, number]
  let to = { x, y, w, h }
  if (fit === 'fill' || fit === 'fit') {
    const size = await naturalSize(e.media, el.media)
    if (!size) throw new ToolError('Writan can’t read that picture’s size, so it can’t fit it. Give left, top, right and bottom instead.')
    // Another call may have made another slide active meanwhile, or someone changed this one.
    i = slideIndex(e, int(args, 'slide'))
    e.run({ cmd: 'setActive', index: i })
    const now = topShape(e, i, id, 'crop')
    if (now.media !== el.media || now.x !== x || now.y !== y || now.width !== w || now.height !== h) throw new ToolError('Someone changed that picture meanwhile. Read the slide again.')
    const A = size.width / size.height
    const F = w / h
    if (fit === 'fill') {
      const keep = str(args, 'keep') ?? 'center'
      if (A > F) {
        const cut = 1 - F / A
        const l = keep === 'left' ? 0 : keep === 'right' ? cut : cut / 2
        crop = [l, 0, cut - l, 0]
      } else {
        const cut = 1 - A / F
        const t = keep === 'top' ? 0 : keep === 'bottom' ? cut : cut / 2
        crop = [0, t, 0, cut - t]
      }
    } else {
      crop = [0, 0, 0, 0]
      to = A > F ? { x, y: y + (h - w / A) / 2, w, h: w / A } : { x: x + (w - h * A) / 2, y, w: h * A, h }
    }
  } else {
    if (fit) throw new ToolError('fit is "fill" or "fit".')
    const side = (k: string, now: number) => {
      const v = num(args, k)
      if (v === undefined) return now
      if (v < 0 || v >= 1) throw new ToolError(`${k} is a share of the picture, from 0 to under 1, like 0.1.`)
      return v
    }
    crop = [side('left', l0), side('top', t0), side('right', r0), side('bottom', b0)]
    if (crop[0] + crop[2] > 0.98 || crop[1] + crop[3] > 0.98) throw new ToolError('That would cut away the whole picture.')
    // The picture stays where it is and the same size; the frame shows a different part of it.
    const iw = w / Math.max(0.01, 1 - l0 - r0)
    const ih = h / Math.max(0.01, 1 - t0 - b0)
    const ix = x - l0 * iw
    const iy = y - t0 * ih
    to = { x: ix + crop[0] * iw, y: iy + crop[1] * ih, w: iw * (1 - crop[0] - crop[2]), h: ih * (1 - crop[1] - crop[3]) }
  }
  e.run({ cmd: 'crop', id: Number(id), left: crop[0], top: crop[1], right: crop[2], bottom: crop[3], ...to })
  pick(e, [id])
  return { text: 'Cropped.', after: afterShape(e, i, id, true) }
}

/** edit_shape's moving, resizing, turning, flipping and restacking, checked; null if there's none. */
function planMove(e: SlidesEngine, id: string, el: Element, args: Args): (() => void) | null {
  const fh = bool(args, 'flip_horizontal')
  const fv = bool(args, 'flip_vertical')
  const layer = str(args, 'layer')
  if (!['x', 'y', 'width', 'height', 'rotation'].some((k) => num(args, k) !== undefined) && fh === undefined && fv === undefined && layer === undefined) return null
  checkSize(args)
  const b = boxFrom(args, el, args.keep_proportions !== false)
  const rot = num(args, 'rotation') ?? el.rotation
  if (layer && !['front', 'forward', 'backward', 'back'].includes(layer)) throw new ToolError('layer is front, forward, backward or back.')
  return () => {
    // Moving alone keeps the size as it is: a line's box has no height, which a new box can't have.
    if (b.w === el.width && b.h === el.height && rot === el.rotation) {
      if (b.x !== el.x || b.y !== el.y) e.run({ cmd: 'moveFrames', ids: [Number(id)], dx: b.x - el.x, dy: b.y - el.y })
    } else e.run({ cmd: 'setBox', id: Number(id), x: b.x, y: b.y, w: b.w, h: b.h, rot })
    pick(e, [id])
    if (fh !== undefined && fh !== el.flipHorizontal) e.run({ cmd: 'flip', horizontal: true })
    if (fv !== undefined && fv !== el.flipVertical) e.run({ cmd: 'flip', horizontal: false })
    if (layer) e.run({ cmd: 'restack', how: layer })
  }
}

/** edit_shape's fill, outline and link, checked; null if there's none. */
function planFormat(e: SlidesEngine, id: string, el: Element, args: Args): (() => void) | null {
  const fill = str(args, 'fill')
  const outline = str(args, 'outline')
  const width = num(args, 'outline_width')
  const link = str(args, 'link')
  if (fill === undefined && outline === undefined && width === undefined && link === undefined) return null
  if ((fill !== undefined || outline !== undefined || width !== undefined) && el.type === 'image') throw new ToolError('A picture has no fill or outline here. Its link can be set.')
  const fillHex = colorArg(args, 'fill', 'The fill')
  const lineHex = colorArg(args, 'outline', 'The outline')
  return () => {
    pick(e, [id])
    if (fill !== undefined) e.run({ cmd: 'fill', hex: fillHex })
    if (outline !== undefined || width !== undefined) {
      const none = lineHex === null
      e.run({ cmd: 'line', hex: lineHex ?? null, width: none ? 0 : (width ?? (outline ? Math.max(1, el.shapeStyle?.strokeWidth ?? 1) : null)) })
    }
    if (link !== undefined) e.run({ cmd: 'link', id: Number(id), url: link.trim() || null })
  }
}

/** edit_shape: everything given, checked before anything changes, as one change. */
function editShape(e: SlidesEngine, args: Args): Answer {
  const i = slideIndex(e, int(args, 'slide'))
  const id = need(str(args, 'shape'), 'the shape')
  e.run({ cmd: 'setActive', index: i })
  const el = topShape(e, i, id, 'change')
  const move = planMove(e, id, el, args)
  const format = planFormat(e, id, el, args)
  if (!move && !format) throw new ToolError('Give what to change: x, y, width, height, rotation, flip_horizontal, flip_vertical, layer, fill, outline, outline_width or link.')
  grouped(e, () => {
    move?.()
    format?.()
  })
  return { text: move ? 'Moved.' : 'Formatted.', after: afterShape(e, i, id, true) }
}

function addShape(e: SlidesEngine, args: Args): Answer {
  const i = slideIndex(e, int(args, 'slide'))
  const kind = need(str(args, 'kind'), 'the kind').trim().toLowerCase()
  const text = str(args, 'text')
  e.run({ cmd: 'setActive', index: i })
  const patch = textPatch(args)
  checkSize(args)
  const fillHex = colorArg(args, 'fill', 'The fill')
  const lineHex = colorArg(args, 'outline', 'The outline')
  const id = grouped(e, () => {
    let id: string
    if (kind === 'text box' || kind === 'textbox' || kind === 'text') id = e.run<string>({ cmd: 'insertTextBox' })
    else if (kind === 'table') {
      const rows = int(args, 'rows') ?? 2
      const cols = int(args, 'columns') ?? 2
      if (rows < 1 || rows > 50 || cols < 1 || cols > 20) throw new ToolError('A table has 1 to 50 rows and 1 to 20 columns.')
      id = e.run<string>({ cmd: 'insertTable', rows, cols })
    } else {
      const preset = Object.hasOwn(SHAPES, kind) ? SHAPES[kind] : undefined
      if (!preset) throw new ToolError(`Writan has no shape “${kind}”. Kinds: text box, ${Object.keys(SHAPES).join(', ')}, table.`)
      id = e.run<string>({ cmd: 'insertShape', preset })
    }
    if (!findHit(e, i, id)) throw new ToolError(`Writan has no shape “${kind}”.`)
    const el = findShape(e, i, id).el
    const b = boxFrom(args, el, false)
    if (num(args, 'x') !== undefined || num(args, 'y') !== undefined || num(args, 'width') !== undefined || num(args, 'height') !== undefined) {
      // A size without a place keeps it in the middle.
      const x = num(args, 'x') ?? (e.width - b.w) / 2
      const y = num(args, 'y') ?? (e.height - b.h) / 2
      e.run({ cmd: 'setBox', id: Number(id), x, y, w: b.w, h: b.h, rot: 0 })
    }
    if (text && kind !== 'table' && kind !== 'line') typeInto(e, id, text)
    pick(e, [id])
    if (Object.keys(patch).length) e.run({ cmd: kind === 'table' ? 'tableTextStyle' : 'textStyle', patch })
    if (fillHex !== undefined) e.run({ cmd: 'fill', hex: fillHex })
    if (lineHex !== undefined) e.run({ cmd: 'line', hex: lineHex, width: lineHex === null ? 0 : null })
    return id
  })
  return { text: `Added shape "${id}" on slide ${i + 1}.`, after: afterShape(e, i, id, true) }
}

async function addPicture(e: SlidesEngine, args: Args, local: boolean): Promise<Answer> {
  slideIndex(e, int(args, 'slide'))
  checkSize(args)
  const got = await pictureBytes(args, local)
  const type = pictureType(got.bytes)
  if (!type) throw new ToolError('That isn’t a picture Writan can put on a slide. Give a PNG, JPEG, GIF, WebP or SVG.')
  const pic = await scaledPicture(got.bytes, type, e.width * 2, e.height * 2)
  // The slide is found once the picture has loaded, as when any edit starts.
  const i = slideIndex(e, int(args, 'slide'))
  e.run({ cmd: 'setActive', index: i })
  const id = grouped(e, () => {
    const id = e.doc.insertImage(pic.bytes, pic.type, pic.width, pic.height, str(args, 'name') ?? got.name)
    e.refresh()
    const el = findShape(e, i, id).el
    if (['x', 'y', 'width', 'height'].some((k) => num(args, k) !== undefined)) {
      const b = boxFrom(args, el, true)
      const x = num(args, 'x') ?? (e.width - b.w) / 2
      const y = num(args, 'y') ?? (e.height - b.h) / 2
      e.run({ cmd: 'setBox', id: Number(id), x, y, w: b.w, h: b.h, rot: 0 })
    }
    pick(e, [id])
    return id
  })
  return { text: `Added picture "${id}" on slide ${i + 1}.`, after: afterShape(e, i, id, true) }
}

function setTableText(e: SlidesEngine, args: Args): Answer {
  const i = slideIndex(e, int(args, 'slide'))
  const id = need(str(args, 'shape'), 'the shape')
  e.run({ cmd: 'setActive', index: i })
  const el = topShape(e, i, id, 'write in')
  if (el.type !== 'table' || !el.table) throw new ToolError(`Shape "${id}" isn’t a table.`)
  const cells = args.cells
  if (!Array.isArray(cells) || !cells.every((row) => Array.isArray(row))) throw new ToolError('Give cells as a list of rows, like [["Dish", "Price"], ["Pad thai", "$12"]].')
  const r0 = (int(args, 'row') ?? 1) - 1
  const c0 = (int(args, 'column') ?? 1) - 1
  if (r0 < 0 || c0 < 0) throw new ToolError('row and column count from 1.')
  const rows = r0 + cells.length
  const cols = c0 + Math.max(0, ...cells.map((row: unknown[]) => row.length))
  if (rows > 50 || cols > 20) throw new ToolError('A table has at most 50 rows and 20 columns.')
  grouped(e, () => {
    pick(e, [id])
    if (rows > el.table!.rows) e.run({ cmd: 'table', op: 'rows', value: String(rows) })
    if (cols > el.table!.columns) e.run({ cmd: 'table', op: 'cols', value: String(cols) })
    cells.forEach((row: unknown[], ri: number) =>
      row.forEach((v, ci) => {
        const r = r0 + ri
        const c = c0 + ci
        const paras = textsOf(e, id).filter((t) => t.path.length >= 4 && t.path[t.path.length - 3] === r && t.path[t.path.length - 2] === c)
        if (!paras.length) return
        const last = paras[paras.length - 1]
        selectIn(e, id, [paras[0].path, 0], [last.path, last.text.length])
        const text = String(v ?? '')
        if (paras.length > 1 || paras[0].text) e.run({ cmd: 'deleteText', word: false })
        typeLines(e, text || '')
      }),
    )
    pick(e, [id])
  })
  return {
    text: `Wrote ${plural(cells.reduce((n: number, row: unknown[]) => n + row.length, 0), 'cell')}.`,
    after: afterShape(e, i, id),
  }
}

/** Runs a PowerPoint tool; `local` says whether the agent is on this computer. */
export async function slidesTool(e: SlidesEngine, name: string, args: Args, local: boolean): Promise<Answer> {
  switch (name) {
    case 'set_shape_text': {
      const i = slideIndex(e, int(args, 'slide'))
      const id = need(str(args, 'shape'), 'the shape')
      e.run({ cmd: 'setActive', index: i })
      topShape(e, i, id, 'write in')
      grouped(e, () => typeInto(e, id, need(str(args, 'text'), 'the text')))
      return { text: 'Done.', after: afterShape(e, i, id) }
    }
    case 'format_text':
      return formatText(e, args)
    case 'edit_shape':
      return editShape(e, args)
    case 'crop_picture':
      return cropPicture(e, args)
    case 'add_shape':
      return addShape(e, args)
    case 'add_picture':
      return addPicture(e, args, local)
    case 'set_table_text':
      return setTableText(e, args)
    case 'delete_shapes': {
      const i = slideIndex(e, int(args, 'slide'))
      const ids = shapeIds(args)
      e.run({ cmd: 'setActive', index: i })
      for (const id of ids) topShape(e, i, id, 'delete')
      pick(e, ids)
      e.run({ cmd: 'delete' })
      return { text: `Deleted ${plural(ids.length, 'shape')}.` }
    }
    case 'copy_shapes': {
      const i = slideIndex(e, int(args, 'slide'))
      const ids = shapeIds(args)
      const to = int(args, 'to_slide') === undefined ? i : slideIndex(e, int(args, 'to_slide'))
      e.run({ cmd: 'setActive', index: i })
      for (const id of ids) topShape(e, i, id, 'copy')
      const made = grouped(e, () => {
        pick(e, ids)
        if (to === i) {
          e.run({ cmd: 'duplicate' })
          return [...e.state.frames]
        }
        const json = e.doc.copyFrames()
        e.run({ cmd: 'setActive', index: to })
        if (!e.run<boolean>({ cmd: 'pasteFrames', json })) throw new ToolError('Those shapes can’t be copied to another slide.')
        return [...e.state.frames]
      })
      return {
        text: `Copied to slide ${to + 1} as ${made.map((m) => `"${m}"`).join(', ')}.`,
        after: () => made.map((m) => afterShape(e, to, m, true)()).join(''),
      }
    }
    case 'group_shapes': {
      const i = slideIndex(e, int(args, 'slide'))
      const ids = shapeIds(args)
      e.run({ cmd: 'setActive', index: i })
      for (const id of ids) topShape(e, i, id, 'group')
      if (ids.length < 2) throw new ToolError('Give at least two shapes to group.')
      pick(e, ids)
      e.run({ cmd: 'group' })
      const id = e.state.frames[0]
      return { text: `Grouped as shape "${id}".`, after: afterShape(e, i, id) }
    }
    case 'ungroup_shapes': {
      const i = slideIndex(e, int(args, 'slide'))
      const id = need(str(args, 'group'), 'the group')
      e.run({ cmd: 'setActive', index: i })
      if (topShape(e, i, id, 'ungroup').type !== 'group') throw new ToolError(`Shape "${id}" isn’t a group.`)
      pick(e, [id])
      e.run({ cmd: 'ungroup' })
      const parts = [...e.state.frames]
      return { text: `Ungrouped into ${parts.map((m) => `"${m}"`).join(', ')}.`, after: () => parts.map((m) => afterShape(e, i, m)()).join('') }
    }
    case 'add_slide': {
      const copy = int(args, 'copy')
      if (copy !== undefined) {
        const given = ['after', 'layout', 'title', 'body'].filter((k) => args[k] !== undefined)
        if (given.length) throw new ToolError(`With copy, leave out ${given.join(', ')}: the copy goes after the slide it copies, as it is. Change it afterwards.`)
        const i = slideIndex(e, copy)
        e.run({ cmd: 'duplicateSlides', indexes: [i] })
        return { text: `Copied slide ${i + 1} as slide ${i + 2}.`, after: () => (i + 1 < e.slides.length ? `\n${slideLines(e, i + 1).join('\n')}` : '') }
      }
      const after = int(args, 'after') === undefined ? e.slides.length - 1 : int(args, 'after') === 0 ? -1 : slideIndex(e, int(args, 'after'))
      const layoutName = str(args, 'layout')
      const layout = layoutName ? e.layouts.find((l) => l.name.toLowerCase() === layoutName.toLowerCase()) : null
      if (layoutName && !layout) throw new ToolError(`No layout “${layoutName}”. Layouts: ${e.layouts.map((l) => l.name).join(', ')}.`)
      grouped(e, () => {
        e.run({ cmd: 'addSlide', after: Math.max(0, after), layout: layout?.path ?? null })
        // "After 0" puts it first: added after the first, then moved to the front.
        if (after < 0) e.run({ cmd: 'moveSlide', from: 1, to: 0 })
        const i = after + 1
        e.run({ cmd: 'setActive', index: i })
        const els = elementsOf(e, i)
        const title = els.find((el) => el.placeholder === 'title' || el.placeholder === 'ctrTitle')
        const body = els.find((el) => el.placeholder === 'body' || el.placeholder === 'subTitle' || el.placeholder === 'obj')
        const t = str(args, 'title')
        const b = str(args, 'body')
        // A layout without the placeholder (Blank) gets a text box for it instead.
        const box = (text: string, y: number, h: number, size: number) => {
          const id = e.run<string>({ cmd: 'insertTextBox' })
          e.run({ cmd: 'setBox', id: Number(id), x: e.width * 0.08, y: e.height * y, w: e.width * 0.84, h: e.height * h, rot: 0 })
          typeInto(e, id, text)
          e.run({ cmd: 'selectAllText' })
          e.run({ cmd: 'textStyle', patch: { fontSize: size } })
        }
        if (t) title ? typeInto(e, title.id, t) : box(t, 0.08, 0.16, 36)
        if (b) body ? typeInto(e, body.id, b) : box(b, 0.3, 0.6, 20)
        e.run({ cmd: 'endEdit' })
      })
      const n = after + 2
      return { text: `Added slide ${n}.`, after: () => (n <= e.slides.length ? `\n${slideLines(e, n - 1).join('\n')}` : '') }
    }
    case 'delete_slide': {
      const i = slideIndex(e, int(args, 'slide'))
      if (e.slides.length === 1) throw new ToolError('That’s the only slide; a presentation needs one.')
      e.run({ cmd: 'deleteSlides', indexes: [i] })
      return { text: `Deleted slide ${i + 1}. The slides after it moved up one.`, after: () => `\n${plural(e.slides.length, 'slide')} now.` }
    }
    case 'edit_slide': {
      let i = slideIndex(e, int(args, 'slide'))
      const layoutName = str(args, 'layout')
      const hidden = bool(args, 'hidden')
      const notes = str(args, 'notes')
      const to = int(args, 'to') === undefined ? undefined : slideIndex(e, int(args, 'to'))
      if (layoutName === undefined && hidden === undefined && notes === undefined && to === undefined) throw new ToolError('Give what to change: layout, hidden, notes or to.')
      const layout = layoutName ? e.layouts.find((l) => l.name.toLowerCase() === layoutName.toLowerCase()) : null
      if (layoutName && !layout) throw new ToolError(`No layout “${layoutName}”. Layouts: ${e.layouts.map((l) => l.name).join(', ')}.`)
      grouped(e, () => {
        e.run({ cmd: 'setActive', index: i })
        if (layout) e.run({ cmd: 'applyLayout', layout: layout.path })
        if (hidden !== undefined && hidden !== e.slides[i].hidden) e.run({ cmd: 'toggleHidden', indexes: [i] })
        if (notes !== undefined) e.run({ cmd: 'notes', index: i, text: notes })
        if (to !== undefined && to !== i) {
          e.run({ cmd: 'moveSlide', from: i, to })
          i = to
        }
      })
      const moved = to !== undefined ? ` It’s slide ${i + 1} now.` : ''
      return { text: `Done.${moved}`, after: () => (i < e.slides.length ? `\n${slideLines(e, i).join('\n')}` : '') }
    }
  }
  throw new ToolError(`${name} isn’t for presentations.`)
}

// ── What changed, for wait_for_changes

export interface SlidesSnapshot {
  slides: { id: string; revision: number; shapes: Map<string, string>; order: string[]; notes: string }[]
}

export function snapshotSlides(e: SlidesEngine): SlidesSnapshot {
  return {
    slides: e.slides.map((s, i) => {
      const shapes = new Map<string, string>()
      const walk = (els: Element[]) => {
        for (const el of els) {
          shapes.set(el.id, shapeLine(el, 0).replace(/^- /, ''))
          if (el.children) walk(el.children)
        }
      }
      const els = elementsOf(e, i)
      walk(els)
      return { id: s.id, revision: s.revision, shapes, order: els.map((el) => el.id), notes: s.notes }
    }),
  }
}

/** What's different between two snapshots, slide by slide and shape by shape. */
export function slidesChanges(a: SlidesSnapshot, b: SlidesSnapshot): string {
  const out: string[] = []
  const before = new Map(a.slides.map((s, i) => [s.id, { s, i }]))
  const after = new Set(b.slides.map((s) => s.id))
  // A slide made in Writan has no part yet, and its id follows its revision: such a slide that
  // changed can't be told from a new one, so it isn't called deleted or new.
  const made = (id: string) => id.startsWith('new-')
  for (const [id, { i }] of before) if (!after.has(id) && !made(id)) out.push(`Slide ${i + 1} (as it was numbered) was deleted.`)
  if (a.slides.length !== b.slides.length) out.push(`There are ${plural(b.slides.length, 'slide')} now, ${a.slides.length} before.`)
  b.slides.forEach((s, i) => {
    const old = before.get(s.id)
    if (!old) {
      out.push(made(s.id) ? `Slide ${i + 1} is new or changed (it was made in Writan, so it can’t be compared); read it again.` : `Slide ${i + 1} is new.`)
      return
    }
    if (old.s.revision === s.revision && old.s.notes === s.notes) return
    const lines: string[] = []
    for (const [sid, line] of s.shapes) {
      const was = old.s.shapes.get(sid)
      if (was === undefined) lines.push(`  - added ${line}`)
      else if (was !== line) lines.push(`  - changed ${line}`)
    }
    for (const [sid, line] of old.s.shapes) if (!s.shapes.has(sid)) lines.push(`  - deleted ${line}`)
    if (old.s.notes !== s.notes) lines.push(`  notes now: ${clip(s.notes, 300) || '(none)'}`)
    const kept = s.order.filter((sid) => old.s.order.includes(sid))
    if (kept.join() !== old.s.order.filter((sid) => s.order.includes(sid)).join()) lines.push(`  restacked, back to front now: ${s.order.map((sid) => `"${sid}"`).join(', ')}`)
    if (old.i !== i) out.push(`Slide ${old.i + 1} moved to ${i + 1}.`)
    if (lines.length) out.push(`Slide ${i + 1}:`, ...lines)
  })
  return out.join('\n')
}
