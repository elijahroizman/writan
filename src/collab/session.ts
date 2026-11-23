// Sharing a Word document, presentation or workbook so others can edit it at the same time.
//
// The person who shares hosts the session: their copy stays the file on disk, saved as usual,
// and everyone else's tab holds a copy that follows it. Every copy, the host's included, sends
// its changes to the relay, which puts them in one order and sends each to everyone. The engine
// does the rest (crates/office-engine/src/collab.rs): it applies changes in that order, so the
// copies agree, and drops a change that collides with an earlier one.
//
// In the desktop app, internet sharing runs encrypted P2P (`iroh` + SPAKE2 + 6-digit code
// directory Worker) bridged into the same relay, with LAN discovery as a local fast path.

import { create } from 'zustand'
import { showDialog } from '../lib/dialogs'
import { stripExtension } from '../lib/kinds'
import { collabHooks, getEditor, useWorkspace } from '../state/workspace'
import {
  approveJoin,
  connect,
  connectP2p,
  findInvite,
  host,
  p2pReconnectUrl,
  parseCode,
  parseInvite,
  rotateCode,
  setHostEventHandlers,
  setRequireApproval,
  unhost,
  type Invite,
  type JoinRequest,
  type Transport,
} from './transport'

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
  /** Whether encrypted internet P2P sharing is active. */
  internet?: boolean
  /** Unix epoch ms when the current 6-digit internet code expires. */
  expiresAt?: number | null
  /** Whether internet joiners must be approved by the host. */
  requireApproval?: boolean
  /** If internet sharing couldn't start (e.g. offline), why it fell back to LAN. */
  internetError?: string | null
  /** Guests waiting for the host's approval to join. */
  pendingJoins?: JoinRequest[]
}

/** Shared tabs, by tab id. */
export const useCollab = create<Record<string, Share>>(() => ({}))

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
  return {
    tabId: '',
    role: 'guest',
    client,
    name,
    transport: null,
    engine: null,
    queue: [],
    welcomed: false,
    live: false,
    closed: false,
    presence: null,
    presenceTimer: null,
    invite: null,
    lastSeq: 0,
    reconnecting: false,
    reconnectTimer: null,
    reconnectAttempts: 0,
    reconnectExtra: {},
  }
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
  /** Changes that arrived before the engine was ready, in order. */
  queue: string[]
  welcomed: boolean
  live: boolean
  closed: boolean
  /** The last presence sent, and a send waiting to go. */
  presence: string | null
  presenceTimer: number | null
  /** Where this copy connects (and reconnects with its authenticated token). */
  invite: Invite | null
  /** For an agent's copy: which agent, sent in the hello. */
  agent?: string
  /** Highest relay `seq` seen so far, used for `sinceSeq` resume on reconnect. */
  lastSeq: number
  reconnecting: boolean
  reconnectTimer: number | null
  reconnectAttempts: number
  reconnectExtra: Record<string, unknown>
}

/** Moving the caret sends where it is at most this often. */
const PRESENCE_EVERY = 50
const MAX_RECONNECT_ATTEMPTS = 5

const links = new Map<string, Link>()

function update(tabId: string, patch: Partial<Share>) {
  const now = useCollab.getState()[tabId]
  if (now) useCollab.setState({ [tabId]: { ...now, ...patch } })
}

function tabForSession(session: string): string | null {
  for (const [tabId, link] of links) {
    if (link.role === 'host' && link.session === session && !link.closed) return tabId
  }
  return null
}

// Wire host-side P2P events from Rust (join approval requests & code rotations).
setHostEventHandlers({
  onJoinRequest: (req) => {
    const tabId = tabForSession(req.session)
    if (!tabId) return
    const cur = useCollab.getState()[tabId]
    if (!cur) return
    const existing = cur.pendingJoins ?? []
    if (existing.some((r) => r.requestId === req.requestId)) return
    update(tabId, { pendingJoins: [...existing, req] })
  },
  onCodeChanged: (upd) => {
    const tabId = tabForSession(upd.session)
    if (!tabId) return
    update(tabId, {
      code: upd.code,
      expiresAt: upd.expiresAt,
      ...(upd.rotatedByLockout ? { message: 'Code rotated after too many wrong guesses.' } : {}),
    })
  },
})

function hello(link: Link, token: string, extra: Record<string, unknown> = {}) {
  // The color asked for, from the name so a person tends to keep theirs from one document to
  // the next; the relay gives another if someone has it. An agent asks for its own.
  const color = link.agent ? (extra.color as string) : colorFor(link.name || link.client)
  link.transport?.send(
    JSON.stringify({
      t: 'hello',
      v: 1,
      token,
      client: link.client,
      name: link.name,
      ...extra,
      color,
      ...(link.agent ? { agent: link.agent } : {}),
    }),
  )
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

function peerOf(p: Record<string, unknown>): Peer {
  return {
    client: p.client as string,
    name: p.name as string,
    color: p.color as string,
    host: !!p.host,
    ...(typeof p.agent === 'string' ? { agent: p.agent } : {}),
  }
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
  for (const op of link.queue) engine.doc.collabReceive(op)
  link.queue = []
  engine.refresh()
  flush(link)
  update(link.tabId, { status: 'live', message: null })
}

/** Stops sharing on this copy: it goes on as an ordinary unsaved document. */
function finish(link: Link, status: Share['status'], message: string | null) {
  if (link.closed) return
  link.closed = true
  link.live = false
  link.reconnecting = false
  if (link.reconnectTimer !== null) window.clearTimeout(link.reconnectTimer)
  link.reconnectTimer = null
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
  update(link.tabId, { status, message, peers: [], pendingJoins: [] })
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
      if (typeof msg.seq === 'number' && msg.seq > link.lastSeq) link.lastSeq = msg.seq
      const resumed = !!msg.resumed
      // An older relay doesn't hand out colors; then this copy keeps the one it asked for.
      const you: Peer = { client: link.client, name: link.name, color: (msg.color as string) || colorFor(link.name || link.client), host: link.role === 'host' }
      if (opened && !resumed) {
        const session = msg.session as { name: string; kind: string }
        const tabId = opened({ bytes: fromBase64(msg.snapshot as string), name: session.name, kind: session.kind })
        if (!tabId) return finish(link, 'disconnected', 'Writan can’t open this kind of document.')
        link.tabId = tabId
        links.set(tabId, link)
        useCollab.setState({ [tabId]: { role: 'guest', status: 'connecting', you, invite: null, code: null, addresses: [], peers, message: null } })
      } else {
        update(link.tabId, { peers, you, status: 'live', message: null })
      }
      link.reconnecting = false
      link.reconnectAttempts = 0
      raw.forEach((p, i) => p.presence != null && place(link.tabId, peers[i], p.presence))
      const missed = (msg.ops as Record<string, unknown>[]).map((op) => {
        if (typeof op.seq === 'number' && op.seq > link.lastSeq) link.lastSeq = op.seq
        return JSON.stringify(op)
      })
      if (resumed && link.engine) {
        let changed = false
        for (const op of missed) {
          if (link.engine.doc.collabReceive(op)) changed = true
        }
        if (changed) link.engine.refresh()
        link.live = true
        link.presence = null
        flush(link)
      } else {
        link.queue.push(...missed)
        link.welcomed = true
        start(link)
      }
      return
    }
    case 'op':
      if (typeof msg.seq === 'number' && msg.seq > link.lastSeq) link.lastSeq = msg.seq
      if (msg.client !== link.client) for (const f of changeListeners.get(link.tabId) ?? []) f(msg.client as string)
      if (!link.live || !link.engine) {
        link.queue.push(data)
        return
      }
      if (link.engine.doc.collabReceive(data)) link.engine.refresh()
      // The next change made here goes once the last one is back.
      flush(link)
      return
    case 'join': {
      const peers = useCollab.getState()[link.tabId]?.peers ?? []
      const peer = peerOf(msg)
      update(link.tabId, { peers: [...peers.filter((p) => p.client !== peer.client), peer] })
      if (!peers.some((p) => p.client === peer.client)) arrive(link.tabId, peer, true)
      // Someone new hasn't seen where this copy is.
      link.presence = null
      if (link.live && link.presenceTimer === null) link.presenceTimer = window.setTimeout(() => sendPresence(link), 0)
      return
    }
    case 'leave': {
      const peers = useCollab.getState()[link.tabId]?.peers ?? []
      const gone = peers.find((p) => p.client === msg.client)
      update(link.tabId, { peers: peers.filter((p) => p.client !== msg.client) })
      if (gone) arrive(link.tabId, gone, false)
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
    case 'end': {
      const hostName = hostOf(link.tabId)
      finish(link, 'ended', 'Sharing has stopped. This is your own copy now; save it to keep it.')
      void hostGone(link, hostName, true)
      return
    }
    case 'error': {
      const code = msg.code as string
      if (code === 'bad-message' && link.welcomed) return
      return finish(link, 'disconnected', ERRORS[code] ?? (msg.message as string) ?? 'Something went wrong with sharing.')
    }
  }
}

/** Schedules a guest reconnect with exponential backoff and `sinceSeq` resume. */
function scheduleReconnect(link: Link) {
  if (link.closed || link.role !== 'guest' || !link.invite || !link.invite.token) return
  link.live = false
  link.transport = null
  if (link.reconnectAttempts >= MAX_RECONNECT_ATTEMPTS) {
    const hostName = hostOf(link.tabId)
    finish(link, 'disconnected', 'Lost the connection to the person sharing. Your copy is still here; save it to keep your changes.')
    void hostGone(link, hostName, false)
    return
  }
  link.reconnecting = true
  const delay = Math.min(30_000, 1000 * 2 ** link.reconnectAttempts)
  link.reconnectAttempts++
  update(link.tabId, { status: 'connecting', message: 'Reconnecting…' })
  link.reconnectTimer = window.setTimeout(() => {
    link.reconnectTimer = null
    if (link.closed || !link.invite) return
    const invite = link.invite
    const events = {
      message: (data: string) => onMessage(link, data),
      closed: () => {
        if (!link.closed) scheduleReconnect(link)
      },
    }
    const open = invite.p2p
      ? connectP2p(invite.url, link.name, events).then((r) => r.transport)
      : connect(invite.url, events)
    open
      .then((t) => {
        if (link.closed) {
          t.close()
          return
        }
        link.transport = t
        hello(link, invite.token, { ...link.reconnectExtra, sinceSeq: link.lastSeq })
      })
      .catch(() => {
        if (!link.closed) scheduleReconnect(link)
      })
  }, delay)
}

/** Shares a tab's document over the internet (with LAN fast-path). */
export async function share(
  tabId: string,
  name: string,
  opts?: { internet?: boolean; requireApproval?: boolean },
): Promise<void> {
  const tab = useWorkspace.getState().tabs.find((t) => t.id === tabId)
  if (!tab || !getEditor(tabId) || !SHAREABLE.has(tab.kind) || links.has(tabId)) return
  const link: Link = { ...newLink(name), tabId, role: 'host' }
  links.set(tabId, link)
  useCollab.setState({
    [tabId]: {
      role: 'host',
      status: 'connecting',
      you: null,
      invite: null,
      code: null,
      addresses: [],
      peers: [],
      message: null,
      internet: opts?.internet ?? true,
      expiresAt: null,
      requireApproval: opts?.requireApproval ?? true,
      internetError: null,
      pendingJoins: [],
    },
  })
  const title = `${tab.title}.${tab.kind}`
  try {
    const hosting = await host(title, tab.kind, opts)
    link.session = hosting.session
    link.invite = hosting.local
    update(tabId, {
      invite: hosting.invite,
      code: hosting.code,
      addresses: hosting.addresses,
      internet: hosting.internet,
      expiresAt: hosting.expiresAt,
      requireApproval: hosting.requireApproval,
      internetError: hosting.internetError,
    })
    // Everyone starts from these bytes, the host too: reloading makes the host's document what
    // the others read from them. Taken last, and again if the document changed while they were
    // written (Word adds its comments asynchronously), so nothing typed is lost.
    let bytes: Uint8Array | null = null
    for (let tries = 0; !bytes; tries++) {
      const editor = getEditor(tabId)
      if (link.closed || !editor) throw new Error('The tab closed before sharing started.')
      if (tries === 5) throw new Error('The document kept changing. Try sharing again.')
      // What the others start from is what's saved: anything done since the panel saved it
      // (while the relay started) is saved first.
      const now = useWorkspace.getState().tabs.find((t) => t.id === tabId)
      if (now?.path && now.dirty && !((await useWorkspace.getState().save(tabId)) && !useWorkspace.getState().tabs.find((t) => t.id === tabId)?.dirty)) {
        throw new Error('Save the document to share it. Everyone edits the saved file with you.')
      }
      editor.prepareSave?.()
      const revision = editor.getRevision?.()
      const written = await editor.getBytes()
      if (getEditor(tabId) === editor && editor.getRevision?.() === revision) bytes = written
    }
    if (link.closed) throw new Error('Sharing was stopped.')
    useWorkspace.getState().reloadBytes(tabId, bytes)
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

/** Rotates an internet-shared tab's 6-digit code and resets its 10-minute window. */
export async function rotateShareCode(tabId: string): Promise<void> {
  const link = links.get(tabId)
  if (!link || link.role !== 'host' || !link.session) return
  const upd = await rotateCode(link.session)
  update(tabId, { code: upd.code, expiresAt: upd.expiresAt, message: null })
}

/** Toggles whether new internet joiners need 1-click host approval. */
export async function setShareRequireApproval(tabId: string, on: boolean): Promise<void> {
  const link = links.get(tabId)
  if (!link || link.role !== 'host' || !link.session) return
  await setRequireApproval(link.session, on)
  update(tabId, { requireApproval: on })
}

/** Approves or denies a pending internet join request. */
export async function approveJoinRequest(tabId: string, requestId: string, allow: boolean): Promise<void> {
  const cur = useCollab.getState()[tabId]
  if (cur?.pendingJoins) {
    update(tabId, { pendingJoins: cur.pendingJoins.filter((r) => r.requestId !== requestId) })
  }
  await approveJoin(requestId, allow)
}

/**
 * Joins a document someone shared, from its six-digit code or its link: opens it in a new tab
 * once it arrives.
 */
export async function join(codeOrLink: string, name: string, cancelled: () => boolean = () => false): Promise<void> {
  const code = parseCode(codeOrLink)
  const inviteText = code ? await findInvite(code) : codeOrLink.trim()
  if (cancelled()) return
  const invite = parseInvite(inviteText)
  if (!invite) throw new Error('That isn’t a code or a link. A code is six digits.')
  const tabId = await follow(invite, newLink(name), ({ bytes, name: fileName, kind }) => {
    // Given up on (the dialog closed) while connecting: leave without opening anything.
    if (cancelled()) throw new Error('Cancelled.')
    const file = fileName.endsWith(`.${kind}`) ? fileName : `${stripExtension(fileName)}.${kind}`
    const id = SHAREABLE.has(kind) ? useWorkspace.getState().openUnsaved(bytes, file) : null
    if (!id) throw new Error('Writan can’t open this kind of document.')
    return id
  })
  update(tabId, { invite: invite.p2p ? null : inviteText, internet: !!invite.p2p })
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
  link.reconnectExtra = extra
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
          if (link.invite?.token && !link.agent) {
            scheduleReconnect(link)
          } else {
            const hostName = hostOf(link.tabId)
            finish(link, 'disconnected', 'Lost the connection to the person sharing. Your copy is still here; save it to keep your changes.')
            void hostGone(link, hostName, false)
          }
        }
      },
    }

    if (invite.p2p) {
      connectP2p(invite.url, link.name, events)
        .then(({ transport, token }) => {
          link.transport = transport
          // Switch stored invite to token mode so reconnect works even after the 10-min code expires.
          link.invite = { url: p2pReconnectUrl(invite.url, token), token, p2p: true }
          hello(link, token, extra)
        })
        .catch(reject)
    } else {
      connect(invite.url, events)
        .then((t) => {
          link.transport = t
          hello(link, invite.token, extra)
        })
        .catch(reject)
    }
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

/** Where this app connects to a shared tab's document, so an agent can join it too. */
export function inviteOf(tabId: string): Invite | null {
  const link = links.get(tabId)
  return link && !link.closed ? link.invite : null
}

/**
 * Joins a shared document as an AI agent: a copy with no tab, whose engine `open` makes. The
 * agent shows to everyone with its name, color and logo. Resolves to the copy's id, which
 * `useCollab` and `usePresence` use like a tab's.
 */
export async function followAsAgent(invite: Invite, who: { name: string; color: string; agent: string }, open: (id: string, doc: Opened) => void): Promise<string> {
  const link = newLink(who.name)
  link.agent = who.agent
  const id = `agent:${link.client}`
  return follow(
    invite,
    link,
    (doc) => {
      open(id, doc)
      return id
    },
    { color: who.color },
  )
}

/** An agent's copy leaves its document. */
export function leaveAsAgent(id: string) {
  const link = links.get(id)
  if (!link) return
  link.transport?.send(JSON.stringify({ t: 'bye' }))
  finish(link, 'ended', null)
  useCollab.setState((s) => {
    const { [id]: _gone, ...rest } = s
    return rest
  }, true)
}

/** Stops sharing (the host) or leaves (a guest). The tab stays open. */
export function stop(tabId: string) {
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
