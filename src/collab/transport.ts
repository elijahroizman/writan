// How a shared document reaches the relay that orders everyone's changes.
//
// In the app the relay runs in Rust (src-tauri/src/collab): sharing starts it on this computer,
// and every connection, the sharer's own included, goes through Rust, since the page's content
// security policy doesn't allow sockets to other machines. In a plain browser (development and
// the browser tests) the page talks WebSocket to a relay itself, the standalone one in
// src-tauri/examples/collab_relay.rs, which creates a session for whoever shares first. A relay
// in the cloud would be reached the same way.

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
  /** `ws://host:port/v1/<session>`, without the token. */
  url: string
  token: string
}

/** What sharing a document gives: the invite to pass on and the sharer's own way in. */
export interface Hosting {
  session: string
  invite: string
  /** Six digits guests on the network can join with instead of the link. None in a browser. */
  code: string | null
  local: Invite
  hostKey: string
  /** This computer's addresses on the network, for showing. */
  addresses: string[]
}

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
 * Finds the computer on the network sharing with `code` and returns its invite link. Rust
 * broadcasts the code and asks the computers nearby (src-tauri/src/collab/codes.rs).
 */
export async function findInvite(code: string): Promise<string> {
  if (!isTauri) throw new Error('In the browser, join with the link instead of a code.')
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<string>('collab_find', { code })
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

/** Starts hosting a session for a document. */
export async function host(name: string, kind: string): Promise<Hosting> {
  await resetNative()
  if (isTauri) {
    const { invoke } = await import('@tauri-apps/api/core')
    const info = await invoke<{ session: string; token: string; hostKey: string; addresses: string[]; inviteUrl: string; code: string; localUrl: string }>('collab_share', { name, kind })
    return { session: info.session, invite: info.inviteUrl, code: info.code, local: { url: info.localUrl, token: info.token }, hostKey: info.hostKey, addresses: info.addresses }
  }
  const session = randomId(12)
  const token = randomId(24)
  const url = `${browserRelay()}/v1/${session}`
  return { session, invite: `${url}?token=${token}`, code: null, local: { url, token }, hostKey: randomId(24), addresses: [] }
}

/** Ends hosting: the relay tells everyone and forgets the session. */
export async function unhost(session: string): Promise<void> {
  if (!isTauri) return
  const { invoke } = await import('@tauri-apps/api/core')
  await invoke('collab_unshare', { session })
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
  })()
  return listening
}

/** Opens a connection to a relay. */
export async function connect(url: string, events: TransportEvents): Promise<Transport> {
  await resetNative()
  if (isTauri) {
    await listenOnce()
    const { invoke } = await import('@tauri-apps/api/core')
    const conn = await invoke<number>('collab_connect', { url })
    routes.set(conn, events)
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
