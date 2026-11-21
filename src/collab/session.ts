// Sharing a Word document, presentation or workbook so others can edit it at the same time.
//
// The person who shares hosts the session: their copy stays the file on disk, saved as usual,
// and everyone else's tab holds a copy that follows it. Every copy, the host's included, sends
// its changes to the relay, which puts them in one order and sends each to everyone. The engine
// does the rest (crates/office-engine/src/collab.rs): it applies changes in that order, so the
// copies agree, and drops a change that collides with an earlier one.
//
// The relay is on the host's computer for now (LAN). Nothing here depends on that: a cloud relay
// speaking the same messages only changes the invite's address (see transport.ts).

import { create } from 'zustand'
import { stripExtension } from '../lib/kinds'
import { getEditor, useWorkspace } from '../state/workspace'
import { connect, host, parseInvite, unhost, type Transport } from './transport'

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
  /** Set while shared: told after every change, to send it. */
  collab: { changed(): void } | null
  refresh(): void
}

export interface Peer {
  client: string
  name: string
  color: string
}

export interface Share {
  role: 'host' | 'guest'
  status: 'connecting' | 'live' | 'ended' | 'disconnected'
  /** The link others join with (the host's to hand out; a guest's to rejoin). */
  invite: string | null
  /** The host's addresses on the network. */
  addresses: string[]
  /** The others in the document. */
  peers: Peer[]
  /** Why it ended or what went wrong. */
  message: string | null
}

/** Shared tabs, by tab id. */
export const useCollab = create<Record<string, Share>>(() => ({}))

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
}

const links = new Map<string, Link>()

function update(tabId: string, patch: Partial<Share>) {
  const now = useCollab.getState()[tabId]
  if (now) useCollab.setState({ [tabId]: { ...now, ...patch } })
}

function hello(link: Link, token: string, extra: Record<string, unknown> = {}) {
  link.transport?.send(JSON.stringify({ t: 'hello', v: 1, token, client: link.client, name: link.name, color: colorFor(link.client), ...extra }))
}

/** Sends what changed here. */
function flush(link: Link) {
  if (!link.live || !link.engine || link.closed) return
  for (let op = link.engine.doc.collabTake(); op; op = link.engine.doc.collabTake()) {
    link.transport?.send(`{"t":"op",${op.slice(1)}`)
  }
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
  update(link.tabId, { status: 'live' })
}

/** Stops sharing on this copy: it goes on as an ordinary unsaved document. */
function finish(link: Link, status: Share['status'], message: string | null) {
  if (link.closed) return
  link.closed = true
  link.live = false
  if (link.engine) {
    link.engine.collab = null
    link.engine.doc.collabStop()
  }
  link.transport?.close()
  link.transport = null
  links.delete(link.tabId)
  update(link.tabId, { status, message, peers: [] })
}

const ERRORS: Record<string, string> = {
  'bad-token': 'This invite link isn’t valid any more. Ask for a new one.',
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
      const peers = (msg.peers as Peer[]).map((p) => ({ client: p.client, name: p.name, color: p.color }))
      if (opened) {
        const session = msg.session as { name: string; kind: string }
        const tabId = opened({ bytes: fromBase64(msg.snapshot as string), name: session.name, kind: session.kind })
        if (!tabId) return finish(link, 'disconnected', 'Writan can’t open this kind of document.')
        link.tabId = tabId
        links.set(tabId, link)
        useCollab.setState({ [tabId]: { role: 'guest', status: 'connecting', invite: null, addresses: [], peers, message: null } })
      } else {
        update(link.tabId, { peers })
      }
      link.queue.push(...(msg.ops as unknown[]).map((op) => JSON.stringify(op)))
      link.welcomed = true
      start(link)
      return
    }
    case 'op':
      if (!link.live || !link.engine) {
        link.queue.push(data)
        return
      }
      if (link.engine.doc.collabReceive(data)) link.engine.refresh()
      return
    case 'join': {
      const peers = useCollab.getState()[link.tabId]?.peers ?? []
      const peer = { client: msg.client as string, name: msg.name as string, color: msg.color as string }
      update(link.tabId, { peers: [...peers.filter((p) => p.client !== peer.client), peer] })
      return
    }
    case 'leave':
      update(link.tabId, { peers: (useCollab.getState()[link.tabId]?.peers ?? []).filter((p) => p.client !== msg.client) })
      return
    case 'end':
      return finish(link, 'ended', 'Sharing has stopped. This is your own copy now; save it to keep it.')
    case 'error': {
      const code = msg.code as string
      if (code === 'bad-message' && link.welcomed) return
      return finish(link, 'disconnected', ERRORS[code] ?? (msg.message as string) ?? 'Something went wrong with sharing.')
    }
  }
}

/** Shares a tab's document on the network. */
export async function share(tabId: string, name: string): Promise<void> {
  const tab = useWorkspace.getState().tabs.find((t) => t.id === tabId)
  if (!tab || !getEditor(tabId) || !SHAREABLE.has(tab.kind) || links.has(tabId)) return
  const link: Link = { tabId, role: 'host', client: newClient(), name, transport: null, engine: null, queue: [], welcomed: false, live: false, closed: false }
  links.set(tabId, link)
  useCollab.setState({ [tabId]: { role: 'host', status: 'connecting', invite: null, addresses: [], peers: [], message: null } })
  const title = `${tab.title}.${tab.kind}`
  try {
    const hosting = await host(title, tab.kind)
    link.session = hosting.session
    update(tabId, { invite: hosting.invite, addresses: hosting.addresses })
    const editor = getEditor(tabId)
    if (link.closed || !editor) throw new Error('The tab closed before sharing started.')
    // Everyone starts from these bytes, the host too: reloading makes the host's document what
    // the others read from them. Taken last, so nothing typed while the relay started is lost.
    editor.prepareSave?.()
    const bytes = await editor.getBytes()
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

/** Joins a document someone shared: opens it in a new tab once it arrives. */
export async function join(inviteText: string, name: string): Promise<void> {
  const invite = parseInvite(inviteText)
  if (!invite) throw new Error('That isn’t an invite link. It starts with ws:// and has a token in it.')
  const link: Link = { tabId: '', role: 'guest', client: newClient(), name, transport: null, engine: null, queue: [], welcomed: false, live: false, closed: false }
  let first = true
  await new Promise<void>((resolve, reject) => {
    const opened = ({ bytes, name: fileName, kind }: { bytes: Uint8Array; name: string; kind: string }) => {
      first = false
      const file = fileName.endsWith(`.${kind}`) ? fileName : `${stripExtension(fileName)}.${kind}`
      const id = SHAREABLE.has(kind) ? useWorkspace.getState().openUnsaved(bytes, file) : null
      if (id) resolve()
      else reject(new Error('Writan can’t open this kind of document.'))
      return id
    }
    connect(invite.url, {
      message: (data) => {
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
          if (link.tabId) update(link.tabId, { invite: inviteText.trim() })
          return
        }
        onMessage(link, data)
      },
      closed: (reason) => {
        if (first) reject(new Error(`Couldn’t join: ${reason}`))
        else finish(link, 'disconnected', 'Lost the connection to the person sharing. Your copy is still here; save it to keep your changes.')
      },
    })
      .then((t) => {
        link.transport = t
        hello(link, invite.token)
      })
      .catch(reject)
  })
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

// A shared tab that closes, however it closes (before its document finished opening too), ends
// its sharing.
useWorkspace.subscribe((state, prev) => {
  if (state.tabs === prev.tabs) return
  for (const tabId of [...links.keys()]) {
    if (!state.tabs.some((t) => t.id === tabId)) stop(tabId)
  }
})
