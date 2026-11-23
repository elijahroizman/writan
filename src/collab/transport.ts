// How a shared document reaches the relay that orders everyone's changes.
//
// In the app the relay runs in Rust (src-tauri/src/collab): sharing starts it on this computer,
// and every connection, the sharer's own included, goes through Rust, since the page's content
// security policy doesn't allow sockets to other machines. For internet sharing, Rust also runs an
// ephemeral `iroh` P2P endpoint and registers a 6-digit code in the directory Worker; guests join
// via `writan-p2p://` URIs authenticated with SPAKE2 and bridged into the same relay.
//
// In a plain browser (development and the browser tests) the page talks WebSocket to a relay
// itself, the standalone one in src-tauri/examples/collab_relay.rs, which creates a session for
// whoever shares first.

import { isTauri } from '../lib/platform'

export interface Transport {
  send(data: string): void
  close(): void
}

export interface TransportEvents {
  message(data: string): void
  closed(reason: string): void
}

/** A session someone can join: either a WebSocket relay URL + token or an encrypted P2P invite. */
export interface Invite {
  /** `ws://host:port/v1/<session>` or `writan-p2p://<payload>?code=…` / `?token=…`. */
  url: string
  token: string
  /** True when `url` is a `writan-p2p://` invite. */
  p2p?: boolean
}

/** What sharing a document gives: the invite to pass on and the sharer's own way in. */
export interface Hosting {
  session: string
  invite: string
  /** Six digits guests can join with (internet + LAN when `internet` is true). None in a browser. */
  code: string | null
  local: Invite
  hostKey: string
  /** This computer's addresses on the network, for showing. */
  addresses: string[]
  /** Whether encrypted internet P2P sharing is active. */
  internet: boolean
  /** Unix epoch ms when the 6-digit internet code expires (`null` for LAN-only / browser). */
  expiresAt: number | null
  /** Whether internet joiners require 1-click host approval. */
  requireApproval: boolean
  /** If internet sharing couldn't start (e.g. offline), why it fell back to LAN. */
  internetError: string | null
}

export interface CodeUpdate {
  session: string
  code: string
  expiresAt: number
  rotatedByLockout: boolean
}

export interface JoinRequest {
  requestId: string
  session: string
  name: string
}

const P2P_SCHEME = 'writan-p2p://'

export function isP2pInvite(text: string): boolean {
  return text.trim().startsWith(P2P_SCHEME)
}

/** Builds a reconnect P2P invite URI using the authenticated session `token` instead of the 6-digit code. */
export function p2pReconnectUrl(originalUrl: string, token: string): string {
  const trimmed = originalUrl.trim()
  const base = trimmed.split('?')[0]
  return `${base}?token=${encodeURIComponent(token)}`
}

/** Reads an invite link (`ws://192.168.1.20:47811/v1/abc?token=…` or `writan-p2p://…`). */
export function parseInvite(text: string): Invite | null {
  const trimmed = text.trim()
  if (trimmed.startsWith(P2P_SCHEME)) {
    const rest = trimmed.slice(P2P_SCHEME.length)
    const [payload] = rest.split('?')
    if (!payload) return null
    return { url: trimmed, token: '', p2p: true }
  }
  try {
    const url = new URL(trimmed)
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
 * Finds who is sharing with `code`, racing the local network against the internet code directory
 * (src-tauri/src/collab/mod.rs). Returns a `ws://` or `writan-p2p://` invite.
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
export async function host(
  name: string,
  kind: string,
  opts?: { internet?: boolean; requireApproval?: boolean },
): Promise<Hosting> {
  await resetNative()
  if (isTauri) {
    const { invoke } = await import('@tauri-apps/api/core')
    const info = await invoke<{
      session: string
      token: string
      hostKey: string
      addresses: string[]
      inviteUrl: string
      code: string
      localUrl: string
      internet: boolean
      expiresAt: number | null
      requireApproval: boolean
      internetError: string | null
    }>('collab_share', {
      name,
      kind,
      internet: opts?.internet ?? true,
      requireApproval: opts?.requireApproval ?? true,
    })
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
  return {
    session,
    invite: `${url}?token=${token}`,
    code: null,
    local: { url, token },
    hostKey: randomId(24),
    addresses: [],
    internet: false,
    expiresAt: null,
    requireApproval: false,
    internetError: null,
  }
}

/** Ends hosting: the relay tells everyone and forgets the session. */
export async function unhost(session: string): Promise<void> {
  if (!isTauri) return
  const { invoke } = await import('@tauri-apps/api/core')
  await invoke('collab_unshare', { session })
}

/** Rotates an internet-shared session's 6-digit join code and resets its 10-minute window. */
export async function rotateCode(session: string): Promise<CodeUpdate> {
  if (!isTauri) throw new Error('Internet sharing is only available in the desktop app.')
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<CodeUpdate>('collab_rotate_code', { session })
}

/** Toggles whether new internet joiners need 1-click host approval. */
export async function setRequireApproval(session: string, requireApproval: boolean): Promise<void> {
  if (!isTauri) return
  const { invoke } = await import('@tauri-apps/api/core')
  await invoke('collab_set_require_approval', { session, requireApproval })
}

/** Approves or denies a pending internet join request. */
export async function approveJoin(requestId: string, allow: boolean): Promise<void> {
  if (!isTauri) return
  const { invoke } = await import('@tauri-apps/api/core')
  await invoke('collab_approve_join', { requestId, allow })
}

const routes = new Map<number, TransportEvents>()
let listening: Promise<void> | null = null

export interface HostEventHandlers {
  onJoinRequest?: (req: JoinRequest) => void
  onCodeChanged?: (update: CodeUpdate) => void
}

let hostHandlers: HostEventHandlers = {}

/** Registers listeners for host-side P2P events (join requests, code auto-rotations). */
export function setHostEventHandlers(handlers: HostEventHandlers) {
  hostHandlers = handlers
  if (isTauri) void listenOnce()
}

function listenOnce() {
  listening ??= (async () => {
    const { listen } = await import('@tauri-apps/api/event')
    await listen<{ conn: number; data: string }>('collab-message', (e) => routes.get(e.payload.conn)?.message(e.payload.data))
    await listen<{ conn: number; reason: string }>('collab-closed', (e) => {
      const route = routes.get(e.payload.conn)
      routes.delete(e.payload.conn)
      route?.closed(e.payload.reason)
    })
    await listen<JoinRequest>('collab-join-request', (e) => hostHandlers.onJoinRequest?.(e.payload))
    await listen<CodeUpdate>('collab-code-changed', (e) => hostHandlers.onCodeChanged?.(e.payload))
  })()
  return listening
}

function nativeTransport(conn: number, events: TransportEvents): Transport {
  routes.set(conn, events)
  const fail = (err: unknown) => {
    if (!routes.delete(conn)) return
    void import('@tauri-apps/api/core').then(({ invoke }) => invoke('collab_close', { conn }).catch(() => {}))
    events.closed(String((err as Error)?.message ?? err))
  }
  return {
    send: (data) =>
      void import('@tauri-apps/api/core')
        .then(({ invoke }) => invoke('collab_send', { conn, data }))
        .catch(fail),
    close: () => {
      routes.delete(conn)
      void import('@tauri-apps/api/core').then(({ invoke }) => invoke('collab_close', { conn }).catch(() => {}))
    },
  }
}

/** Opens an encrypted `iroh` P2P connection, runs SPAKE2 or token reconnect, and returns the transport + authenticated token. */
export async function connectP2p(
  url: string,
  name: string,
  events: TransportEvents,
): Promise<{ transport: Transport; session: string; token: string }> {
  await resetNative()
  if (!isTauri) throw new Error('Internet P2P sharing is only available in the desktop app.')
  await listenOnce()
  const { invoke } = await import('@tauri-apps/api/core')
  const info = await invoke<{ conn: number; session: string; token: string }>('collab_connect_p2p', { url, name })
  return { transport: nativeTransport(info.conn, events), session: info.session, token: info.token }
}

/** Opens a connection to a relay (`ws://` or `wss://`). */
export async function connect(url: string, events: TransportEvents): Promise<Transport> {
  await resetNative()
  if (isTauri) {
    await listenOnce()
    const { invoke } = await import('@tauri-apps/api/core')
    const conn = await invoke<number>('collab_connect', { url })
    return nativeTransport(conn, events)
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
