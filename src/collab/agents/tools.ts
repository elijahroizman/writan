// What an AI agent can do in Writan through MCP (src-tauri/src/collab/mcp.rs): the tools, and
// doing them. An agent works on documents shared from this app. It joins one as a copy of its
// own, with no tab, through the same relay as everyone else (session.ts `followAsAgent`), so its
// changes, caret, name and logo reach everyone like a person's. Its edits are the editor's own
// commands, run on that copy's engine.

import type { SheetsEngine } from '../../engine/sheets/engine'
import type { SlidesEngine } from '../../engine/slides/engine'
import type { WordEngine } from '../../engine/word/engine'
import { stripExtension } from '../../lib/kinds'
import { useWorkspace } from '../../state/workspace'
import { attach, followAsAgent, inviteOf, leaveAsAgent, onOthersChange, useCollab, usePresence, type Opened, type Peer } from '../session'
import { AGENTS, type Agent, type OtherAgent } from './identity'

type Engine = WordEngine | SlidesEngine | SheetsEngine

/** An agent's copy of one shared document. */
interface Copy {
  /** The copy's id in `useCollab` and `usePresence`. */
  id: string
  /** The tab it follows: the document's id for the agent. */
  tab: string
  kind: 'docx' | 'pptx' | 'xlsx'
  engine: Promise<Engine>
}

/** Every connected agent, so their copies leave a document that stops being shared here. */
const sessions = new Set<AgentSession>()

/** One agent connected through MCP, and the documents it has joined. */
export class AgentSession {
  /** Its copies by the tab they follow, joining or joined. */
  readonly copies = new Map<string, Promise<Copy>>()
  /** The copies that have joined, by tab. */
  private readonly joined = new Map<string, Copy>()
  private ended = false
  constructor(readonly who: Agent | OtherAgent) {
    sessions.add(this)
  }

  /** Leaves every document. */
  end() {
    this.ended = true
    sessions.delete(this)
    for (const tab of [...this.copies.keys()]) this.leave(tab)
  }

  leave(tab: string) {
    const c = this.copies.get(tab)
    if (!c) return false
    this.copies.delete(tab)
    this.joined.delete(tab)
    // A join still under way leaves once it's done.
    void c.then(close, () => {})
    return true
  }

  /** The agent's copy of a shared tab's document, joining it first if need be. */
  async copy(tab: string): Promise<{ copy: Copy; engine: Engine }> {
    // Only while it's shared here: someone who stopped sharing or left takes the agent out too.
    if (!documents().some((d) => d.document === tab)) {
      this.leave(tab)
      throw new ToolError(`No shared document “${tab}”. list_documents shows the ones you can work on.`)
    }
    let pending = this.copies.get(tab)
    if (!pending) {
      pending = this.join(tab)
      this.copies.set(tab, pending)
      pending.catch(() => this.copies.get(tab) === pending && this.copies.delete(tab))
    }
    const c = await pending
    this.joined.set(tab, c)
    const engine = await c.engine
    // The relay's welcome is applied when the copy goes live, just after the engine attaches.
    await live(c.id)
    if (this.ended || this.copies.get(tab) !== pending) throw new ToolError('You left the document.')
    return { copy: c, engine }
  }

  /** Whether `c` is still this agent's copy of its document. */
  has(c: Copy): boolean {
    return !this.ended && this.joined.get(c.tab) === c
  }

  private async join(tab: string): Promise<Copy> {
    const invite = inviteOf(tab)
    const doc = documents().find((d) => d.document === tab)
    if (!invite || !doc) throw new ToolError(`No shared document “${tab}”. list_documents shows the ones you can work on.`)
    const who = this.who
    let resolveEngine!: (e: Engine) => void
    let rejectEngine!: (err: unknown) => void
    const engine = new Promise<Engine>((res, rej) => {
      resolveEngine = res
      rejectEngine = rej
    })
    const id = await followAsAgent(invite, { name: who.name, color: who.id ? AGENTS[who.id].color : '#6E56CF', agent: who.id ?? 'ai' }, (copyId, opened) => {
      void openEngine(opened).then((e) => {
        attach(copyId, e)
        resolveEngine(e)
      }, rejectEngine)
    })
    const copy = { id, tab, kind: doc.kind, engine }
    // A copy whose document didn't open leaves again, and the next call joins afresh.
    engine.catch(() => {
      leaveAsAgent(id)
      const now = this.copies.get(tab)
      if (now) void now.then((c) => c === copy && this.copies.delete(tab), () => {})
    })
    return copy
  }
}

/** A copy leaves its document, and its engine goes. */
function close(c: Copy) {
  leaveAsAgent(c.id)
  void c.engine.then((e) => e.dispose(), () => {})
}

// A document that stops being shared here (stopped, left, closed) loses its agents too.
useCollab.subscribe(() => {
  const shared = new Set(documents().map((d) => d.document))
  for (const s of sessions) for (const tab of [...s.copies.keys()]) if (!shared.has(tab)) s.leave(tab)
})

export class ToolError extends Error {}

async function openEngine(doc: Opened): Promise<Engine> {
  if (doc.kind === 'docx') return (await import('../../engine/word/engine')).WordEngine.open(doc.bytes)
  if (doc.kind === 'pptx') return (await import('../../engine/slides/engine')).SlidesEngine.open(doc.bytes)
  if (doc.kind === 'xlsx') return (await import('../../engine/sheets/engine')).SheetsEngine.open(doc.bytes, doc.name)
  throw new ToolError(`Writan can’t open ${doc.name}.`)
}

/** Waits for an agent's copy to have every change so far. */
async function live(id: string) {
  for (let i = 0; i < 200; i++) {
    const s = useCollab.getState()[id]
    if (s?.status === 'live') return
    if (s && s.status !== 'connecting') throw new ToolError(s.message ?? 'The document stopped being shared.')
    await new Promise((r) => setTimeout(r, 25))
  }
  throw new ToolError('Joining the document took too long.')
}

/** The documents shared from this app, as agents see them. */
export function documents(): { document: string; title: string; kind: 'docx' | 'pptx' | 'xlsx'; role: 'sharing' | 'joined' }[] {
  const tabs = useWorkspace.getState().tabs
  const shared = useCollab.getState()
  return tabs.flatMap((t) => {
    const s = shared[t.id]
    if (!s || s.status !== 'live' || !inviteOf(t.id) || (t.kind !== 'docx' && t.kind !== 'pptx' && t.kind !== 'xlsx')) return []
    return [{ document: t.id, title: `${stripExtension(t.title)}.${t.kind}`, kind: t.kind, role: s.role === 'host' ? ('sharing' as const) : ('joined' as const) }]
  })
}

// ── The tools, as MCP lists them

const doc = { type: 'string', description: 'The document, as list_documents gives it.' }

export const TOOLS = [
  {
    name: 'list_documents',
    description: 'The Word documents, presentations and workbooks shared in Writan that you can work on, with who is in each.',
    inputSchema: { type: 'object', properties: {} },
  },
  {
    name: 'read_document',
    description:
      'Reads a shared document, joining it first if you haven’t (people then see you in it). Word: numbered paragraphs, headings marked; use from/to for part of a long one. PowerPoint: each slide’s layout, shapes (by id) with their text, and notes; use slide for one. Excel: a sheet’s cells as A1 references with values and formulas; use sheet and range (like A1:F40) for part.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        from: { type: 'integer', description: 'Word: the first paragraph number to read.' },
        to: { type: 'integer', description: 'Word: the last paragraph number to read.' },
        slide: { type: 'integer', description: 'PowerPoint: one slide’s number.' },
        sheet: { type: 'string', description: 'Excel: the sheet’s name (the first one if left out).' },
        range: { type: 'string', description: 'Excel: the cells to read, like A1:F40.' },
      },
      required: ['document'],
    },
  },
  {
    name: 'where_is_everyone',
    description: 'Who is in a shared document and where: the paragraph and text each person has selected in Word, the slide and shapes in PowerPoint, the sheet and cells in Excel. Use it to work where someone asks, or to stay out of their way.',
    inputSchema: { type: 'object', properties: { document: doc }, required: ['document'] },
  },
  {
    name: 'insert_paragraphs',
    description: 'Word: puts new paragraphs in after paragraph `after` (0 puts them at the start). Each line of `text` is a paragraph. `style` sets their style, like "Heading 2" or "Normal".',
    inputSchema: {
      type: 'object',
      properties: { document: doc, after: { type: 'integer' }, text: { type: 'string' }, style: { type: 'string' } },
      required: ['document', 'after', 'text'],
    },
  },
  {
    name: 'replace_paragraphs',
    description: 'Word: replaces paragraphs `from` to `to` (just `from` if `to` is left out) with `text`, one paragraph per line.',
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
    name: 'replace_text',
    description: 'Word and PowerPoint: replaces every occurrence of `find` with `replace`. Says how many it replaced.',
    inputSchema: {
      type: 'object',
      properties: { document: doc, find: { type: 'string' }, replace: { type: 'string' }, match_case: { type: 'boolean', description: 'Word only.' } },
      required: ['document', 'find', 'replace'],
    },
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
    name: 'set_shape_text',
    description: 'PowerPoint: replaces the text of shape `shape` (its id from read_document) on slide `slide`. Each line is a paragraph.',
    inputSchema: {
      type: 'object',
      properties: { document: doc, slide: { type: 'integer' }, shape: { type: 'string' }, text: { type: 'string' } },
      required: ['document', 'slide', 'shape', 'text'],
    },
  },
  {
    name: 'add_slide',
    description: 'PowerPoint: adds a slide after slide `after` (at the end if left out), with a layout by name if given, and puts `title` and `body` in its placeholders. Says the new slide’s number.',
    inputSchema: {
      type: 'object',
      properties: { document: doc, after: { type: 'integer' }, layout: { type: 'string' }, title: { type: 'string' }, body: { type: 'string' } },
      required: ['document'],
    },
  },
  {
    name: 'delete_slide',
    description: 'PowerPoint: deletes slide `slide`.',
    inputSchema: { type: 'object', properties: { document: doc, slide: { type: 'integer' } }, required: ['document', 'slide'] },
  },
  {
    name: 'set_notes',
    description: 'PowerPoint: replaces slide `slide`’s speaker notes.',
    inputSchema: { type: 'object', properties: { document: doc, slide: { type: 'integer' }, text: { type: 'string' } }, required: ['document', 'slide', 'text'] },
  },
  {
    name: 'write_cells',
    description: 'Excel: writes cells, as `{ "A1": "Total", "B1": "=SUM(B2:B9)", "C1": "42" }`: text, numbers, dates or formulas, as typed in a cell.',
    inputSchema: {
      type: 'object',
      properties: { document: doc, sheet: { type: 'string', description: 'The first sheet if left out.' }, cells: { type: 'object', additionalProperties: { type: 'string' } } },
      required: ['document', 'cells'],
    },
  },
  {
    name: 'undo',
    description: 'Takes back your last change in a document (only yours, never someone else’s). In a workbook, that’s the last cell you wrote.',
    inputSchema: { type: 'object', properties: { document: doc }, required: ['document'] },
  },
  {
    name: 'wait_for_changes',
    description: 'Waits until someone else changes the document, up to `seconds` (60 at most), and says who. Read the document again afterwards.',
    inputSchema: { type: 'object', properties: { document: doc, seconds: { type: 'number' } }, required: ['document'] },
  },
  {
    name: 'leave_document',
    description: 'Leaves a document, so people stop seeing you in it.',
    inputSchema: { type: 'object', properties: { document: doc }, required: ['document'] },
  },
]

// ── Doing them

type Args = Record<string, unknown>

const str = (a: Args, k: string) => (typeof a[k] === 'string' ? (a[k] as string) : undefined)
const int = (a: Args, k: string) => (typeof a[k] === 'number' && Number.isFinite(a[k]) ? Math.trunc(a[k] as number) : undefined)
function need<T>(v: T | undefined, what: string): T {
  if (v === undefined) throw new ToolError(`Give ${what}.`)
  return v
}

/** Runs a tool for an agent; returns its text. */
export async function runTool(session: AgentSession, name: string, args: Args): Promise<string> {
  if (name === 'list_documents') return listDocuments()
  const tab = need(str(args, 'document'), 'the document')
  if (name === 'leave_document') return session.leave(tab) ? 'Left.' : 'You weren’t in it.'
  const { copy, engine } = await session.copy(tab)
  switch (name) {
    case 'read_document':
      return copy.kind === 'docx' ? readWord(engine as WordEngine, args) : copy.kind === 'pptx' ? readSlides(engine as SlidesEngine, args) : readSheets(engine as SheetsEngine, args)
    case 'where_is_everyone':
      return whereIsEveryone(copy, engine)
    case 'undo': {
      const e = engine as { run(c: { cmd: string }): unknown; state: { canUndo: boolean } }
      if (!e.state.canUndo) return 'Nothing of yours to undo.'
      e.run({ cmd: 'undo' })
      return `Undone.${await delivered(session, copy, engine)}`
    }
    case 'wait_for_changes':
      return waitForChanges(copy, Math.min(60, Math.max(1, Number(args.seconds) || 30)))
    case 'replace_text':
      if (copy.kind === 'xlsx') throw new ToolError('In a workbook, use write_cells.')
      return `${replaceText(engine as WordEngine | SlidesEngine, need(str(args, 'find'), 'find'), str(args, 'replace') ?? '', args.match_case === true)}${await delivered(session, copy, engine)}`
  }
  const text = copy.kind === 'docx' ? wordTool(engine as WordEngine, name, args) : copy.kind === 'pptx' ? slidesTool(engine as SlidesEngine, name, args) : sheetsTool(engine as SheetsEngine, name, args)
  return `${text}${await delivered(session, copy, engine)}`
}

/**
 * Waits for the relay to put the agent's changes in order with everyone's, so the answer is
 * about the shared document. A change that met someone else's at the same spot can still lose;
 * then reading again shows theirs.
 */
async function delivered(session: AgentSession, copy: Copy, engine: Engine): Promise<string> {
  // A copy that left meanwhile has no document to ask any more.
  const here = () => session.has(copy) && useCollab.getState()[copy.id]?.status === 'live'
  for (let i = 0; i < 100 && here() && engine.doc.collabPending(); i++) await new Promise((r) => setTimeout(r, 50))
  if (!here()) return ' (Your connection to the document ended before it reached the others.)'
  return engine.doc.collabPending() ? ' (Still on its way to the others.)' : ''
}

function peopleIn(id: string): Peer[] {
  const s = useCollab.getState()[id]
  return s ? [...(s.you ? [s.you] : []), ...s.peers] : []
}

function listDocuments(): string {
  const docs = documents()
  if (!docs.length) return 'Nothing is shared in Writan right now. Ask the person to open a Word document, presentation or workbook and share it (the people button at the top right).'
  return docs
    .map((d) => {
      const people = peopleIn(d.document).map((p) => (p.agent ? `${p.name} (AI)` : p.name))
      return `- ${d.title}  document: "${d.document}"  (${d.role === 'sharing' ? 'shared from here' : 'joined from here'}; in it: ${people.join(', ') || 'nobody yet'})`
    })
    .join('\n')
}

async function waitForChanges(copy: Copy, seconds: number): Promise<string> {
  const who = await new Promise<string | null>((resolve) => {
    const timer = window.setTimeout(() => {
      stop()
      resolve(null)
    }, seconds * 1000)
    const stop = onOthersChange(copy.id, (client) => {
      window.clearTimeout(timer)
      stop()
      resolve(client)
    })
  })
  if (!who) return `Nobody changed it in ${seconds} seconds.`
  const peer = useCollab.getState()[copy.id]?.peers.find((p) => p.client === who)
  return `${peer?.name ?? 'Someone'} changed it.`
}

// ── Word

interface Para {
  path: number[]
  text: string
}

/** Whether two paragraphs are in the same list of blocks (the body, or one table cell). */
const sameList = (a: number[], b: number[]) => a.length === b.length && a.slice(0, -1).every((x, i) => x === b[i])
/** Whether paragraph `b` comes straight after `a` in the same list, with no other block between. */
const nextTo = (a: number[], b: number[]) => sameList(a, b) && b[b.length - 1] === a[a.length - 1] + 1

const paragraphsOf = (e: WordEngine) => JSON.parse(e.doc.paragraphs()) as Para[]
const samePath = (a: number[], b: number[]) => a.length === b.length && a.every((x, i) => x === b[i])

function readWord(e: WordEngine, args: Args): string {
  const paras = paragraphsOf(e)
  const headings = JSON.parse(e.doc.outline()) as { text: string; depth: number; path: number[] }[]
  const from = Math.max(1, int(args, 'from') ?? 1)
  const to = Math.min(paras.length, int(args, 'to') ?? paras.length)
  const lines = [`${paras.length} paragraphs. Numbers are what the editing tools take.`]
  for (let i = from; i <= to; i++) {
    const p = paras[i - 1]
    const h = headings.find((x) => samePath(x.path, p.path))
    const where = p.path.length > 1 ? ' (in a table)' : ''
    // The outline counts levels from 0.
    lines.push(`[${i}]${h ? ` (Heading ${h.depth + 1})` : ''}${where} ${p.text.replace(/\uFFFC/g, '[picture]')}`)
  }
  return lines.join('\n')
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

/** Runs commands as one change, which undo takes back at once. */
function grouped(e: { run(c: { cmd: string }): unknown }, f: () => void) {
  e.run({ cmd: 'beginGroup' })
  try {
    f()
  } finally {
    e.run({ cmd: 'endGroup' })
  }
}

function wordTool(e: WordEngine, name: string, args: Args): string {
  const paras = paragraphsOf(e)
  switch (name) {
    case 'insert_paragraphs': {
      const after = need(int(args, 'after'), 'after')
      const text = need(str(args, 'text'), 'the text').replace(/\r\n?/g, '\n')
      const lines = text.split('\n').length
      const style = str(args, 'style')
      const id = style ? styleId(e, style) : null
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
          const last = now[after + lines - 1]
          if (first && last) {
            select(e, [first.path, 0], [last.path, last.text.length])
            e.run({ cmd: 'style', id })
          }
        }
      })
      return `Put in ${lines} paragraph${lines === 1 ? '' : 's'} as ${after + 1}${lines > 1 ? `–${after + lines}` : ''}.`
    }
    case 'replace_paragraphs': {
      const from = para(paras, int(args, 'from'))
      const toN = int(args, 'to') ?? int(args, 'from')!
      const to = para(paras, toN)
      select(e, [from.path, 0], [to.path, to.text.length])
      e.run({ cmd: 'paste', text: need(str(args, 'text'), 'the text').replace(/\r\n?/g, '\n'), json: null })
      return 'Replaced.'
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
      const deleted = e.run<boolean>({ cmd: 'deleteBlocks', container, from: from.path[from.path.length - 1], to: to.path[to.path.length - 1] })
      if (!deleted) {
        // They're all there is in that list (a table cell, or the whole body): their text goes
        // and one empty paragraph stays.
        select(e, [from.path, 0], [to.path, to.text.length])
        e.run({ cmd: 'delete', word: false })
      }
      return `Deleted ${toN - fromN + 1} paragraph${toN === fromN ? '' : 's'}.`
    }
    case 'set_style': {
      const from = para(paras, int(args, 'from'))
      const to = para(paras, int(args, 'to') ?? int(args, 'from'))
      const id = styleId(e, need(str(args, 'style'), 'the style'))
      select(e, [from.path, 0], [to.path, to.text.length])
      e.run({ cmd: 'style', id })
      return 'Done.'
    }
  }
  throw new ToolError(`${name} isn’t for Word documents.`)
}

function replaceText(e: WordEngine | SlidesEngine, find: string, by: string, matchCase: boolean): string {
  if (!find) throw new ToolError('Give the text to find.')
  if ('sizes' in e) {
    const { count } = e.run<{ count: number }>({ cmd: 'find', query: find, matchCase, wholeWord: false })
    if (!count) return `“${find}” isn’t in the document.`
    e.run({ cmd: 'replaceAll', text: by })
    e.run({ cmd: 'find', query: '', matchCase: false, wholeWord: false })
    return `Replaced ${count}.`
  }
  // Presentations find text whatever its case.
  if (matchCase) throw new ToolError('In a presentation, replace_text can’t match case. Leave match_case out, or use set_shape_text.')
  const found = e.run<{ slide: number; id: string }[]>({ cmd: 'find', query: find })
  if (!found.length) return `“${find}” isn’t in the presentation.`
  const textOf = (m: { slide: number; id: string }) => elementsOf(e, m.slide).find((el) => el.id === m.id)?.text ?? ''
  const before = found.map(textOf)
  e.run({ cmd: 'replaceAll', text: by })
  // Each shape's text should now read as if every match were replaced. Where it doesn't, some
  // of it had formatting changing partway through and was left.
  // The same matching as the presentation's own replace: the query trimmed, any case.
  const every = new RegExp(find.trim().replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi')
  const left = found.filter((m, i) => textOf(m) !== before[i].replace(every, () => by)).length
  return left
    ? `Replaced in ${found.length - left} shape${found.length - left === 1 ? '' : 's'}. In ${left} more, some of it has formatting changing partway through and wasn’t replaced; use set_shape_text there.`
    : `Replaced in ${found.length} shape${found.length === 1 ? '' : 's'}.`
}

// ── PowerPoint

interface Element {
  id: string
  type: string
  name: string
  text: string
  placeholder?: string
  children?: Element[]
}

const elementsOf = (e: SlidesEngine, i: number) => JSON.parse(e.doc.elements(i)) as Element[]

function readSlides(e: SlidesEngine, args: Args): string {
  const one = int(args, 'slide')
  const out = [`${e.slides.length} slides. Layouts: ${e.layouts.map((l) => l.name).join(', ')}.`]
  e.slides.forEach((s, i) => {
    if (one !== undefined && one !== i + 1) return
    out.push(`\nSlide ${i + 1}${s.layoutName ? ` (${s.layoutName})` : ''}${s.hidden ? ' (hidden)' : ''}`)
    const shapes = (els: Element[], depth: number) => {
      for (const el of els) {
        const label = `${'  '.repeat(depth)}- shape "${el.id}" ${el.type}${el.placeholder ? ` placeholder:${el.placeholder}` : ''} “${el.name}”`
        out.push(el.text ? `${label}: ${el.text.replace(/\n/g, ' / ')}` : label)
        if (el.children) shapes(el.children, depth + 1)
      }
    }
    shapes(elementsOf(e, i), 1)
    if (s.notes) out.push(`  notes: ${s.notes.replace(/\n/g, ' / ')}`)
  })
  return out.join('\n')
}

function slideIndex(e: SlidesEngine, n: number | undefined): number {
  if (n === undefined || n < 1 || n > e.slides.length) throw new ToolError(`There’s no slide ${n ?? ''}; the presentation has ${e.slides.length} (1 to ${e.slides.length}).`)
  return n - 1
}

/** Types `text` into a shape on the active slide, replacing what's there. */
function typeInto(e: SlidesEngine, id: string, text: string) {
  e.run({ cmd: 'beginEdit', id: Number(id), x: null, y: null })
  if (e.state.editing !== id) throw new ToolError(`Shape "${id}" doesn’t hold text.`)
  e.run({ cmd: 'selectAllText' })
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

function slidesTool(e: SlidesEngine, name: string, args: Args): string {
  switch (name) {
    case 'set_shape_text': {
      const i = slideIndex(e, int(args, 'slide'))
      const id = need(str(args, 'shape'), 'the shape')
      e.run({ cmd: 'setActive', index: i })
      if (!elementsOf(e, i).some((el) => el.id === id)) throw new ToolError(`Slide ${i + 1} has no shape "${id}". read_document lists them.`)
      grouped(e, () => typeInto(e, id, need(str(args, 'text'), 'the text')))
      return 'Done.'
    }
    case 'add_slide': {
      const after = int(args, 'after') === undefined ? e.slides.length - 1 : slideIndex(e, int(args, 'after'))
      const layoutName = str(args, 'layout')
      const layout = layoutName ? e.layouts.find((l) => l.name.toLowerCase() === layoutName.toLowerCase()) : null
      if (layoutName && !layout) throw new ToolError(`No layout “${layoutName}”. Layouts: ${e.layouts.map((l) => l.name).join(', ')}.`)
      grouped(e, () => {
        e.run({ cmd: 'addSlide', after, layout: layout?.path ?? null })
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
      return `Added slide ${after + 2}.`
    }
    case 'delete_slide': {
      const i = slideIndex(e, int(args, 'slide'))
      e.run({ cmd: 'deleteSlides', indexes: [i] })
      return 'Deleted.'
    }
    case 'set_notes': {
      const i = slideIndex(e, int(args, 'slide'))
      e.run({ cmd: 'notes', index: i, text: need(str(args, 'text'), 'the text') })
      return 'Done.'
    }
  }
  throw new ToolError(`${name} isn’t for presentations.`)
}

// ── Excel

/** `B3` as [row, column], from 0. */
function cellRef(ref: string): [number, number] {
  const m = /^\$?([A-Za-z]{1,3})\$?(\d{1,7})$/.exec(ref.trim())
  if (!m) throw new ToolError(`“${ref}” isn’t a cell like B3.`)
  let col = 0
  for (const ch of m[1].toUpperCase()) col = col * 26 + (ch.charCodeAt(0) - 64)
  return [Number(m[2]) - 1, col - 1]
}

function columnName(c: number) {
  let s = ''
  for (let n = c + 1; n > 0; n = Math.floor((n - 1) / 26)) s = String.fromCharCode(65 + ((n - 1) % 26)) + s
  return s
}

function sheetIndex(e: SheetsEngine, name: string | undefined): number {
  if (name === undefined) return e.sheets.findIndex((s) => !s.hidden)
  const i = e.sheets.findIndex((s) => s.name.toLowerCase() === name.toLowerCase())
  if (i < 0) throw new ToolError(`No sheet “${name}”. Sheets: ${e.sheets.map((s) => s.name).join(', ')}.`)
  return i
}

function readSheets(e: SheetsEngine, args: Args): string {
  const sheet = sheetIndex(e, str(args, 'sheet'))
  const g = e.geometryOf(sheet).info
  let [r0, c0, r1, c1] = [0, 0, Math.min(g.content[0], 200) - 1, Math.min(g.content[1], 52) - 1]
  const range = str(args, 'range')
  if (range) {
    const [a, b = a] = range.split(':')
    ;[r0, c0] = cellRef(a)
    ;[r1, c1] = cellRef(b)
  }
  if ((Math.abs(r1 - r0) + 1) * (Math.abs(c1 - c0) + 1) > 20_000) throw new ToolError('That range is too big to read at once. Read up to 20,000 cells at a time, like A1:Z500.')
  const out = [`Sheets: ${e.sheets.map((s) => s.name).join(', ')}. Reading ${e.sheets[sheet].name}!${columnName(c0)}${r0 + 1}:${columnName(Math.max(c0, c1))}${Math.max(r0, r1) + 1}.`]
  for (let r = r0; r <= r1; r++) {
    const cells: string[] = []
    for (let c = c0; c <= c1; c++) {
      const cell = JSON.parse(e.doc.cell(sheet, r, c)) as { input: string; display: string }
      if (!cell.input) continue
      cells.push(cell.input.startsWith('=') ? `${columnName(c)}${r + 1}: ${cell.input} → ${cell.display}` : `${columnName(c)}${r + 1}: ${cell.display}`)
    }
    if (cells.length) out.push(cells.join(' | '))
  }
  if (out.length === 1) out.push('(empty)')
  return out.join('\n')
}

function sheetsTool(e: SheetsEngine, name: string, args: Args): string {
  if (name !== 'write_cells') throw new ToolError(`${name} isn’t for workbooks.`)
  const sheet = sheetIndex(e, str(args, 'sheet'))
  const cells = args.cells as Record<string, unknown> | undefined
  if (!cells || typeof cells !== 'object') throw new ToolError('Give cells, like { "A1": "Total" }.')
  const entries = Object.entries(cells).map(([ref, v]) => [cellRef(ref), String(v ?? '')] as const)
  if (!entries.length) return 'Nothing to write.'
  // Each cell is its own change (workbooks have no grouping), so undo takes back the last one.
  for (const [[row, col], text] of entries) e.run({ cmd: 'input', sheet, row, col, text })
  // People see the agent where it wrote.
  const rows = entries.map(([[r]]) => r)
  const cols = entries.map(([[, c]]) => c)
  e.run({ cmd: 'select', sheet, anchor: [Math.min(...rows), Math.min(...cols)], focus: [Math.max(...rows), Math.max(...cols)] })
  return `Wrote ${entries.length} cell${entries.length === 1 ? '' : 's'}.`
}

// ── Where everyone is

function whereIsEveryone(copy: Copy, engine: Engine): string {
  const share = useCollab.getState()[copy.id]
  const places = usePresence.getState()[copy.id] ?? {}
  const lines: string[] = []
  for (const p of share?.peers ?? []) {
    const at = places[p.client]?.at as Record<string, unknown> | undefined
    const who = `${p.name}${p.agent ? ' (AI)' : ''}${p.host ? ' (shared it)' : ''}`
    lines.push(`- ${who}: ${at ? describe(copy.kind, engine, at) : 'hasn’t moved yet'}`)
  }
  return lines.length ? lines.join('\n') : 'Nobody else is in it.'
}

function describe(kind: Copy['kind'], engine: Engine, at: Record<string, unknown>): string {
  if (kind === 'docx') {
    if (at.s !== 'b') return 'in a header, footer or note'
    const e = engine as WordEngine
    const paras = paragraphsOf(e)
    const [ap, ao] = at.a as [number[], number]
    const [fp, fo] = at.f as [number[], number]
    const ai = paras.findIndex((x) => samePath(x.path, ap))
    const fi = paras.findIndex((x) => samePath(x.path, fp))
    if (ai < 0 || fi < 0) return 'somewhere this copy hasn’t caught up with'
    if (ai === fi && ao === fo) return `paragraph ${fi + 1}`
    const [lo, hi] = ai <= fi ? [ai, fi] : [fi, ai]
    // Offsets are bytes of UTF-8; the selected text is shown when it's within one paragraph.
    const bytes = new TextEncoder().encode(paras[ai].text)
    const picked = ai === fi ? new TextDecoder().decode(bytes.slice(Math.min(ao, fo), Math.max(ao, fo))) : null
    return lo === hi ? `paragraph ${lo + 1}, selecting “${picked}”` : `paragraphs ${lo + 1}–${hi + 1} selected`
  }
  if (kind === 'pptx') {
    const e = engine as SlidesEngine
    const i = e.slides.findIndex((s) => s.id === at.slide)
    if (i < 0) return 'on a slide this copy hasn’t caught up with'
    const ids = ((at.frames as number[] | undefined) ?? []).map(String)
    const names = elementsOf(e, i).filter((el) => ids.includes(el.id)).map((el) => `"${el.id}" “${el.name}”`)
    const typing = at.e ? ', typing' : ''
    return `slide ${i + 1}${names.length ? `, shape ${names.join(', ')}${typing}` : ''}`
  }
  const [ar, ac] = at.a as [number, number]
  const [fr, fc] = at.f as [number, number]
  const a = `${columnName(Math.min(ac, fc))}${Math.min(ar, fr) + 1}`
  const b = `${columnName(Math.max(ac, fc))}${Math.max(ar, fr) + 1}`
  return `sheet ${at.sheet}, ${a === b ? a : `${a}:${b}`}`
}
