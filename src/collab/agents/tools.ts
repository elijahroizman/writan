// What an AI agent can do in Writan through MCP (src-tauri/src/collab/mcp.rs): the tools, and
// doing them. An agent works on the Word documents, presentations and workbooks open in this app,
// shared or not. It joins one as a copy of its own, with no tab, through the same relay as
// everyone else (session.ts `followAsAgent`), so its changes, caret, name and logo reach everyone
// like a person's. A document that isn't shared is opened to this computer's agents only
// (`openToAgents`). Its edits are the editor's own commands, run on that copy's engine.

import type { SheetsEngine } from '../../engine/sheets/engine'
import type { SlidesEngine } from '../../engine/slides/engine'
import type { WordEngine } from '../../engine/word/engine'
import { stripExtension } from '../../lib/kinds'
import { useWorkspace } from '../../state/workspace'
import { appliedSeq, attach, chatOf, clientOf, comings, followAsAgent, inviteOf, keeperOf, leaveAsAgent, nameOf, onApplied, onComing, onOthersChange, openToAgents, SHAREABLE, unread, useCollab, usePresence, type Opened, type Peer } from '../session'
import { type Answer, type Args, BOX, bool, doc, findTool, HIGHLIGHTS, need, num, paraRef, plural, str, TEXT_FORMAT, type Tool, ToolError } from './common'
import { FILE_TOOLS, openDocument, saveDocument } from './files'
import { AGENTS, type Agent, type OtherAgent } from './identity'
import { ago, MAX_WAIT, MESSAGE_TOOL_NAMES, MESSAGE_TOOLS, messagesOn, newMessages, nextMessage, OFF, readMessages, sendMessage } from './messages'
import { describeSheetsPlace, findInSheets, readSheets, replaceInSheets, SHEETS_TOOLS, sheetsTool, viewSheet } from './sheets'
import { findInSlides, readSlides, SLIDES_TOOLS, slidesChanges, slidesTool, snapshotSlides, viewSlide } from './slides'
import { runThemeTool, THEME_TOOLS } from './theme'
import { findInWord, History, readWord, snapshotWord, viewPage, WORD_TOOLS, wordChanges, wordTool } from './word'

export { ToolError } from './common'

type Engine = WordEngine | SlidesEngine | SheetsEngine

/** An agent's copy of one shared document. */
interface Copy {
  /** The copy's id in `useCollab` and `usePresence`. */
  id: string
  /** The tab it follows: the document's id for the agent. */
  tab: string
  kind: 'docx' | 'pptx' | 'xlsx'
  engine: Promise<Engine>
  /** In a Word document, each paragraph's past: who wrote it, when, and what its id was. */
  history?: History
  /** Whether the agent has read it through this copy yet. */
  read?: boolean
}

/** Each Word copy's history, by its engine, from before the relay's changes are applied. */
const histories = new WeakMap<WordEngine, History>()

/** How long a copy spends working out who wrote what before it joined; the rest counts as there before. */
const REPLAY_BUDGET = 2000

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

  /** The agent's copy of a tab's document, joining it first if need be. */
  async copy(tab: string): Promise<{ copy: Copy; engine: Engine }> {
    // Only while it's open here: closing the tab takes the agent out too.
    if (!documents().some((d) => d.document === tab)) {
      this.leave(tab)
      throw new ToolError(`No document “${tab}”. list_documents shows the ones you can work on.`)
    }
    // A copy whose session ended (sharing stopped) joins again.
    const was = this.joined.get(tab)
    if (was && !alive(was.id)) this.leave(tab)
    let pending = this.copies.get(tab)
    if (!pending) {
      pending = this.join(tab)
      this.copies.set(tab, pending)
      pending.catch(() => this.copies.get(tab) === pending && this.copies.delete(tab))
    }
    const c = await pending
    // Left (and maybe joined again) meanwhile: this copy is on its way out.
    if (this.ended || this.copies.get(tab) !== pending) throw new ToolError('You left the document.')
    this.joined.set(tab, c)
    const engine = await c.engine
    // The relay's welcome is applied when the copy goes live, just after the engine attaches.
    await live(c.id)
    if (this.ended || this.copies.get(tab) !== pending) throw new ToolError('You left the document.')
    if (c.kind === 'docx') c.history ??= histories.get(engine as WordEngine)
    return { copy: c, engine }
  }

  /** Leaves documents that closed, and copies whose session ended. */
  tidy(open: Set<string>) {
    for (const tab of [...this.copies.keys()]) {
      const c = this.joined.get(tab)
      if (!open.has(tab) || (c && !alive(c.id))) this.leave(tab)
    }
  }

  /** This agent's copy of a tab's document, if it has joined it. */
  joinedCopy(tab: string): Copy | undefined {
    return this.joined.get(tab)
  }

  /** Whether `c` is still this agent's copy of its document. */
  has(c: Copy): boolean {
    return !this.ended && this.joined.get(c.tab) === c
  }

  private async join(tab: string): Promise<Copy> {
    const doc = documents().find((d) => d.document === tab)
    if (!doc) throw new ToolError(`No document “${tab}”. list_documents shows the ones you can work on.`)
    if (doc.role === 'open') await openToAgents(tab).catch((err) => Promise.reject(new ToolError(`Couldn’t open “${doc.title}” to you: ${String((err as Error)?.message ?? err)}`)))
    const who = this.who
    let resolveEngine!: (e: Engine) => void
    let rejectEngine!: (err: unknown) => void
    const engine = new Promise<Engine>((res, rej) => {
      resolveEngine = res
      rejectEngine = rej
    })
    const id = await followAsAgent(tab, { name: who.name, color: who.id ? AGENTS[who.id].color : '#6E56CF', agent: who.id ?? 'ai', listens: messagesOn() }, (copyId, opened) => {
      void openEngine(opened).then((e) => {
        // A Word document's history starts from the bytes everyone opened, before the changes
        // since, which attaching applies, so it knows who wrote what before the agent came.
        if (opened.kind === 'docx') histories.set(e as WordEngine, follow(copyId, tab, e as WordEngine))
        attach(copyId, e)
        resolveEngine(e)
      }, rejectEngine)
    }).catch((err) => Promise.reject(err instanceof ToolError ? err : new ToolError(String((err as Error)?.message ?? err))))
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

/** Keeps track of each paragraph's past in an agent's copy `id` of a Word document, following tab `tab`. */
function follow(id: string, tab: string, e: WordEngine): History {
  const history = new History(e, id, clientOf(tab), (client) => nameOf(id, client))
  // The changes from before the agent joined come at once. On a long session's document, past
  // the budget the rest count as one change by someone unknown.
  let until = 0
  let skipped = false
  history.stop = onApplied(id, (client, seq, left) => {
    if (left > 0) {
      until ||= Date.now() + REPLAY_BUDGET
      if (Date.now() > until) {
        skipped = true
        return
      }
    }
    history.saw(skipped ? null : client, seq)
    if (!left) {
      until = 0
      skipped = false
    }
  })
  return history
}

/** A copy leaves its document, and its engine goes. */
function close(c: Copy) {
  c.history?.stop()
  leaveAsAgent(c.id)
  void c.engine.then((e) => e.dispose(), () => {})
}

/** Whether an agent's copy still follows its document. */
function alive(id: string) {
  const s = useCollab.getState()[id]
  return s?.status === 'live' || s?.status === 'connecting'
}

// A document that closes, or whose session ends (sharing stopped), loses its agents' copies; a
// copy of one that's still open joins again on the agent's next call.
const tidy = () => {
  const open = new Set(documents().map((d) => d.document))
  for (const s of sessions) s.tidy(open)
}
useCollab.subscribe(tidy)
useWorkspace.subscribe((state, prev) => state.tabs !== prev.tabs && tidy())

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

/**
 * The documents agents can work on: every Word document, presentation and workbook open in this
 * app, whether it's shared from here (`sharing`), someone else's joined from here (`joined`), or
 * not shared (`open`, which joining opens to this computer's agents, loading its editor if nobody
 * has looked at the tab yet). A document joined from a host that can't let agents in has no way
 * in.
 */
export function documents(): { document: string; title: string; kind: 'docx' | 'pptx' | 'xlsx'; role: 'sharing' | 'joined' | 'open' }[] {
  const tabs = useWorkspace.getState().tabs
  const shared = useCollab.getState()
  return tabs.flatMap((t) => {
    if (t.kind !== 'docx' && t.kind !== 'pptx' && t.kind !== 'xlsx') return []
    const s = shared[t.id]
    const on = s && (s.status === 'live' || s.status === 'connecting')
    let role: 'sharing' | 'joined' | 'open'
    if (on && s.role === 'guest') {
      if (!inviteOf(t.id)) return []
      role = 'joined'
    } else if (on) role = s.private ? 'open' : 'sharing'
    else {
      if (!SHAREABLE.has(t.kind) || keeperOf(t.id) !== null) return []
      role = 'open'
    }
    return [{ document: t.id, title: `${stripExtension(t.title)}.${t.kind}`, kind: t.kind, role }]
  })
}

// ── The tools, as MCP lists them

const SHARED_TOOLS: Tool[] = [
  {
    name: 'list_documents',
    description:
      'Lists the documents open in Writan that you can work on, with their ids and who is in each. With `document`, joins it and says where each person is.',
    inputSchema: { type: 'object', properties: { document: { type: 'string', description: 'Joins it and says where everyone is.' } } },
  },
  {
    name: 'read_document',
    description: 'Reads a document, joining it on the first call.\nWord: paragraphs as [number #id], with style and author. `from` and `to` read part; `since` returns only what changed after a version.\nPowerPoint: each slide’s layout, its shapes back to front with id, position, size, look and text, and its notes.\nExcel: a sheet’s cells with values and formulas, and its charts.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        from: paraRef('Word: the first'),
        to: paraRef('Word: the last'),
        since: { type: 'integer', description: 'Word: a version from an earlier answer.' },
        slide: { type: 'integer', description: 'PowerPoint: one slide.' },
        sheet: { type: 'string', description: 'Excel. Default: the first sheet.' },
        range: { type: 'string', description: 'Excel: cells like A1:F40.' },
      },
      required: ['document'],
    },
  },
  {
    name: 'view_document',
    description:
      'Returns an image of a Word page, PowerPoint slide or Excel sheet as people see it, with shape ids and chart numbers labeled. Use it to check layout, like #### in a column that’s too narrow.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        page: { type: 'integer', description: 'Word. Default 1.' },
        slide: { type: 'integer', description: 'PowerPoint. Required.' },
        shape: { type: 'string', description: 'PowerPoint: only this shape, closer up.' },
        sheet: { type: 'string', description: 'Excel. Default: the first sheet.' },
        range: { type: 'string', description: 'Excel: these cells, like A1:F40.' },
        chart: { type: 'integer', description: 'Excel: only this chart, closer up.' },
        labels: { type: 'boolean', description: 'Default true.' },
        width: { type: 'integer', description: 'In pixels, 200 to 2000.' },
      },
      required: ['document'],
    },
  },
  findTool,
  {
    name: 'replace_text',
    description:
      'Replaces every `find` with `replace` and says how many. Ignores case, except in Word with `match_case`. In Excel, formulas are left alone.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        find: { type: 'string', description: 'Text to replace.' },
        replace: { type: 'string', description: '"" deletes it.' },
        match_case: { type: 'boolean', description: 'Word only.' },
      },
      required: ['document', 'find', 'replace'],
    },
  },
  {
    name: 'format_text',
    description: 'Word and PowerPoint: formats text. Give only what changes.\nWord: paragraphs `from` to `to`, or only `find` in them.\nPowerPoint: all of shape `shape`, its paragraphs `from` to `to`, or only `find`.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        slide: { type: 'integer', description: 'PowerPoint.' },
        shape: { type: 'string', description: 'PowerPoint.' },
        from: { type: ['integer', 'string'], description: 'Word: a number or an id like "#k3f9x2"; required. PowerPoint: a number within the shape.' },
        to: { type: ['integer', 'string'], description: 'Default: `from`.' },
        find: { type: 'string', description: 'Only this text, each time it occurs.' },
        ...TEXT_FORMAT,
        style: { type: 'string', description: 'Word: like "Heading 1" or "Normal".' },
        highlight: { type: 'string', enum: HIGHLIGHTS, description: 'Word.' },
        list: { type: 'string', enum: ['bullet', 'numbered', 'none'], description: 'Word.' },
        line_spacing: { type: 'number', description: 'Word: a multiple, like 1.15.' },
      },
      required: ['document'],
    },
  },
  {
    name: 'add_picture',
    description: 'Word and PowerPoint: adds a PNG, JPEG, GIF, WebP or SVG from `path`, `url` or `data`. Giving only width or height keeps its proportions.\nWord: as its own paragraph after `after`.\nPowerPoint: on slide `slide`, in the middle unless x and y are given. Returns its id.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        path: { type: 'string', description: 'An absolute path. Agents on this computer only.' },
        url: { type: 'string', description: 'Agents on this computer only.' },
        data: { type: 'string', description: 'Base64.' },
        after: paraRef('Word: 0 for the start'),
        slide: { type: 'integer', description: 'PowerPoint.' },
        x: BOX.x,
        y: BOX.y,
        width: { type: 'number', description: 'In pixels.' },
        height: { type: 'number', description: 'In pixels.' },
        name: { type: 'string', description: 'PowerPoint.' },
      },
      required: ['document'],
    },
  },
]

const AFTER_TOOLS: Tool[] = [
  {
    name: 'undo',
    description: 'Undoes your last change, never anyone else’s. In Excel, that’s one cell.',
    inputSchema: { type: 'object', properties: { document: doc }, required: ['document'] },
  },
  {
    name: 'redo',
    description: 'Redoes what your last undo removed, unless someone has changed it since.',
    inputSchema: { type: 'object', properties: { document: doc }, required: ['document'] },
  },
  {
    name: 'wait_for_changes',
    description: 'Waits up to `seconds` for someone to edit the document, post in its chat, or come in or leave, then says who changed what. Returns at once if something you haven’t seen is waiting.',
    inputSchema: {
      type: 'object',
      properties: {
        document: doc,
        seconds: { type: 'number', description: `0 to ${MAX_WAIT}. Default 30.` },
        messages_only: { type: 'boolean', description: 'Ignore edits.' },
        full: { type: 'boolean', description: 'Word: whole paragraphs, up to 500.' },
      },
      required: ['document'],
    },
  },
  {
    name: 'leave_document',
    description: 'Leaves a document. Its tab stays open.',
    inputSchema: { type: 'object', properties: { document: doc }, required: ['document'] },
  },
]

export const TOOLS: Tool[] = [...SHARED_TOOLS, ...WORD_TOOLS, ...SLIDES_TOOLS, ...SHEETS_TOOLS, ...AFTER_TOOLS, ...MESSAGE_TOOLS, ...FILE_TOOLS, ...THEME_TOOLS]

/** Tools Writan had before, and what does their work now, for an agent that still calls them. */
export const RENAMED: Record<string, string> = {
  where_is_everyone: 'list_documents with document',
  read_messages: 'wait_for_changes with messages_only, or seconds: 0',
  set_style: 'format_text with style',
  change_table: 'edit_table',
  format_picture: 'edit_picture',
  move_shape: 'edit_shape',
  format_shape: 'edit_shape',
  duplicate_slide: 'add_slide with copy',
  move_slide: 'edit_slide with to',
  set_slide: 'edit_slide',
  set_notes: 'edit_slide with notes',
  clear_cells: 'write_cells with clear',
  merge_cells: 'format_cells with merge',
  size_cells: 'resize_rows_columns',
  insert_cells: 'insert_rows_columns',
  delete_cells: 'delete_rows_columns',
  read_theme: 'read_app_theme',
  set_theme: 'set_app_theme',
}

/** Which kind of document each kind's own tools are for, to say so when one is called on another. */
const KIND_OF = new Map<string, string>([...WORD_TOOLS.map((t) => [t.name, 'Word documents'] as const), ...SLIDES_TOOLS.map((t) => [t.name, 'presentations'] as const), ...SHEETS_TOOLS.map((t) => [t.name, 'workbooks'] as const)])
const KIND_NAME = { docx: 'a Word document', pptx: 'a presentation', xlsx: 'a workbook' } as const

/** The tools as an agent here sees them: without messages when Settings › AI turned them off. */
export function listTools(): Tool[] {
  return messagesOn() ? TOOLS : TOOLS.filter((t) => !MESSAGE_TOOL_NAMES.has(t.name))
}

// ── Doing them

/** Runs a tool for an agent. `local` says whether the agent is on this computer. */
export async function runTool(session: AgentSession, name: string, args: Args, local = false): Promise<Answer> {
  // Waiting counts from here, so joining first (which can wait for the host to let the agent in)
  // doesn't make the answer too late for the agent's client.
  const began = Date.now()
  if (name === 'list_documents' && !str(args, 'document')) return { text: listDocuments(session) }
  if (THEME_TOOLS.some((t) => t.name === name)) return runThemeTool(name, args, local)
  if (name === 'open_document') return { text: await openDocument(args, local, documents) }
  if (MESSAGE_TOOL_NAMES.has(name) && !messagesOn()) throw new ToolError(OFF)
  const tab = need(str(args, 'document'), 'the document')
  // Messages it hasn't seen come with these too: on leaving, before they go with its copy.
  const joined = session.joinedCopy(tab)
  if (name === 'leave_document') {
    const unseen = joined ? `${newComings(joined.id)}${newMessages(joined.id)}` : ''
    return { text: `${session.leave(tab) ? 'Left.' : 'You weren’t in it.'}${unseen}` }
  }
  if (name === 'save_document') {
    const d = documents().find((x) => x.document === tab)
    if (!d) throw new ToolError(`No document “${tab}”. list_documents shows the ones you can work on.`)
    // What this agent changed reaches the tab first: the relay has it, and the tab applied it.
    const c = joined
    try {
      if (c) {
        const note = await delivered(session, c, await c.engine)
        if (note) throw new ToolError(`Not saved: your last change hasn’t reached the document yet.${note}`)
        if (!(await caughtUp(tab, appliedSeq(c.id)))) throw new ToolError('Not saved: the document hasn’t caught up with your last change yet. Try again in a moment.')
      }
      return { text: `${await saveDocument(tab, d.role, args, local)}${c ? `${newComings(c.id)}${newMessages(c.id)}` : ''}` }
    } catch (err) {
      if (c && err instanceof ToolError) throw new ToolError(`${err.message}${newComings(c.id)}${newMessages(c.id)}`)
      throw err
    }
  }
  const { copy, engine } = await session.copy(tab)
  let text: Answer
  try {
    text = await runOn(session, copy, engine, name, args, local, began)
  } catch (err) {
    // A refusal brings the chat's new messages too: the answer it waits for may be among them.
    if (err instanceof ToolError) throw new ToolError(`${err.message}${newComings(copy.id)}${newMessages(copy.id)}`)
    throw err
  }
  // Every answer brings who came or went and the chat's new messages: an agent hears of them on
  // its next call, whichever tool it is.
  return { ...text, text: `${text.text}${newComings(copy.id)}${newMessages(copy.id)}` }
}

/** Runs a tool on the agent's copy of a document. */
async function runOn(session: AgentSession, copy: Copy, engine: Engine, name: string, args: Args, local: boolean, began: number): Promise<Answer> {
  let answer: Answer
  /** The seconds asked for, at most what's left of MAX_WAIT since the call began. */
  const waitFor = (fallback: number) => Math.min(MAX_WAIT - (Date.now() - began) / 1000, Math.max(0, num(args, 'seconds') ?? fallback))
  switch (name) {
    case 'list_documents':
      return { text: `${listDocuments(session, copy.tab)}\nWhere everyone is:\n${whereIsEveryone(copy, engine)}` }
    case 'read_document': {
      // The key to a Word reading comes with the first one only.
      const legend = !copy.read
      copy.read = true
      return {
        text:
          copy.kind === 'docx'
            ? readWord(engine as WordEngine, args, { history: copy.history, chat: messagesOn() ? chatOf(copy.id) : undefined, version: appliedSeq(copy.id), legend })
            : copy.kind === 'pptx'
              ? readSlides(engine as SlidesEngine, args)
              : readSheets(engine as SheetsEngine, args),
      }
    }
    case 'view_document':
      return copy.kind === 'pptx' ? viewSlide(engine as SlidesEngine, args) : copy.kind === 'xlsx' ? await viewSheet(engine as SheetsEngine, args) : viewPage(engine as WordEngine, args)
    case 'wait_for_changes': {
      const seconds = Math.max(0, Math.round(waitFor(30)))
      if (bool(args, 'messages_only') === true) {
        if (!messagesOn()) throw new ToolError(OFF)
        return { text: await readMessages(copy.id, seconds, () => nextComing(copy.id)) }
      }
      if (!seconds) return { text: comings(copy.id).length || (messagesOn() && unread(copy.id).length) ? 'Since your last call:' : 'Nothing new since your last call.' }
      return { text: await waitForChanges(copy, engine, seconds, began, bool(args, 'full') === true) }
    }
    case 'send_message':
      return { text: sendMessage(copy.id, args) }
    case 'find_text': {
      const find = need(str(args, 'text'), 'the text to find')
      if (!find) throw new ToolError('Give the text to find.')
      const matchCase = bool(args, 'match_case') === true
      return { text: copy.kind === 'docx' ? findInWord(engine as WordEngine, find, matchCase) : copy.kind === 'pptx' ? findInSlides(engine as SlidesEngine, find, matchCase) : findInSheets(engine as SheetsEngine, find, matchCase) }
    }
    case 'undo':
    case 'redo': {
      const e = engine as { run(c: { cmd: string }): unknown; state: { canUndo: boolean; canRedo: boolean } }
      if (name === 'undo' ? !e.state.canUndo : !e.state.canRedo) return { text: name === 'undo' ? 'Nothing of yours to undo.' : 'Nothing to redo.' }
      // Someone else may have changed that part since; then it's left as they made it.
      if (e.run({ cmd: name }) !== true) return { text: `Couldn’t ${name} it: someone else has changed that part since. Read it again.` }
      answer = { text: name === 'undo' ? 'Undone.' : 'Redone.' }
      break
    }
    case 'replace_text':
      answer = { text: copy.kind === 'xlsx' ? replaceInSheets(engine as SheetsEngine, need(str(args, 'find'), 'find'), str(args, 'replace') ?? '') : replaceText(engine as WordEngine | SlidesEngine, need(str(args, 'find'), 'find'), str(args, 'replace') ?? '', bool(args, 'match_case') === true) }
      break
    default: {
      if (copy.kind === 'xlsx' && (name === 'format_text' || name === 'add_picture')) throw new ToolError(name === 'format_text' ? 'In a workbook, use format_cells.' : 'add_picture works in Word documents and presentations, not workbooks.')
      const kind = KIND_OF.get(name)
      if (kind && !(kind === 'Word documents' ? copy.kind === 'docx' : kind === 'presentations' ? copy.kind === 'pptx' : copy.kind === 'xlsx')) throw new ToolError(`${name} is for ${kind}, and this is ${KIND_NAME[copy.kind]}.`)
      answer =
        copy.kind === 'docx'
          ? await wordTool(engine as WordEngine, name, args, local, copy.history)
          : copy.kind === 'pptx'
            ? await slidesTool(engine as SlidesEngine, name, args, local)
            : sheetsTool(engine as SheetsEngine, name, args)
    }
  }
  // What changed is this agent's, in the change after the last it has.
  copy.history?.saw(copy.id, appliedSeq(copy.id) + 1)
  const note = await delivered(session, copy, engine)
  // Once everyone has it, the answer says what the document holds now.
  const now = answer.after && session.has(copy) ? await answer.after() : ''
  return { text: `${answer.text}${note}${now}`, pictures: answer.pictures }
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

/** Waits up to three seconds for a tab to have applied the relay's changes up to `seq`. */
async function caughtUp(tab: string, seq: number): Promise<boolean> {
  for (let i = 0; i < 60 && appliedSeq(tab) < seq; i++) await new Promise((r) => setTimeout(r, 50))
  return appliedSeq(tab) >= seq
}

function peopleIn(id: string): Peer[] {
  const s = useCollab.getState()[id]
  return s ? [...(s.you ? [s.you] : []), ...s.peers] : []
}

/** list_documents: every document, or just `only`. */
function listDocuments(session: AgentSession, only?: string): string {
  const docs = documents().filter((d) => !only || d.document === only)
  if (!docs.length) return 'No Word document, presentation or workbook is open in Writan. Open one with open_document, or ask the person to.'
  const roles = { sharing: 'shared from here', joined: 'joined from here', open: 'not shared: only the person here and agents on this computer' }
  return docs
    .map((d) => {
      const people = peopleIn(d.document).map((p) => (p.agent ? `${p.name} (AI${messagesOn() && !p.listens ? ', no messages' : ''})` : p.name))
      const copy = session.joinedCopy(d.document)
      const waiting = copy && messagesOn() ? unread(copy.id).length : 0
      return `- ${d.title}  document: "${d.document}"  (${roles[d.role]}; in it: ${people.join(', ') || 'nobody yet'})${waiting ? `; ${plural(waiting, 'unread message')} for you` : ''}`
    })
    .join('\n')
}

type Snapshot = { kind: 'docx'; paras: string[] } | { kind: 'pptx'; slides: ReturnType<typeof snapshotSlides> } | { kind: 'xlsx' }

function snapshot(copy: Copy, engine: Engine): Snapshot {
  if (copy.kind === 'docx') return { kind: 'docx', paras: snapshotWord(engine as WordEngine) }
  if (copy.kind === 'pptx') return { kind: 'pptx', slides: snapshotSlides(engine as SlidesEngine) }
  return { kind: 'xlsx' }
}

/** How long people wait between keys while still typing a word or a sentence, at most. */
const TYPING_PAUSE = 2000
/** Agents' changes come whole: a second without one means they're done. */
const AGENT_PAUSE = 1000
/** The answer has to be in before the agent's client gives up (see MAX_WAIT). */
const SETTLE_BY = (MAX_WAIT + 5) * 1000 - 100

/**
 * Waits for someone else's change, then a moment more while they're still at it (typing comes a
 * key at a time), and says who changed what. A message in the chat ends the wait at once.
 */
async function waitForChanges(copy: Copy, engine: Engine, seconds: number, began: number, full: boolean): Promise<string> {
  const before = snapshot(copy, engine)
  const who = new Set<string>()
  const said = messagesOn() ? nextMessage(copy.id) : null
  const arrival = nextComing(copy.id)
  const first = await new Promise<'change' | 'message' | 'coming' | null>((resolve) => {
    const timer = window.setTimeout(() => done(null), seconds * 1000)
    const stop = onOthersChange(copy.id, (client) => {
      who.add(client)
      done('change')
    })
    void said?.message.then(() => done('message'))
    void arrival.coming.then(() => done('coming'))
    function done(how: 'change' | 'message' | 'coming' | null) {
      window.clearTimeout(timer)
      stop()
      said?.stop()
      arrival.stop()
      resolve(how)
    }
  })
  // The messages and who came or went come at the end of the answer.
  if (first === 'message') return 'A message came in the document’s chat. Nobody changed the document meanwhile.'
  if (first === 'coming') return 'Someone came into the document or left it. Nobody changed the document meanwhile.'
  if (!first) return `Nobody changed it in ${seconds} seconds.`
  const peers = () => useCollab.getState()[copy.id]?.peers ?? []
  // Until they've stopped for a moment: a person pauses between words, an agent's change comes
  // whole. A message ends it too.
  const pause = () => ([...who].some((c) => !peers().find((p) => p.client === c)?.agent) ? TYPING_PAUSE : AGENT_PAUSE)
  const settled = await new Promise<'quiet' | 'busy' | 'message' | 'coming'>((resolve) => {
    let quiet = window.setTimeout(() => done('quiet'), pause())
    const cap = window.setTimeout(() => done('busy'), Math.max(0, SETTLE_BY - (Date.now() - began)))
    const stop = onOthersChange(copy.id, (client) => {
      who.add(client)
      window.clearTimeout(quiet)
      quiet = window.setTimeout(() => done('quiet'), pause())
    })
    const msg = messagesOn() ? nextMessage(copy.id) : null
    void msg?.message.then(() => done('message'))
    const arrival = nextComing(copy.id)
    void arrival.coming.then(() => done('coming'))
    function done(how: 'quiet' | 'busy' | 'message' | 'coming') {
      window.clearTimeout(quiet)
      window.clearTimeout(cap)
      stop()
      msg?.stop()
      arrival.stop()
      resolve(how)
    }
  })
  // Their last change is applied once the relay's message has been handled.
  await new Promise((r) => setTimeout(r, 50))
  const names = [...who].map((c) => peers().find((p) => p.client === c)?.name ?? 'Someone')
  const after = snapshot(copy, engine)
  let what = ''
  if (before.kind === 'docx' && after.kind === 'docx') what = `${wordChanges(before.paras, after.paras, full)}\nThis is version ${appliedSeq(copy.id)}.`
  else if (before.kind === 'pptx' && after.kind === 'pptx') what = slidesChanges(before.slides, after.slides) || 'Nothing on the slides looks different (the change was to something read_document doesn’t show).'
  // Where each of them is now, which in a workbook is where they changed it.
  const places = usePresence.getState()[copy.id] ?? {}
  const at = [...who]
    .map((c) => {
      const p = places[c]?.at as Record<string, unknown> | undefined
      return p ? `${peers().find((x) => x.client === c)?.name ?? 'Someone'} is at ${describe(copy.kind, engine, p)}.` : null
    })
    .filter(Boolean)
  const still = {
    quiet: '',
    busy: ' They’re still at it, so this may be half done: wait again before acting on it.',
    message: ' A message came before they’d stopped, so this may be half done: wait again before acting on it.',
    coming: ' Someone came in or left before they’d stopped, so this may be half done: wait again before acting on it.',
  }[settled]
  return [`${names.join(' and ')} changed it.${still}`, what, ...at].filter(Boolean).join('\n')
}

/** For the end of any answer about a document: who came in or left since the agent last heard. */
function newComings(id: string): string {
  const got = comings(id, true)
  if (!got.length) return ''
  return `\n\n${got.map((c) => `${c.peer.name}${c.peer.agent ? ' (AI)' : ''} ${c.joined ? 'came into the document' : 'left the document'} ${ago(c.at)}.`).join('\n')}`
}

/** Resolves once someone has come into or left copy `id`'s document since its agent last heard (at once if they have); `stop` gives up. */
function nextComing(id: string): { coming: Promise<void>; stop: () => void } {
  let stop = () => {}
  const coming = new Promise<void>((resolve) => {
    if (comings(id).length) return resolve()
    stop = onComing(id, () => {
      stop()
      resolve()
    })
  })
  return { coming, stop }
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
  // The presentation says how many shapes it changed: a match whose formatting changes partway
  // through it is left.
  const { matched, changed } = e.run<{ matched: number; changed: number }>({ cmd: 'replaceAll', text: by })
  const left = matched - changed
  return left
    ? `Replaced in ${changed} shape${changed === 1 ? '' : 's'}. In ${left} more, the text’s formatting changes partway through it, so it wasn’t replaced there; use set_shape_text.`
    : `Replaced in ${changed} shape${changed === 1 ? '' : 's'}.`
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

const samePath = (a: number[], b: number[]) => a.length === b.length && a.every((x, i) => x === b[i])

function describe(kind: Copy['kind'], engine: Engine, at: Record<string, unknown>): string {
  if (kind === 'docx') {
    if (at.s !== 'b') return 'in a header, footer or note'
    const e = engine as WordEngine
    const paras = JSON.parse(e.doc.paragraphs()) as { path: number[]; text: string }[]
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
    const names = (JSON.parse(e.doc.elements(i)) as { id: string; name: string }[]).filter((el) => ids.includes(el.id)).map((el) => `"${el.id}" “${el.name}”`)
    const typing = at.e ? ', typing' : ''
    return `slide ${i + 1}${names.length ? `, shape ${names.join(', ')}${typing}` : ''}`
  }
  return describeSheetsPlace(at)
}

