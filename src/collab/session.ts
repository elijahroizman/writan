// Sharing a Word document, presentation or workbook so others can edit it at the same time.
//
// The person who shares hosts the session: their copy stays the file on disk, saved as usual,
// and everyone else's tab holds a copy that follows it. Every copy, the host's included, sends
// its changes to the relay, which puts them in one order and sends each to everyone. The engine
// does the rest (crates/office-engine/src/collab.rs): it applies changes in that order, so the
// copies agree, and drops a change that collides with an earlier one.
//
// The relay is on the host's computer. Guests on the network reach it over a WebSocket, and guests
// from the internet through an iroh connection between the apps (transport.ts). Nothing here
// depends on which: a cloud relay speaking the same messages only changes the invite's address.

import { create } from 'zustand'
import { showDialog } from '../lib/dialogs'
import { stripExtension } from '../lib/kinds'
import { collabHooks, getEditor, useWorkspace, wakeTab } from '../state/workspace'
import { approveJoin, connect, connectP2p, findInvite, host, onHostEvents, parseCode, parseInvite, publish, rotateCode, setRequireApproval, unhost, type Invite, type JoinRequest, type Published, type Transport } from './transport'

export const SHAREABLE = new Set(['docx', 'pptx', 'xlsx'])

/** What an engine's WebAssembly document offers for sharing. */
export interface CollabDoc {
  collabStart(client: string): void
  collabStop(): void
  collabTake(): string | undefined
  collabReceive(op: string): boolean
  collabPending(): boolean
}

/** An open document's engine, as sharing sees it. */
export interface CollabEngine {
  readonly doc: CollabDoc
  /** Set while shared: told after every change, selection included, to send it. */
  collab: { changed(): void } | null
  refresh(): void
  /** Where the caret or selection is, as JSON, for the others to draw. */
  presence(): string
}

export interface Peer {
  client: string
  name: string
  color: string
  /** The person sharing, who keeps the file. */
  host?: boolean
  /** For an AI agent: which (`claude`, `codex`…; see agents/identity.ts), or `ai` for another. */
  agent?: string
  /** For an AI agent: the person whose app it joined through. `name` already says so (“Yoav’s
   * Claude”), except for this app's own agents, which are just “Claude” here. */
  owner?: string
  /** For an AI agent: whether it gets other agents' messages (its person allows them). */
  listens?: boolean
}

/** A message in a document's chat, from an agent, as a copy here got it. */
export interface Said {
  /** The sender's copy. */
  client: string
  /** The sender as people here see it (“Yoav’s Claude”), when it arrived. */
  name: string
  /** Which agent sent it (`claude`, `codex`…, or `ai`). */
  agent: string
  text: string
  /** When it arrived here (Unix ms). */
  at: number
  /** Said before this agent joined. */
  earlier?: boolean
  /** Said by this agent's copy itself. */
  mine?: boolean
}

/** Where someone else is in a document: what their engine's `presence` gave. */
export interface Place {
  peer: Peer
  /** Parsed `presence`. */
  at: unknown
  /** Goes up each time they move, so a name tag can show again. */
  moves: number
}

export interface Share {
  role: 'host' | 'guest'
  status: 'connecting' | 'live' | 'ended' | 'disconnected'
  /** This copy as the others see it, once the relay has welcomed it. */
  you: Peer | null
  /** The link others join with (the host's to hand out; a guest's to rejoin). */
  invite: string | null
  /** The six digits guests join with, for the host to hand out. */
  code: string | null
  /** The host's addresses on the network. */
  addresses: string[]
  /** The others in the document. */
  peers: Peer[]
  /** Why it ended or what went wrong. */
  message: string | null
  /** For the host: whether the code works over the internet (else on the network only), and until when (Unix ms). */
  internet?: boolean
  expiresAt?: number | null
  /** For the host: whether guests from the internet wait to be let in, and who's waiting. */
  requireApproval?: boolean
  waiting?: JoinRequest[]
  /** For the host: why sharing over the internet didn't start. */
  internetError?: string | null
  /** For the host: shared with this computer's AI agents only, with no code yet (`openToAgents`). */
  private?: boolean
}

/** Shared tabs, by tab id. */
export const useCollab = create<Record<string, Share>>(() => ({}))

/** A shared tab's chat: what the agents in the document said to each other, oldest first, and
 * how many of those the person has seen. People don't get a note for each: they look when they
 * want to (AgentChat.tsx). A relay before Writan 0.15.3 sends people none. */
export const useChat = create<Record<string, { said: Said[]; seen: number }>>(() => ({}))

/** A tab's chat keeps at most this many messages. */
const CHAT_KEPT = 200

/** The person has seen a tab's chat up to now. */
export function seenChat(tabId: string) {
  const c = useChat.getState()[tabId]
  if (c && c.seen !== c.said.length) useChat.setState({ [tabId]: { ...c, seen: c.said.length } })
}

/** Someone arriving in or leaving a shared tab, for the note in the corner (Arrivals.tsx). */
export interface Arrival {
  id: number
  tabId: string
  peer: Peer
  joined: boolean
}

export const useArrivals = create<Arrival[]>(() => [])
let arrivals = 0

/** Shows that someone came or went, for a few seconds. Not for an agent's own copies. */
function arrive(tabId: string, peer: Peer, joined: boolean) {
  if (tabId.startsWith('agent:')) return
  const a = { id: ++arrivals, tabId, peer, joined }
  useArrivals.setState((now) => [...now.slice(-3), a], true)
  window.setTimeout(() => useArrivals.setState((now) => now.filter((x) => x.id !== a.id), true), 6500)
}

/** Where everyone else is, by tab id and then client. Apart from `useCollab`, so the title bar
 * doesn't draw again every time someone moves their caret. */
export const usePresence = create<Record<string, Record<string, Place>>>(() => ({}))

/** The tab each attached engine belongs to. */
const engineTabs = new WeakMap<object, string>()

/** Everyone else's place in the document an engine has open, while it's shared. */
export function usePlaces(engine: object | null): Place[] {
  // Looked up on each change: the engine is attached after the editor first draws.
  const places = usePresence((s) => {
    const tabId = engine ? engineTabs.get(engine) : undefined
    return tabId ? s[tabId] : undefined
  })
  return places ? Object.values(places) : NO_PLACES
}
const NO_PLACES: Place[] = []

function setPlaces(tabId: string, f: (now: Record<string, Place>) => Record<string, Place> | null) {
  usePresence.setState((s) => {
    const next = f(s[tabId] ?? {})
    const { [tabId]: _old, ...rest } = s
    return next ? { ...rest, [tabId]: next } : rest
  }, true)
}

/** The relay hands these out so no two people match (src-tauri/src/collab/relay.rs, `COLORS`). */
const COLORS = ['#e5484d', '#3e63dd', '#30a46c', '#f76b15', '#8e4ec6', '#12a594', '#d6409f', '#ad7f58']

export function colorFor(client: string) {
  let h = 0
  for (const c of client) h = (h * 31 + c.charCodeAt(0)) | 0
  return COLORS[Math.abs(h) % COLORS.length]
}

const NAME_KEY = 'workspace.collabName'

/** The name others see, as last typed. */
export function displayName(): string {
  try {
    return localStorage.getItem(NAME_KEY) || ''
  } catch {
    return ''
  }
}

export function setDisplayName(name: string) {
  try {
    localStorage.setItem(NAME_KEY, name.trim())
  } catch {
    // Not remembered; it's asked again next time.
  }
}

function newClient() {
  return Array.from(crypto.getRandomValues(new Uint8Array(9)), (b) => b.toString(16).padStart(2, '0')).join('')
}

function toBase64(bytes: Uint8Array) {
  let s = ''
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000))
  return btoa(s)
}

function fromBase64(text: string) {
  const s = atob(text)
  const out = new Uint8Array(s.length)
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i)
  return out
}

function newLink(name: string): Link {
  const client = newClient()
  return { tabId: '', role: 'guest', client, name, transport: null, engine: null, queue: [], welcomed: false, live: false, closed: false, presence: null, presenceTimer: null, invite: null, features: [], inbox: [], chat: [], comings: [], gone: new Map(), seq: 0, queuedSeq: 0 }
}

/** One shared tab's connection. */
interface Link {
  tabId: string
  role: 'host' | 'guest'
  client: string
  name: string
  transport: Transport | null
  session?: string
  engine: CollabEngine | null
  /** Changes that arrived before the engine was ready, in order, with whose each is and its number. */
  queue: { data: string; client: string | null; seq: number }[]
  welcomed: boolean
  live: boolean
  closed: boolean
  /** The last presence sent, and a send waiting to go. */
  presence: string | null
  presenceTimer: number | null
  /** Where this copy connects, to let an agent join the same document. */
  invite: Invite | null
  /** For an agent's copy: which agent, and whose (the name of the person whose tab it
   * follows), sent in the hello; and that tab. */
  agent?: string
  owner?: string
  follows?: string
  /** For a guest over the internet: what this app's agents join the same document with. */
  seat?: string
  /** What the relay does beyond version 1 (`say`), from its welcome. */
  features: string[]
  /** The relay's number of the last change applied here, and of the last one waiting to be. */
  seq: number
  queuedSeq: number
  /** For an agent's copy: whether it gets other agents' messages, and the ones it hasn't read. */
  listens?: boolean
  inbox: Said[]
  /** For an agent's copy: the chat it has seen and said, oldest first, for the notes on paragraphs. */
  chat: Said[]
  /** For an agent's copy: who came or went since it last heard. */
  comings: Coming[]
  /** Who was here and left, by client, so their changes can be put to names. */
  gone: Map<string, Peer>
  /** For a host sharing what only its agents were in: getting its code, once. */
  publishing?: Promise<void>
  /** Until the welcome: gives up joining, for `finish`. */
  abort?: (reason: string) => void
}

/** Moving the caret sends where it is at most this often. */
const PRESENCE_EVERY = 50

const links = new Map<string, Link>()

function update(tabId: string, patch: Partial<Share>) {
  const now = useCollab.getState()[tabId]
  if (now) useCollab.setState({ [tabId]: { ...now, ...patch } })
}

function hello(link: Link, token: string, extra: Record<string, unknown> = {}) {
  // The color asked for, from the name so a person tends to keep theirs from one document to
  // the next; the relay gives another if someone has it. An agent asks for its own.
  const color = link.agent ? (extra.color as string) : colorFor(link.name || link.client)
  link.transport?.send(JSON.stringify({ t: 'hello', v: 1, token, client: link.client, name: link.name, ...extra, color, ...(link.agent ? { agent: link.agent, owner: link.owner, listens: !!link.listens } : {}) }))
}

/** Sends what changed here, then where the caret is if that changed. */
function flush(link: Link) {
  if (!link.live || !link.engine || link.closed) return
  let sent = false
  for (let op = link.engine.doc.collabTake(); op; op = link.engine.doc.collabTake()) {
    link.transport?.send(`{"t":"op",${op.slice(1)}`)
    sent = true
  }
  // After a change the others get the caret again even if it didn't move: what they had was
  // in a document without that change, and may not have fit theirs.
  if (sent) link.presence = null
  if (link.presenceTimer === null) link.presenceTimer = window.setTimeout(() => sendPresence(link), sent ? 0 : PRESENCE_EVERY)
}

function sendPresence(link: Link) {
  link.presenceTimer = null
  if (!link.live || !link.engine || link.closed) return
  const at = link.engine.presence()
  if (at === link.presence) return
  link.presence = at
  link.transport?.send(`{"t":"presence","data":${at}}`)
}

/** This app's agents' copies, by client, joining or joined. Here they show by their own name
 * (“Claude”); everywhere else as their person's (“Eli’s Claude”). */
const ownAgents = new Set<string>()

function peerOf(p: Record<string, unknown>): Peer {
  const client = p.client as string
  const agent = typeof p.agent === 'string' ? p.agent : undefined
  const owner = agent && typeof p.owner === 'string' && p.owner ? p.owner : undefined
  const name = owner && !ownAgents.has(client) ? `${owner}’s ${p.name as string}` : (p.name as string)
  return { client, name, color: p.color as string, host: !!p.host, ...(agent ? { agent } : {}), ...(owner ? { owner } : {}), ...(agent && p.listens === true ? { listens: true } : {}) }
}

/** Someone's place, from a presence message or a welcome's peer. */
function place(tabId: string, peer: Peer, at: unknown) {
  setPlaces(tabId, (now) => ({ ...now, [peer.client]: { peer, at, moves: (now[peer.client]?.moves ?? 0) + 1 } }))
}

/** Goes live once the relay has welcomed this copy and its engine is open. */
function start(link: Link) {
  const engine = link.engine
  if (!engine || !link.welcomed || link.live || link.closed) return
  link.live = true
  // Applied one after another as the relay ordered them; whoever keeps track of whose changes
  // are whose hears after each.
  const heard = appliedListeners.get(link.tabId)
  link.queue.forEach((op, i) => {
    engine.doc.collabReceive(op.data)
    if (heard && op.client && op.client !== link.client) for (const f of heard) f(op.client, op.seq, link.queue.length - 1 - i)
  })
  link.queue = []
  link.seq = link.queuedSeq
  engine.refresh()
  flush(link)
  update(link.tabId, { status: 'live' })
}

/** Stops sharing on this copy: it goes on as an ordinary unsaved document. */
function finish(link: Link, status: Share['status'], message: string | null) {
  if (link.closed) return
  link.closed = true
  link.live = false
  if (!link.welcomed) link.abort?.(message ?? 'Cancelled.')
  if (link.engine) {
    link.engine.collab = null
    link.engine.doc.collabStop()
  }
  link.transport?.close()
  link.transport = null
  if (link.presenceTimer !== null) window.clearTimeout(link.presenceTimer)
  link.presenceTimer = null
  links.delete(link.tabId)
  setPlaces(link.tabId, () => null)
  update(link.tabId, { status, message, peers: [], waiting: [] })
  if (link.agent) {
    agentLinks.delete(link)
    ownAgents.delete(link.client)
    if (link.follows) closeIfNoAgents(link.follows)
    return
  }
  // The agents that joined through this tab leave with it: a guest who leaves takes theirs along,
  // waiting to be let in or not.
  for (const agent of [...agentLinks]) {
    if (agent.follows !== link.tabId) continue
    agent.transport?.send(JSON.stringify({ t: 'bye' }))
    finish(agent, 'ended', 'The document isn’t shared here any more.')
  }
}

/** This app's agents' copies, joining or joined. */
const agentLinks = new Set<Link>()

/** A tab shared with this computer's agents only ends when the last of them leaves. */
function closeIfNoAgents(tabId: string) {
  if (!useCollab.getState()[tabId]?.private || links.get(tabId)?.publishing) return
  if ([...agentLinks].some((l) => l.follows === tabId && !l.closed)) return
  stop(tabId)
}

/** The name of whoever shares a guest's document. */
function hostOf(tabId: string): string {
  return useCollab.getState()[tabId]?.peers.find((p) => p.host)?.name || 'The person sharing'
}

/**
 * The person sharing a guest's tab stopped, or the guest lost them: says so, then closes the
 * tab, since it can't follow their document any more. The guest can keep it as a copy instead.
 */
async function hostGone(link: Link, host: string, stopped: boolean) {
  // An agent's copy has no tab; its agent finds out from its tools.
  if (link.role !== 'guest' || link.agent) return
  const tabId = link.tabId
  const title = useWorkspace.getState().tabs.find((t) => t.id === tabId)?.title
  if (!title) return
  const answer = await showDialog({
    kind: 'info',
    title: stopped ? `${host} stopped sharing “${title}”` : `Lost the connection to ${host}`,
    message: `${stopped ? `${host} has the file, with the changes that reached them.` : `“${title}” can’t follow ${host}’s changes any more.`} It closes here unless you keep a copy of your own.`,
    buttons: [
      { label: 'Keep a Copy', value: 'keep' },
      { label: 'Close', value: 'close', primary: true },
    ],
    cancelValue: 'close',
  })
  const workspace = useWorkspace.getState()
  if (answer !== 'close' || !workspace.tabs.some((t) => t.id === tabId)) return
  // Nothing in it is the guest's to save: the file is the host's.
  workspace.markDirty(tabId, false)
  await workspace.closeTab(tabId)
}

const ERRORS: Record<string, string> = {
  'bad-token': 'This invite isn’t valid any more. Ask for a new code.',
  'no-session': 'This document isn’t being shared any more.',
  'not-ready': 'This document isn’t being shared any more.',
  version: 'The person sharing has a different version of Writan.',
}

function onMessage(link: Link, data: string, opened?: (snapshot: { bytes: Uint8Array; name: string; kind: string }) => string | null) {
  let msg: Record<string, unknown>
  try {
    msg = JSON.parse(data)
  } catch {
    return
  }
  switch (msg.t) {
    case 'welcome': {
      const raw = msg.peers as Record<string, unknown>[]
      const peers = raw.map(peerOf)
      // An older relay doesn't hand out colors; then this copy keeps the one it asked for.
      const you: Peer = { client: link.client, name: link.name, color: (msg.color as string) || colorFor(link.name || link.client), host: link.role === 'host' }
      if (opened) {
        const session = msg.session as { name: string; kind: string }
        const tabId = opened({ bytes: fromBase64(msg.snapshot as string), name: session.name, kind: session.kind })
        if (!tabId) return finish(link, 'disconnected', 'Writan can’t open this kind of document.')
        link.tabId = tabId
        links.set(tabId, link)
        useCollab.setState({ [tabId]: { role: 'guest', status: 'connecting', you, invite: null, code: null, addresses: [], peers, message: null } })
      } else {
        update(link.tabId, { peers, you })
      }
      raw.forEach((p, i) => p.presence != null && place(link.tabId, peers[i], p.presence))
      link.features = Array.isArray(msg.features) ? (msg.features as unknown[]).filter((f): f is string => typeof f === 'string') : []
      // The chat so far: an agent's copy reads it as messages from before it joined; a person's
      // tab starts its chat again from it.
      if (!link.agent) useChat.setState({ [link.tabId]: { said: [], seen: 0 } })
      if (Array.isArray(msg.said)) for (const m of msg.said as Record<string, unknown>[]) heard(link, m, true)
      seenChat(link.tabId)
      // Who left before this copy came, for the changes they made (Writan after 0.15.3).
      if (Array.isArray(msg.gone)) for (const g of msg.gone as Record<string, unknown>[]) if (typeof g.client === 'string' && typeof g.name === 'string') link.gone.set(g.client, peerOf({ ...g, color: '' }))
      link.queue.push(
        ...(msg.ops as Record<string, unknown>[]).map((op) => ({ data: JSON.stringify(op), client: typeof op.client === 'string' ? op.client : null, seq: Number(op.seq) || 0 })),
      )
      link.queuedSeq = Number(msg.seq) || 0
      link.welcomed = true
      start(link)
      return
    }
    case 'op':
      if (msg.client !== link.client) for (const f of changeListeners.get(link.tabId) ?? []) f(msg.client as string)
      if (!link.live || !link.engine) {
        link.queue.push({ data, client: typeof msg.client === 'string' ? msg.client : null, seq: Number(msg.seq) || 0 })
        link.queuedSeq = Number(msg.seq) || link.queuedSeq
        return
      }
      if (link.engine.doc.collabReceive(data)) link.engine.refresh()
      link.seq = Number(msg.seq) || link.seq
      if (msg.client !== link.client) for (const f of appliedListeners.get(link.tabId) ?? []) f(msg.client as string, link.seq, 0)
      // The next change made here goes once the last one is back.
      flush(link)
      return
    case 'join': {
      const peers = useCollab.getState()[link.tabId]?.peers ?? []
      const peer = peerOf(msg)
      // Someone already here is renamed (the host who shared what only their agents were in).
      const known = peers.some((p) => p.client === peer.client)
      update(link.tabId, { peers: known ? peers.map((p) => (p.client === peer.client ? peer : p)) : [...peers, peer] })
      if (known) setPlaces(link.tabId, (now) => (now[peer.client] ? { ...now, [peer.client]: { ...now[peer.client], peer } } : now))
      else {
        arrive(link.tabId, peer, true)
        link.gone.delete(peer.client)
        came(link, peer, true)
      }
      // Someone new hasn't seen where this copy is.
      link.presence = null
      if (link.live && link.presenceTimer === null) link.presenceTimer = window.setTimeout(() => sendPresence(link), 0)
      return
    }
    case 'leave': {
      const peers = useCollab.getState()[link.tabId]?.peers ?? []
      const gone = peers.find((p) => p.client === msg.client)
      update(link.tabId, { peers: peers.filter((p) => p.client !== msg.client) })
      if (gone) {
        arrive(link.tabId, gone, false)
        link.gone.set(gone.client, gone)
        came(link, gone, false)
      }
      setPlaces(link.tabId, (now) => {
        const { [msg.client as string]: _gone, ...rest } = now
        return rest
      })
      return
    }
    case 'presence': {
      const peer = useCollab.getState()[link.tabId]?.peers.find((p) => p.client === msg.client)
      if (peer) place(link.tabId, peer, msg.data)
      return
    }
    case 'say':
      heard(link, msg, false)
      return
    case 'end': {
      const host = hostOf(link.tabId)
      finish(link, 'ended', 'Sharing has stopped. This is your own copy now; save it to keep it.')
      void hostGone(link, host, true)
      return
    }
    case 'error': {
      const code = msg.code as string
      if (code === 'bad-message' && link.welcomed) return
      return finish(link, 'disconnected', ERRORS[code] ?? (msg.message as string) ?? 'Something went wrong with sharing.')
    }
  }
}

/** An agent's copy keeps at most this many unread messages; the oldest go first. */
const INBOX = 200

/** A chat message, from the relay's `say` or its welcome's `said`: for an agent's copy that
 * listens, to read; for a person's tab, to show. */
function heard(link: Link, msg: Record<string, unknown>, earlier: boolean) {
  if (typeof msg.text !== 'string' || typeof msg.client !== 'string' || msg.client === link.client) return
  if (link.agent && !link.listens) return
  const from = useCollab.getState()[link.tabId]?.peers.find((p) => p.client === msg.client)
  // A relay before 0.15.3 doesn't say who sent it; then it's whoever is here with that client.
  const sender = typeof msg.name === 'string' ? peerOf({ ...msg, agent: typeof msg.agent === 'string' ? msg.agent : 'ai' }) : null
  const said: Said = {
    client: msg.client,
    name: from?.name ?? sender?.name ?? 'An agent that has left',
    agent: from?.agent ?? sender?.agent ?? 'ai',
    text: msg.text,
    at: Date.now(),
    ...(earlier ? { earlier } : {}),
  }
  if (!link.agent) {
    const chat = useChat.getState()[link.tabId] ?? { said: [], seen: 0 }
    const all = [...chat.said, said]
    const drop = Math.max(0, all.length - CHAT_KEPT)
    useChat.setState({ [link.tabId]: { said: all.slice(drop), seen: Math.max(0, chat.seen - drop) } })
    return
  }
  link.inbox.push(said)
  if (link.inbox.length > INBOX) link.inbox.splice(0, link.inbox.length - INBOX)
  link.chat.push(said)
  if (link.chat.length > INBOX) link.chat.splice(0, link.chat.length - INBOX)
  for (const f of saidListeners.get(link.tabId) ?? []) f(said)
}

/** Heard when an agent's copy gets a message, by the copy's id. */
const saidListeners = new Map<string, Set<(m: Said) => void>>()

/** Calls `f` each time the agent's copy `id` gets a message. Returns the undo. */
export function onSaid(id: string, f: (m: Said) => void): () => void {
  const set = saidListeners.get(id) ?? new Set()
  set.add(f)
  saidListeners.set(id, set)
  return () => {
    set.delete(f)
    if (!set.size) saidListeners.delete(id)
  }
}

/** Someone came into or left a document an agent's copy is in. */
export interface Coming {
  peer: Peer
  joined: boolean
  /** When (Unix ms). */
  at: number
}

/** Heard when someone comes or goes, by an agent copy's id. */
const comingListeners = new Map<string, Set<(c: Coming) => void>>()

/** For an agent's copy: someone came or went, to tell its agent with its next answer. */
function came(link: Link, peer: Peer, joined: boolean) {
  if (!link.agent || !link.live) return
  const c = { peer, joined, at: Date.now() }
  link.comings.push(c)
  if (link.comings.length > 50) link.comings.splice(0, link.comings.length - 50)
  for (const f of comingListeners.get(link.tabId) ?? []) f(c)
}

/** Calls `f` each time someone comes into or leaves the document of agent's copy `id`. Returns the undo. */
export function onComing(id: string, f: (c: Coming) => void): () => void {
  const set = comingListeners.get(id) ?? new Set()
  set.add(f)
  comingListeners.set(id, set)
  return () => {
    set.delete(f)
    if (!set.size) comingListeners.delete(id)
  }
}

/** Who came or went since an agent's copy `id` last heard; `take` marks them heard. */
export function comings(id: string, take = false): Coming[] {
  const link = links.get(id)
  if (!link) return []
  const out = link.comings
  if (take) link.comings = []
  return out
}

/** The chat an agent's copy has seen and said, oldest first. */
export function chatOf(id: string): Said[] {
  return links.get(id)?.chat ?? []
}

/** The name of whoever has client id `client` in shared document `id`, here or gone; null if nobody knows. */
export function nameOf(id: string, client: string): string | null {
  const link = links.get(id)
  const share = useCollab.getState()[id]
  return share?.peers.find((p) => p.client === client)?.name ?? link?.gone.get(client)?.name ?? null
}

/** Heard after someone else's change is applied, by a shared document's id. */
const appliedListeners = new Map<string, Set<(client: string, seq: number, left: number) => void>>()

/**
 * Calls `f` after each change someone else made has been applied to document `id`, with whose it
 * was and the relay's number for it: the changes from before this copy joined too, with how many
 * of those are still to be applied. Returns the undo.
 */
export function onApplied(id: string, f: (client: string, seq: number, left: number) => void): () => void {
  const set = appliedListeners.get(id) ?? new Set()
  set.add(f)
  appliedListeners.set(id, set)
  return () => {
    set.delete(f)
    if (!set.size) appliedListeners.delete(id)
  }
}

/** The messages an agent's copy hasn't read; `take` marks them read. */
export function unread(id: string, take = false): Said[] {
  const link = links.get(id)
  if (!link) return []
  const out = link.inbox
  if (take) link.inbox = []
  return out
}

/**
 * Whether an agent's copy can send messages: it's live and the relay passes them on. A relay in
 * Writan before 0.15.2 doesn't.
 */
export function canSay(id: string): boolean {
  const link = links.get(id)
  return !!link && link.live && !link.closed && link.features.includes('say')
}

/** The engine of a shared tab or agent's copy, once it's open. */
export function engineOf(id: string): CollabEngine | null {
  return links.get(id)?.engine ?? null
}

/** The relay's client id of a shared tab or agent's copy, while it's shared. */
export function clientOf(id: string): string | null {
  return links.get(id)?.client ?? null
}

/** The relay's number of the last change a shared tab or agent's copy has applied; -1 if it isn't shared. */
export function appliedSeq(id: string): number {
  return links.get(id)?.seq ?? -1
}

/** Sends a message from an agent's copy to the document's chat. */
export function say(id: string, text: string): boolean {
  const link = links.get(id)
  if (!link || !canSay(id) || !link.transport) return false
  link.transport.send(JSON.stringify({ t: 'say', text }))
  link.chat.push({ client: link.client, name: link.name, agent: link.agent ?? 'ai', text, at: Date.now(), mine: true })
  if (link.chat.length > INBOX) link.chat.splice(0, link.chat.length - INBOX)
  return true
}

/** Whether the relay puts agents' messages in one chat that people see too (Writan 0.15.3 on). */
export function chatShown(id: string): boolean {
  return !!links.get(id)?.features.includes('chat')
}

/** This app's agents get other agents' messages from now on, or stop: the others are told. */
export function setAgentsListen(on: boolean) {
  for (const link of agentLinks) {
    if (link.closed || link.listens === on) continue
    link.listens = on
    if (!on) link.inbox = []
    // Once its hello is out (it goes as soon as there's a connection); before that, the hello says it.
    link.transport?.send(JSON.stringify({ t: 'listen', on }))
  }
}

/** The tab sharing `session`. */
function hostTab(session: string): string | null {
  for (const [tabId, link] of links) if (link.role === 'host' && link.session === session && !link.closed) return tabId
  return null
}

onHostEvents({
  joinRequest(req) {
    const tabId = hostTab(req.session)
    const now = tabId ? useCollab.getState()[tabId] : null
    if (!tabId || !now) return
    const others = (now.waiting ?? []).filter((r) => r.request !== req.request)
    update(tabId, { waiting: req.waiting ? [...others, req] : others })
  },
})

/** Gives a shared tab's code ten more minutes, as a new code; the old one stops working. */
export async function newCode(tabId: string): Promise<void> {
  const link = links.get(tabId)
  if (!link || link.role !== 'host' || !link.session) return
  const { code, expiresAt } = await rotateCode(link.session)
  update(tabId, { code, expiresAt, message: null })
}

/** Whether guests from the internet wait for the host to let them in, from now on. */
export async function requireApproval(tabId: string, on: boolean): Promise<void> {
  const link = links.get(tabId)
  if (!link || link.role !== 'host' || !link.session) return
  await setRequireApproval(link.session, on)
  update(tabId, { requireApproval: on })
}

/** Lets someone waiting into a shared tab, or doesn't. */
export async function letIn(tabId: string, request: string, allow: boolean): Promise<void> {
  const now = useCollab.getState()[tabId]
  if (now?.waiting) update(tabId, { waiting: now.waiting.filter((r) => r.request !== request) })
  // They may have stopped waiting already; then there's nothing to answer.
  await approveJoin(request, allow).catch(() => {})
}

/** What the host's panel shows about how people join. */
function shown(p: Published): Partial<Share> {
  return { invite: p.invite, code: p.code, addresses: p.addresses, internet: p.internet, expiresAt: p.expiresAt, requireApproval: p.requireApproval, internetError: p.internetError }
}

/**
 * Shares a tab's document: in the app, with anyone who has the code (who wait to be let in when
 * `approval`); in a browser, through the relay. A document this computer's agents are in already
 * gets a code, and they stay.
 */
export async function share(tabId: string, name: string, approval = true): Promise<void> {
  const link = links.get(tabId)
  // Asked again while it gets its code (Enter pressed twice): the same code.
  if (link?.publishing) return link.publishing
  if (link?.role === 'host' && useCollab.getState()[tabId]?.private) {
    // It stays open while it gets its code, even if the last agent leaves meanwhile.
    link.publishing = (async () => {
      try {
        await opened(link)
        const p = await publish(link.session!, link.invite!, approval)
        if (link.closed) {
          // Stopped meanwhile: the code it just got goes too.
          void unhost(link.session!)
          throw new Error('Sharing was stopped.')
        }
        if (link.name !== name) rename(link, name)
        update(tabId, { ...shown(p), private: false, waiting: [], message: null })
      } finally {
        link.publishing = undefined
        closeIfNoAgents(tabId)
      }
    })()
    return link.publishing
  }
  await startHosting(tabId, name, approval, false)
}

/** The host takes the name they typed to share, and their agents become theirs by it. */
function rename(link: Link, name: string) {
  link.name = name
  link.transport?.send(JSON.stringify({ t: 'rename', name }))
  const you = useCollab.getState()[link.tabId]?.you
  if (you) update(link.tabId, { you: { ...you, name } })
  for (const agent of agentLinks) {
    if (agent.follows !== link.tabId || agent.closed) continue
    agent.owner = name
    agent.transport?.send(JSON.stringify({ t: 'rename', name: agent.name, owner: name }))
  }
}

/** How long agents wait for a document to open to them. */
const OPENING = 15_000

/**
 * Lets this computer's agents into a tab's document that isn't shared: a session with no code,
 * which only they join, until Share gives it one. Resolves once they can join.
 */
export async function openToAgents(tabId: string): Promise<void> {
  const until = Date.now() + OPENING
  // A tab nobody has looked at yet loads its editor in the background first.
  if (!getEditor(tabId)) wakeTab(tabId)
  while (!getEditor(tabId) && Date.now() < until && useWorkspace.getState().tabs.some((t) => t.id === tabId)) await new Promise((r) => setTimeout(r, 25))
  if (!links.has(tabId)) await startHosting(tabId, displayName() || 'The person sharing', true, true)
  const link = links.get(tabId)
  if (!link) throw new Error(getEditor(tabId) ? 'Writan can’t open this document to agents.' : 'The document didn’t open in time.')
  await opened(link, until)
}

/** Waits for a session's relay to have the document, so others can join it. */
async function opened(link: Link, until = Date.now() + OPENING) {
  while (!link.closed && !link.welcomed && Date.now() < until) await new Promise((r) => setTimeout(r, 25))
  if (link.closed) throw new Error(useCollab.getState()[link.tabId]?.message ?? 'The document closed.')
  if (!link.welcomed) throw new Error('Opening the document took too long.')
}

/** Starts a tab's session. A `private` one is for this computer's agents only (`openToAgents`). */
async function startHosting(tabId: string, name: string, approval: boolean, isPrivate: boolean): Promise<void> {
  const tab = useWorkspace.getState().tabs.find((t) => t.id === tabId)
  if (!tab || !getEditor(tabId) || !SHAREABLE.has(tab.kind) || links.has(tabId)) return
  const link: Link = { ...newLink(name), tabId, role: 'host' }
  links.set(tabId, link)
  useCollab.setState({ [tabId]: { role: 'host', status: 'connecting', you: null, invite: null, code: null, addresses: [], peers: [], message: null, private: isPrivate } })
  const title = `${tab.title}.${tab.kind}`
  try {
    const hosting = await host(title, tab.kind, approval, isPrivate)
    link.session = hosting.session
    link.invite = hosting.local
    update(tabId, { ...(hosting.published ? shown(hosting.published) : {}), waiting: [] })
    // Everyone starts from these bytes, the host too: reloading makes the host's document what
    // the others read from them. Taken last, and again if the document changed while they were
    // written (Word adds its comments asynchronously), so nothing typed is lost.
    let bytes: Uint8Array | null = null
    let unsaved = false
    for (let tries = 0; !bytes; tries++) {
      const editor = getEditor(tabId)
      if (link.closed || !editor) throw new Error('The tab closed before sharing started.')
      if (tries === 5) throw new Error('The document kept changing. Try sharing again.')
      const now = useWorkspace.getState().tabs.find((t) => t.id === tabId)
      // What the others start from is what's saved: anything done since the panel saved it
      // (while the relay started) is saved first. Agents alone start from what's there, saved
      // or not, and it stays unsaved.
      if (isPrivate) unsaved = !!now?.dirty
      else if (now?.path && now.dirty && !((await useWorkspace.getState().save(tabId)) && !useWorkspace.getState().tabs.find((t) => t.id === tabId)?.dirty)) {
        throw new Error('Save the document to share it. Everyone edits the saved file with you.')
      }
      editor.prepareSave?.()
      const revision = editor.getRevision?.()
      const written = await editor.getBytes()
      if (getEditor(tabId) === editor && editor.getRevision?.() === revision) bytes = written
    }
    if (link.closed) throw new Error('Sharing was stopped.')
    useWorkspace.getState().reloadBytes(tabId, bytes, unsaved)
    const transport = await connect(hosting.local.url, {
      message: (data) => onMessage(link, data),
      closed: (reason) => finish(link, 'disconnected', `Sharing stopped: ${reason}`),
    })
    if (link.closed) {
      transport.close()
      return
    }
    link.transport = transport
    hello(link, hosting.local.token, { hostKey: hosting.hostKey, snapshot: { name: title, kind: tab.kind, bytes: toBase64(bytes) } })
  } catch (err) {
    if (link.session) void unhost(link.session)
    if (link.closed) return
    finish(link, 'disconnected', String((err as Error)?.message ?? err))
    throw err
  }
}

/**
 * Joins a document someone shared, from its six-digit code or its link: opens it in a new tab
 * once it arrives.
 */
export async function join(codeOrLink: string, name: string, cancelled: () => boolean = () => false): Promise<void> {
  const code = parseCode(codeOrLink)
  const found = code ? await findInvite(code) : { invite: parseInvite(codeOrLink), link: codeOrLink.trim() }
  if (cancelled()) return
  const invite = found.invite
  if (!invite) throw new Error('That isn’t a code or a link. A code is six digits.')
  const tabId = await follow(invite, newLink(name), ({ bytes, name: fileName, kind }) => {
    // Given up on (the dialog closed) while connecting: leave without opening anything.
    if (cancelled()) throw new Error('Cancelled.')
    const file = fileName.endsWith(`.${kind}`) ? fileName : `${stripExtension(fileName)}.${kind}`
    const id = SHAREABLE.has(kind) ? useWorkspace.getState().openUnsaved(bytes, file) : null
    if (!id) throw new Error('Writan can’t open this kind of document.')
    return id
  })
  update(tabId, { invite: found.link })
}

/** A shared document as the relay first sends it. */
export interface Opened {
  bytes: Uint8Array
  name: string
  kind: string
}

/**
 * Follows a shared document as `link`: connects, says hello, and once the relay sends the
 * document, `open` makes a place for it (a tab, or an agent's copy) and returns its id. The
 * place's engine then calls `attach` with that id. Resolves once it's opened.
 */
async function follow(invite: Invite, link: Link, open: (doc: Opened) => string, extra: Record<string, unknown> = {}): Promise<string> {
  link.invite = invite
  let first = true
  return new Promise<string>((resolve, reject) => {
    const opened = (doc: Opened) => {
      first = false
      try {
        const id = open(doc)
        resolve(id)
        return id
      } catch (err) {
        reject(err)
        return null
      }
    }
    const events = {
      message: (data: string) => {
        if (first) {
          // Until the welcome, an error means the link didn't work.
          const msg = JSON.parse(data) as { t: string; code?: string; message?: string }
          if (msg.t === 'error') {
            link.closed = true
            link.transport?.close()
            reject(new Error(ERRORS[msg.code ?? ''] ?? msg.message ?? 'Couldn’t join.'))
            return
          }
          onMessage(link, data, opened)
          return
        }
        onMessage(link, data)
      },
      closed: (reason: string) => {
        if (first) reject(new Error(`Couldn’t join: ${reason}`))
        else if (!link.closed) {
          const host = hostOf(link.tabId)
          finish(link, 'disconnected', 'Lost the connection to the person sharing. Your copy is still here; save it to keep your changes.')
          void hostGone(link, host, false)
        }
      },
    }
    // Over the internet, the token comes once the host has let this copy in. An agent asks with
    // its person's seat.
    const as = link.agent && invite.seat ? { seat: invite.seat, agent: link.agent } : undefined
    const opening = invite.p2p ? connectP2p(invite.url, link.name, events, as) : connect(invite.url, events).then((transport) => ({ transport, token: invite.token, seat: '' }))
    link.abort = (reason) => {
      if (first) reject(new Error(reason))
    }
    opening
      .then(({ transport, token, seat }) => {
        // Given up on while connecting or waiting to be let in.
        if (link.closed) return transport.close()
        link.transport = transport
        if (!link.agent && seat) link.seat = seat
        hello(link, token, extra)
      })
      .catch(reject)
  })
}

/** Heard when someone else changes a shared document, by its id (a tab's, or an agent copy's). */
const changeListeners = new Map<string, Set<(client: string) => void>>()

/** Calls `f` with who it was each time someone else changes the document `id`. Returns the undo. */
export function onOthersChange(id: string, f: (client: string) => void): () => void {
  const set = changeListeners.get(id) ?? new Set()
  set.add(f)
  changeListeners.set(id, set)
  return () => {
    set.delete(f)
    if (!set.size) changeListeners.delete(id)
  }
}

/**
 * Where this app connects to a shared tab's document, so an agent can join it too. A document
 * joined over the internet needs the seat the host gave this app, and the host lets the agent in
 * like a person; one from a host without seats (Writan 0.14) can't be joined.
 */
export function inviteOf(tabId: string): Invite | null {
  const link = links.get(tabId)
  if (!link || link.closed || !link.invite) return null
  if (!link.invite.p2p) return link.invite
  return link.seat ? { ...link.invite, seat: link.seat } : null
}

/**
 * Joins the document a tab shares or follows as an AI agent: a copy with no tab, whose engine
 * `open` makes. The agent shows to everyone with its name, color and logo, and to everyone on
 * other computers as the tab's person's (“Eli’s Claude”). Resolves to the copy's id, which
 * `useCollab` and `usePresence` use like a tab's.
 */
export async function followAsAgent(tabId: string, who: { name: string; color: string; agent: string; listens?: boolean }, open: (id: string, doc: Opened) => void): Promise<string> {
  const invite = inviteOf(tabId)
  const owner = links.get(tabId)
  if (!invite || !owner) throw new Error('This document isn’t open to agents.')
  const link = newLink(who.name)
  link.agent = who.agent
  link.listens = !!who.listens
  link.owner = owner.name
  link.follows = tabId
  const id = `agent:${link.client}`
  ownAgents.add(link.client)
  agentLinks.add(link)
  try {
    return await follow(
      invite,
      link,
      (doc) => {
        open(id, doc)
        return id
      },
      { color: who.color },
    )
  } catch (err) {
    link.closed = true
    agentLinks.delete(link)
    ownAgents.delete(link.client)
    closeIfNoAgents(tabId)
    throw err
  }
}

/** An agent's copy leaves its document. */
export function leaveAsAgent(id: string) {
  const link = links.get(id)
  if (link) {
    link.transport?.send(JSON.stringify({ t: 'bye' }))
    finish(link, 'ended', null)
  }
  if (!useCollab.getState()[id]) return
  useCollab.setState((s) => {
    const { [id]: _gone, ...rest } = s
    return rest
  }, true)
}

/** Stops sharing (the host) or leaves (a guest). The tab stays open. */
export function stop(tabId: string) {
  useChat.setState((s) => {
    const { [tabId]: _gone, ...rest } = s
    return rest
  }, true)
  const link = links.get(tabId)
  if (!link) {
    useCollab.setState((s) => {
      const { [tabId]: _gone, ...rest } = s
      return rest
    }, true)
    return
  }
  link.transport?.send(JSON.stringify({ t: link.role === 'host' ? 'end' : 'bye' }))
  if (link.role === 'host' && link.session) void unhost(link.session)
  finish(link, 'ended', null)
  useCollab.setState((s) => {
    const { [tabId]: _gone, ...rest } = s
    return rest
  }, true)
}

/** Connects a tab's engine, once it's open, to the tab's sharing. Returns the undo. */
export function attach(tabId: string, engine: CollabEngine): () => void {
  const link = links.get(tabId)
  if (!link || link.closed) return () => {}
  link.engine = engine
  engineTabs.set(engine, tabId)
  // From now on changes here are kept to send, even made before the relay's welcome.
  engine.doc.collabStart(link.client)
  engine.collab = { changed: () => flush(link) }
  start(link)
  return () => {
    if (link.engine !== engine) return
    // The tab closed or loaded other bytes (revert, an older version): this copy can't follow
    // the others any more.
    link.engine = null
    engine.collab = null
    if (link.closed) return
    if (link.role === 'host') stop(tabId)
    else {
      link.transport?.send(JSON.stringify({ t: 'bye' }))
      finish(link, 'ended', null)
    }
  }
}

/** Who keeps the file of a tab following someone else's shared document, while it follows. */
export function keeperOf(tabId: string): string | null {
  const share = useCollab.getState()[tabId]
  if (share?.role !== 'guest' || (share.status !== 'live' && share.status !== 'connecting')) return null
  return share.peers.find((p) => p.host)?.name || 'The person sharing'
}
collabHooks.keeper = keeperOf
collabHooks.hosting = (tabId) => {
  const share = useCollab.getState()[tabId]
  return share?.role === 'host' && !share.private && (share.status === 'live' || share.status === 'connecting')
}

/** For a tab following someone else's shared document: waits up to a few seconds for every
 * change made here to reach the relay, which sends it on to them. True once none is left. */
async function settled(tabId: string): Promise<boolean> {
  const link = links.get(tabId)
  if (!link || link.role !== 'guest' || keeperOf(tabId) === null) return false
  const until = Date.now() + 3000
  for (;;) {
    // What's being typed in a cell counts too: commit it, so it goes.
    getEditor(tabId)?.prepareSave?.()
    if (link.closed || !link.engine) return false
    if (!link.engine.doc.collabPending()) return true
    if (Date.now() > until) return false
    await new Promise((r) => setTimeout(r, 50))
  }
}
collabHooks.settled = settled

// A shared tab that closes, however it closes (before its document finished opening too), ends
// its sharing.
useWorkspace.subscribe((state, prev) => {
  if (state.tabs === prev.tabs) return
  for (const tabId of [...links.keys()]) {
    // An agent's copy has no tab; it leaves when the agent does.
    if (!tabId.startsWith('agent:') && !state.tabs.some((t) => t.id === tabId)) stop(tabId)
  }
})
