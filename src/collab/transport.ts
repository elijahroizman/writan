// How a shared document reaches the relay that orders everyone's changes.
//
// In the app the relay runs in Rust (src-tauri/src/collab): sharing starts it on this computer,
// and every connection, the sharer's own included, goes through Rust, since the page's content
// security policy doesn't allow sockets to other machines. Guests from the internet reach the
// sharer's relay through an iroh connection between the two apps (src-tauri/src/collab/p2p.rs),
// found through the code directory. In a plain browser (development and the browser tests) the
// page talks WebSocket to a relay itself, the standalone one in src-tauri/examples/collab_relay.rs,
// which creates a session for whoever shares first. A relay in the cloud would be reached the same
// way.

import { isTauri } from '../lib/platform'

export interface Transport {
  send(data: string): void
  close(): void
}

export interface TransportEvents {
  message(data: string): void
  closed(reason: string): void
}

/** A session someone can join: the relay's address and the secret that lets them in. */
export interface Invite {
  /** `ws://host:port/v1/<session>`, without the token, or `writan-p2p://…` from the directory. */
  url: string
  /** Empty for `writan-p2p://`, whose token comes once the sharer lets this copy in. */
  token: string
  p2p?: boolean
}

/** What sharing a document gives: the invite to pass on and the sharer's own way in. */
export interface Hosting {
  session: string
  invite: string
  /** Six digits guests join with instead of the link: anywhere when `internet`, else on the network. None in a browser. */
  code: string | null
  local: Invite
  hostKey: string
  /** This computer's addresses on the network, for showing. */
  addresses: string[]
  /** Whether the code is in the directory, for guests anywhere. */
  internet: boolean
  /** When the code stops working (Unix ms), when `internet`. */
  expiresAt: number | null
  /** Whether guests from the internet wait for the sharer to let them in. */
  requireApproval: boolean
  /** Why sharing over the internet didn't start, when it didn't. */
  internetError: string | null
}

/** Someone with the code waiting to be let in, or who stopped waiting. */
export interface JoinRequest {
  request: string
  session: string
  name: string
  waiting: boolean
}

const P2P = 'writan-p2p://'

/** Reads an invite link (`ws://192.168.1.20:47811/v1/abc?token=…`). */
export function parseInvite(text: string): Invite | null {
  try {
    const url = new URL(text.trim())
    if (url.protocol !== 'ws:' && url.protocol !== 'wss:') return null
    const token = url.searchParams.get('token')
    if (!token || !/^\/v1\/[A-Za-z0-9_-]+$/.test(url.pathname)) return null
    url.search = ''
    url.hash = ''
    return { url: url.toString(), token }
  } catch {
    return null
  }
}

/** The six digits in what someone typed (`482 913`, `482-913`), or null if it isn't a code. */
export function parseCode(text: string): string | null {
  const digits = text.replace(/[\s-]/g, '')
  return /^\d{6}$/.test(digits) ? digits : null
}

/**
 * Finds who is sharing with `code` and returns their invite, with the link to show for it: from the
 * code directory, or else from the computers nearby (src-tauri/src/collab/mod.rs, `collab_find`).
 */
export async function findInvite(code: string): Promise<{ invite: Invite; link: string | null }> {
  if (!isTauri) throw new Error('In the browser, join with the link instead of a code.')
  const { invoke } = await import('@tauri-apps/api/core')
  const found = await invoke<string>('collab_find', { code })
  // From the directory: only Rust reads it, and there's no link to show for it.
  if (found.startsWith(P2P)) return { invite: { url: found, token: '', p2p: true }, link: null }
  const invite = parseInvite(found)
  if (!invite) throw new Error('The computer sharing sent a link Writan can’t read.')
  return { invite, link: found }
}

function randomId(bytes: number) {
  const b = crypto.getRandomValues(new Uint8Array(bytes))
  return btoa(String.fromCharCode(...b)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

/** The relay a browser shares through. */
export function browserRelay(): string {
  try {
    const stored = localStorage.getItem('workspace.collabRelay')
    if (stored) return stored.replace(/\/+$/, '')
  } catch {
    // Storage can be off; the default relay is fine.
  }
  return (import.meta.env.VITE_COLLAB_RELAY as string | undefined) ?? 'ws://127.0.0.1:47812'
}

let reset: Promise<void> | null = null

/**
 * Forgets the connections and shared sessions a page loaded earlier left in Rust (a reload).
 * Called once when the page starts; sharing and joining wait for it.
 */
export function resetNative() {
  reset ??= isTauri ? import('@tauri-apps/api/core').then(({ invoke }) => invoke<void>('collab_reset')).catch(() => {}) : Promise.resolve()
  return reset
}

/** Starts hosting a session for a document. In the app, guests from the internet wait to be let in when `requireApproval`. */
export async function host(name: string, kind: string, requireApproval: boolean): Promise<Hosting> {
  await resetNative()
  if (isTauri) {
    await listenOnce()
    const { invoke } = await import('@tauri-apps/api/core')
    type Info = { session: string; token: string; hostKey: string; addresses: string[]; inviteUrl: string; code: string; localUrl: string; internet: boolean; expiresAt: number | null; requireApproval: boolean; internetError: string | null }
    const info = await invoke<Info>('collab_share', { name, kind, requireApproval })
    return {
      session: info.session,
      invite: info.inviteUrl,
      code: info.code,
      local: { url: info.localUrl, token: info.token },
      hostKey: info.hostKey,
      addresses: info.addresses,
      internet: info.internet,
      expiresAt: info.expiresAt,
      requireApproval: info.requireApproval,
      internetError: info.internetError,
    }
  }
  const session = randomId(12)
  const token = randomId(24)
  const url = `${browserRelay()}/v1/${session}`
  return { session, invite: `${url}?token=${token}`, code: null, local: { url, token }, hostKey: randomId(24), addresses: [], internet: false, expiresAt: null, requireApproval: false, internetError: null }
}

/** Ends hosting: the relay tells everyone and forgets the session. */
export async function unhost(session: string): Promise<void> {
  if (!isTauri) return
  const { invoke } = await import('@tauri-apps/api/core')
  await invoke('collab_unshare', { session })
}

/** Gives an internet-shared session a new code for ten minutes; the old one stops working. */
export async function rotateCode(session: string): Promise<{ code: string; expiresAt: number }> {
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<{ code: string; expiresAt: number }>('collab_rotate_code', { session })
}

/** Whether guests from the internet wait to be let in, from now on. */
export async function setRequireApproval(session: string, requireApproval: boolean): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core')
  await invoke('collab_set_require_approval', { session, requireApproval })
}

/** Lets someone waiting in, or doesn't. Fails when they've stopped waiting. */
export async function approveJoin(request: string, allow: boolean): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core')
  await invoke('collab_approve_join', { request, allow })
}

/** What the sharer's app says about guests from the internet. */
export interface HostEvents {
  joinRequest(request: JoinRequest): void
}

let hostEvents: HostEvents | null = null

export function onHostEvents(events: HostEvents) {
  hostEvents = events
}

const routes = new Map<number, TransportEvents>()
let listening: Promise<void> | null = null

function listenOnce() {
  listening ??= (async () => {
    const { listen } = await import('@tauri-apps/api/event')
    await listen<{ conn: number; data: string }>('collab-message', (e) => routes.get(e.payload.conn)?.message(e.payload.data))
    await listen<{ conn: number; reason: string }>('collab-closed', (e) => {
      const route = routes.get(e.payload.conn)
      routes.delete(e.payload.conn)
      route?.closed(e.payload.reason)
    })
    await listen<JoinRequest>('collab-join-request', (e) => hostEvents?.joinRequest(e.payload))
  })()
  return listening
}

/** A connection Rust holds, by its id. */
async function native(conn: number, events: TransportEvents): Promise<Transport> {
  routes.set(conn, events)
  const { invoke } = await import('@tauri-apps/api/core')
  const fail = (err: unknown) => {
    if (!routes.delete(conn)) return
    void invoke('collab_close', { conn }).catch(() => {})
    events.closed(String((err as Error)?.message ?? err))
  }
  return {
    send: (data) => void invoke('collab_send', { conn, data }).catch(fail),
    close: () => {
      routes.delete(conn)
      void invoke('collab_close', { conn }).catch(() => {})
    },
  }
}

/**
 * Joins a session found in the directory (`writan-p2p://…`) as `name`. Resolves once the sharer
 * has let this copy in, with the token for the hello.
 */
export async function connectP2p(url: string, name: string, events: TransportEvents): Promise<{ transport: Transport; token: string }> {
  // Numbered before anything waits, so `cancelJoins` covers it from the start, even when Rust
  // hears the cancel first.
  const attempt = ++joinAttempts
  await resetNative()
  await listenOnce()
  const { invoke } = await import('@tauri-apps/api/core')
  const { conn, token } = await invoke<{ conn: number; token: string }>('collab_connect_p2p', { url, name, attempt })
  return { transport: await native(conn, events), token }
}

let joinAttempts = 0

/** Gives up the internet joins started so far (the Join dialog closed). */
export function cancelJoins() {
  const upTo = joinAttempts
  // After the reset the joins wait for too, which would forget it.
  if (isTauri) void resetNative().then(() => import('@tauri-apps/api/core')).then(({ invoke }) => invoke('collab_cancel_joins', { upTo }).catch(() => {}))
}

/** Opens a connection to a relay. */
export async function connect(url: string, events: TransportEvents): Promise<Transport> {
  await resetNative()
  if (isTauri) {
    await listenOnce()
    const { invoke } = await import('@tauri-apps/api/core')
    return native(await invoke<number>('collab_connect', { url }), events)
  }
  const ws = new WebSocket(url)
  await new Promise<void>((resolve, reject) => {
    ws.onopen = () => resolve()
    ws.onerror = () => reject(new Error(`Couldn’t reach ${new URL(url).host}.`))
  })
  let done = false
  ws.onmessage = (e) => typeof e.data === 'string' && events.message(e.data)
  ws.onclose = (e) => {
    if (done) return
    done = true
    events.closed(e.reason || 'closed')
  }
  return {
    send: (data) => ws.readyState === WebSocket.OPEN && ws.send(data),
    close: () => {
      done = true
      ws.close()
    },
  }
}
