// Sharing a document over the internet or local network: the title bar's button and panel,
// and the dialog for joining what someone else shared. The work is in collab/session.ts.

import { Check, Copy, Link2, LogIn, RefreshCw, ShieldCheck, Users } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { create } from 'zustand'
import {
  approveJoinRequest,
  colorFor,
  displayName,
  join,
  rotateShareCode,
  setDisplayName,
  setShareRequireApproval,
  share,
  SHAREABLE,
  stop,
  useCollab,
  type Peer,
} from '../collab/session'
import { parseCode, parseInvite } from '../collab/transport'
import { PeerAvatar } from '../collab/cursors'
import { askForNetwork, NETWORK_ASK, NETWORK_DENIED, openNetworkSettings, whileNetworkAllowed } from '../collab/network'
import { isMac, isTauri } from '../lib/platform'
import { useWorkspace } from '../state/workspace'
import { Button, Modal, Spinner, TextField } from './ui'

/** Which tab's sharing panel is open, and whether the join dialog is. */
export const useSharePanel = create<{ tabId: string | null; joining: boolean }>(() => ({ tabId: null, joining: false }))

export const showSharePanel = (tabId = useWorkspace.getState().activeId) => useSharePanel.setState({ tabId })
export const showJoinDialog = () => useSharePanel.setState({ joining: true, tabId: null })

/** Whether the tab in front can be shared, or is shared. */
export function canShare(tabId: string) {
  const tab = useWorkspace.getState().tabs.find((t) => t.id === tabId)
  return !!tab && (SHAREABLE.has(tab.kind) || !!useCollab.getState()[tabId])
}

const Avatar = PeerAvatar

function formatCountdown(expiresAt: number | null | undefined, now: number): string | null {
  if (!expiresAt) return null
  const secs = Math.max(0, Math.ceil((expiresAt - now) / 1000))
  if (secs === 0) return 'Code expired'
  const m = Math.floor(secs / 60)
  const s = secs % 60
  return `Code expires in ${m}:${s.toString().padStart(2, '0')}`
}

export function ShareButton() {
  const activeId = useWorkspace((s) => s.activeId)
  const tab = useWorkspace((s) => s.tabs.find((t) => t.id === s.activeId))
  const state = useCollab((s) => s[activeId])
  const openFor = useSharePanel((s) => s.tabId)
  const button = useRef<HTMLButtonElement>(null)
  if (!tab || (!SHAREABLE.has(tab.kind) && !state)) return null
  const live = state?.status === 'live' || state?.status === 'connecting'
  const peers = live ? state.peers : []
  const pendingCount = live ? (state.pendingJoins?.length ?? 0) : 0
  const label = live
    ? state.role === 'host'
      ? state.internet
        ? 'Shared'
        : 'Shared on your network'
      : 'Editing a shared document'
    : isTauri
      ? 'Share'
      : 'Share on your network'
  return (
    <>
      <button
        ref={button}
        type="button"
        className={`share-btn ${live ? 'live' : ''}`}
        aria-haspopup="dialog"
        aria-expanded={openFor === activeId}
        aria-label={label}
        title={label}
        onClick={() => useSharePanel.setState({ tabId: openFor === activeId ? null : activeId })}
      >
        {peers.length ? (
          <span className="share-stack">
            {peers.slice(0, 3).map((p) => (
              <Avatar key={p.client} peer={p} />
            ))}
          </span>
        ) : (
          <Users size={14} />
        )}
        {(pendingCount > 0 || (live && !peers.length)) && <span className="share-btn-dot" />}
      </button>
      {openFor === activeId && <SharePanel tabId={activeId} anchor={button} />}
    </>
  )
}

function useName() {
  const [name, setName] = useState(displayName)
  return [name, (n: string) => setName(n)] as const
}

/** What to say while macOS hasn't let Writan reach the network: waiting for an answer to its
 * question, or turned down. */
type Network = 'waiting' | 'denied' | null

function NetworkNote({ state }: { state: Network }) {
  if (!state) return null
  return (
    <div className="share-network" role="status">
      <p>{state === 'waiting' ? NETWORK_ASK : NETWORK_DENIED}</p>
      {isMac && <Button onClick={openNetworkSettings}>Open Settings</Button>}
    </div>
  )
}

function SharePanel({ tabId, anchor }: { tabId: string; anchor: React.RefObject<HTMLButtonElement | null> }) {
  const state = useCollab((s) => s[tabId])
  const tab = useWorkspace((s) => s.tabs.find((t) => t.id === tabId))
  const panel = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ x: 0, y: 0 })
  const [name, setName] = useName()
  const [requireApproval, setRequireApproval] = useState(true)
  const [busy, setBusy] = useState(false)
  const [rotating, setRotating] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [copied, setCopied] = useState<'code' | 'link' | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const close = () => useSharePanel.setState({ tabId: null })
  // Others edit the file on disk with you, so it needs to be one, and what they start from is
  // what's saved.
  const unsaved = !tab?.path || !!tab?.dirty

  useEffect(() => {
    if (!state?.expiresAt) return
    setNow(Date.now())
    const id = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(id)
  }, [state?.expiresAt])

  useLayoutEffect(() => {
    const place = () => {
      const rect = anchor.current?.getBoundingClientRect()
      const el = panel.current
      if (!rect || !el) return
      setPos({ x: Math.max(8, Math.min(rect.right - el.offsetWidth, window.innerWidth - el.offsetWidth - 8)), y: rect.bottom + 8 })
    }
    place()
    window.addEventListener('resize', place)
    return () => window.removeEventListener('resize', place)
  }, [anchor, state?.status, state?.pendingJoins?.length])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && close()
    const onDown = (e: PointerEvent) => {
      const target = e.target as Node
      if (!panel.current?.contains(target) && !anchor.current?.contains(target)) close()
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('pointerdown', onDown, true)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('pointerdown', onDown, true)
    }
  }, [anchor])

  const start = async () => {
    const who = name.trim()
    if (!who) return setError('Type the name others will see.')
    setDisplayName(who)
    setBusy(true)
    setError(null)
    try {
      // On a Mac, macOS asks now whether Writan may reach the network, if it hasn't asked before.
      await askForNetwork()
      const ws = useWorkspace.getState()
      const nowTab = ws.tabs.find((t) => t.id === tabId)
      if (nowTab && (!nowTab.path || nowTab.dirty)) {
        const saved = (await ws.save(tabId)) && !useWorkspace.getState().tabs.find((t) => t.id === tabId)?.dirty
        if (!saved) return setError('Save the document to share it. Everyone edits the saved file with you.')
      }
      await share(tabId, who, { internet: true, requireApproval })
    } catch (err) {
      setError(String((err as Error)?.message ?? err))
    } finally {
      setBusy(false)
    }
  }

  const rotate = async () => {
    setRotating(true)
    setError(null)
    try {
      await rotateShareCode(tabId)
    } catch (err) {
      setError(String((err as Error)?.message ?? err))
    } finally {
      setRotating(false)
    }
  }

  const toggleApproval = async (checked: boolean) => {
    setRequireApproval(checked)
    if (state?.role === 'host' && (state.status === 'live' || state.status === 'connecting')) {
      try {
        await setShareRequireApproval(tabId, checked)
      } catch (err) {
        setError(String((err as Error)?.message ?? err))
      }
    }
  }

  const copy = async (what: 'code' | 'link') => {
    const text = what === 'code' ? state?.code : state?.invite
    if (!text) return
    await navigator.clipboard.writeText(text).catch(() => {})
    setCopied(what)
    window.setTimeout(() => setCopied(null), 1500)
  }

  const live = state && (state.status === 'live' || state.status === 'connecting')
  const you: Peer = state?.you ?? { client: 'you', name: displayName() || 'You', color: colorFor(displayName() || 'you') }
  const countdown = formatCountdown(state?.expiresAt, now)

  let body: React.ReactNode
  if (!state || !live) {
    body = (
      <>
        <header className="share-head">
          <h2 id="share-title">
            {state?.status === 'ended' || state?.status === 'disconnected'
              ? 'Not shared any more'
              : isTauri
                ? 'Share document'
                : 'Share on your network'}
          </h2>
          <p>
            {state?.message ??
              (tab?.path
                ? isTauri
                  ? `Share “${tab.title}” with a 6-digit code over an encrypted peer-to-peer connection. Your copy stays the file, and you save it as usual.`
                  : `People on the same network can open “${tab.title}” and edit it with you while it’s open here. Your copy stays the file, and you save it as usual.`
                : isTauri
                  ? `Share “${tab?.title ?? 'this document'}” with a 6-digit code over an encrypted peer-to-peer connection. Save it first: your copy stays the file.`
                  : `People on the same network can edit “${tab?.title ?? 'this document'}” with you while it’s open here. Save it first: your copy stays the file, and you save it as usual.`)}
          </p>
        </header>
        {(!state || state.role === 'host') && (
          <div className="share-body">
            <TextField label="Your name" value={name} placeholder="How others see you" autoFocus onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && void start()} />
            {isTauri && (
              <label className="share-option">
                <input type="checkbox" checked={requireApproval} onChange={(e) => setRequireApproval(e.target.checked)} />
                <span>Require my approval when someone joins</span>
              </label>
            )}
            {error && <p className="share-error">{error}</p>}
            {!isTauri && <p className="share-note">In the browser, sharing goes through the relay at the address in workspace.collabRelay.</p>}
          </div>
        )}
        <footer className="share-foot">
          <button type="button" className="share-link" onClick={() => useSharePanel.setState({ tabId: null, joining: true })}>
            <LogIn size={13} /> Join a shared document…
          </button>
          {state ? (
            <Button onClick={() => stop(tabId)}>Close</Button>
          ) : (
            <Button variant="primary" disabled={busy} onClick={() => void start()}>
              {busy ? <Spinner size={14} /> : null} {unsaved ? 'Save and Share' : 'Share'}
            </Button>
          )}
        </footer>
      </>
    )
  } else {
    const isHost = state.role === 'host'
    const pendingJoins = state.pendingJoins ?? []
    body = (
      <>
        <header className="share-head">
          <h2 id="share-title">
            {isHost ? (state.internet ? 'Shared' : 'Shared on your network') : 'Editing a shared document'}
          </h2>
          <p>
            {state.status === 'connecting'
              ? state.message || 'Connecting…'
              : isHost
                ? state.internet
                  ? 'Give this 6-digit code to anyone you want to edit with. Connections are end-to-end encrypted.'
                  : state.code
                    ? 'Give this code to people on your network. Anyone with it can edit while this tab is open.'
                    : 'Send this link to people on your network. Anyone with it can edit while this tab is open.'
                : 'Your changes go to everyone in the document. The person sharing keeps the file.'}
          </p>
        </header>
        <div className="share-body">
          {isHost && state.internetError && (
            <p className="share-note">Internet sharing unavailable ({state.internetError}). Sharing on your local network only.</p>
          )}
          {isHost && state.code && (
            <div className="share-code">
              <span className="share-code-digits" aria-label={`Code ${state.code.split('').join(' ')}`}>
                {state.code.slice(0, 3)}
                <span className="share-code-gap" />
                {state.code.slice(3)}
              </span>
              <Button onClick={() => void copy('code')}>
                {copied === 'code' ? <Check size={14} /> : <Copy size={14} />} {copied === 'code' ? 'Copied' : 'Copy'}
              </Button>
            </div>
          )}
          {isHost && state.internet && (
            <div className="share-code-meta">
              <span>{countdown ?? 'Encrypted P2P'}</span>
              <button type="button" className="share-link" disabled={rotating} onClick={() => void rotate()}>
                <RefreshCw size={12} /> {rotating ? 'Rotating…' : 'New code'}
              </button>
            </div>
          )}
          {isHost && state.internet && (
            <label className="share-option">
              <input
                type="checkbox"
                checked={state.requireApproval ?? true}
                onChange={(e) => void toggleApproval(e.target.checked)}
              />
              <ShieldCheck size={13} />
              <span>Require approval to join</span>
            </label>
          )}
          {isHost && state.message && <p className="share-note">{state.message}</p>}
          {error && <p className="share-error">{error}</p>}
          {isHost && pendingJoins.length > 0 && (
            <div className="share-requests" role="status" aria-live="polite">
              {pendingJoins.map((req) => (
                <div key={req.requestId} className="share-request">
                  <span>
                    <strong>{req.name}</strong> wants to join
                  </span>
                  <div className="share-request-actions">
                    <Button onClick={() => void approveJoinRequest(tabId, req.requestId, false)}>Deny</Button>
                    <Button variant="primary" onClick={() => void approveJoinRequest(tabId, req.requestId, true)}>
                      Allow
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          )}
          {isHost && !state.internet && state.code && state.invite && (
            // LAN-only fallback when broadcasts are blocked on the local network.
            <button type="button" className="share-link share-copy-link" onClick={() => void copy('link')}>
              {copied === 'link' ? <Check size={12} /> : <Link2 size={12} />} {copied === 'link' ? 'Link copied' : 'Copy the link instead'}
            </button>
          )}
          {isHost && !state.code && state.invite && (
            <div className="share-invite">
              <Link2 size={14} className="share-invite-icon" />
              <input readOnly value={state.invite} aria-label="Invite link" onFocus={(e) => e.currentTarget.select()} />
              <Button onClick={() => void copy('link')}>
                {copied === 'link' ? <Check size={14} /> : <Copy size={14} />} {copied === 'link' ? 'Copied' : 'Copy'}
              </Button>
            </div>
          )}
          <ul className="share-people" aria-label="People in this document">
            <li>
              <Avatar peer={you} you />
              <span>{you.name}</span>
              <span className="share-role">{isHost ? 'You · sharing' : 'You'}</span>
            </li>
            {state.peers.map((p) => (
              <li key={p.client}>
                <Avatar peer={p} />
                <span>{p.name || 'Someone'}</span>
                {(p.agent || p.host) && <span className="share-role">{p.agent ? 'AI agent' : 'Sharing'}</span>}
              </li>
            ))}
            {!state.peers.length && <li className="share-empty">No one else is here yet.</li>}
          </ul>
        </div>
        <footer className="share-foot">
          <Button variant={isHost ? 'danger' : 'default'} onClick={() => stop(tabId)}>
            {isHost ? 'Stop sharing' : 'Leave'}
          </Button>
        </footer>
      </>
    )
  }

  return createPortal(
    <div ref={panel} className="share-panel" role="dialog" aria-labelledby="share-title" style={{ left: pos.x, top: pos.y }}>
      {body}
    </div>,
    document.body,
  )
}

/** Asks for the 6-digit code (or a browser/LAN link) and joins. */
export function JoinDialog() {
  const open = useSharePanel((s) => s.joining)
  const [name, setName] = useName()
  const [code, setCode] = useState('')
  const usable = !!parseCode(code) || !!parseInvite(code)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [network, setNetwork] = useState<Network>(null)
  /** Goes up when the dialog closes, so an attempt from before doesn't open anything. */
  const attempts = useRef(0)
  if (!open) return null
  const close = () => {
    if (busy && network !== 'waiting') return
    attempts.current++
    setBusy(false)
    setError(null)
    setNetwork(null)
    useSharePanel.setState({ joining: false })
  }
  const go = async () => {
    const who = name.trim()
    if (!who) return setError('Type the name others will see.')
    setDisplayName(who)
    setBusy(true)
    setError(null)
    // Closing the dialog (or trying again) makes this attempt stale: it opens nothing then,
    // and says nothing.
    const attempt = ++attempts.current
    const cancelled = () => attempts.current !== attempt || !useSharePanel.getState().joining
    try {
      await askForNetwork()
      const joined = await whileNetworkAllowed(
        () => join(code, who, cancelled).then(() => true),
        (on) => !cancelled() && setNetwork(on ? 'waiting' : null),
        cancelled,
      )
      if (cancelled()) return
      if (!joined) return setNetwork('denied')
      setNetwork(null)
      setCode('')
      useSharePanel.setState({ joining: false })
    } catch (err) {
      if (cancelled()) return
      setNetwork(null)
      setError(String((err as Error)?.message ?? err))
    } finally {
      if (attempts.current === attempt) setBusy(false)
    }
  }
  return (
    <Modal placement="top" dim="light" layer="picker" closeOnEscape onClose={close}>
      <form
        className="dialog share-join"
        role="dialog"
        aria-modal="true"
        aria-labelledby="join-title"
        onSubmit={(e) => {
          e.preventDefault()
          void go()
        }}
      >
        <div className="dialog-text">
          <h2 id="join-title">Join a shared document</h2>
          <p>
            {isTauri
              ? 'Type the 6-digit code from the person sharing. Works over the internet or your local network.'
              : 'Paste the link the person sharing copied from their Share panel.'}
          </p>
        </div>
        <div className="share-body">
          <TextField
            label={isTauri ? 'Code' : 'Code or link'}
            value={code}
            placeholder={isTauri ? '123 456' : '123 456 or ws://…'}
            autoFocus
            autoComplete="off"
            spellCheck={false}
            inputMode="numeric"
            className="share-code-field"
            onChange={(e) => setCode(e.target.value)}
          />
          <TextField label="Your name" value={name} placeholder="How others see you" onChange={(e) => setName(e.target.value)} />
          <NetworkNote state={network} />
          {error && <p className="share-error">{error}</p>}
        </div>
        <div className="dialog-actions">
          <Button onClick={close} disabled={busy}>
            Cancel
          </Button>
          <button type="submit" className="btn primary" disabled={busy || !usable}>
            {busy ? <Spinner size={14} /> : null} {busy && network !== 'waiting' && parseCode(code) ? 'Connecting…' : 'Join'}
          </button>
        </div>
      </form>
    </Modal>
  )
}
