// Sharing a document on the network: the title bar's button and panel, and the dialog for
// joining what someone else shared. The work is in collab/session.ts.

import { Check, Copy, Link2, LogIn, Users } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { create } from 'zustand'
import { colorFor, displayName, join, setDisplayName, share, SHAREABLE, stop, useCollab, type Peer } from '../collab/session'
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

function Avatar({ peer, you }: { peer: Peer; you?: boolean }) {
  const initial = (peer.name || '?').trim().charAt(0).toUpperCase() || '?'
  return (
    <span className="share-avatar" style={{ background: peer.color }} title={you ? `${peer.name} (you)` : peer.name} aria-hidden>
      {initial}
    </span>
  )
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
  const label = live ? (state.role === 'host' ? 'Shared on your network' : 'Editing a shared document') : 'Share on your network'
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
        {live && !peers.length && <span className="share-btn-dot" />}
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
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const close = () => useSharePanel.setState({ tabId: null })
  // Others edit the file on disk with you, so it needs to be one, and what they start from is
  // what's saved.
  const unsaved = !tab?.path || !!tab?.dirty

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
  }, [anchor, state?.status])

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
      const now = ws.tabs.find((t) => t.id === tabId)
      if (now && (!now.path || now.dirty)) {
        const saved = (await ws.save(tabId)) && !useWorkspace.getState().tabs.find((t) => t.id === tabId)?.dirty
        if (!saved) return setError('Save the document to share it. Everyone edits the saved file with you.')
      }
      await share(tabId, who)
    } catch (err) {
      setError(String((err as Error)?.message ?? err))
    } finally {
      setBusy(false)
    }
  }

  const copy = async () => {
    if (!state?.invite) return
    await navigator.clipboard.writeText(state.invite).catch(() => {})
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1500)
  }

  const live = state && (state.status === 'live' || state.status === 'connecting')
  const you: Peer = state?.you ?? { client: 'you', name: displayName() || 'You', color: colorFor(displayName() || 'you') }

  let body: React.ReactNode
  if (!state || !live) {
    body = (
      <>
        <header className="share-head">
          <h2 id="share-title">{state?.status === 'ended' || state?.status === 'disconnected' ? 'Not shared any more' : 'Share on your network'}</h2>
          <p>
            {state?.message ??
              (tab?.path
                ? `People on the same network can open “${tab.title}” and edit it with you while it’s open here. Your copy stays the file, and you save it as usual.`
                : `People on the same network can edit “${tab?.title ?? 'this document'}” with you while it’s open here. Save it first: your copy stays the file, and you save it as usual.`)}
          </p>
        </header>
        {(!state || state.role === 'host') && (
          <div className="share-body">
            <TextField label="Your name" value={name} placeholder="How others see you" autoFocus onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && void start()} />
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
    const host = state.role === 'host'
    body = (
      <>
        <header className="share-head">
          <h2 id="share-title">{host ? 'Shared on your network' : 'Editing a shared document'}</h2>
          <p>
            {state.status === 'connecting'
              ? 'Connecting…'
              : host
                ? 'Send this link to people on your network. Anyone with it can edit while this tab is open.'
                : 'Your changes go to everyone in the document. The person sharing keeps the file.'}
          </p>
        </header>
        <div className="share-body">
          {host && state.invite && (
            <div className="share-invite">
              <Link2 size={14} className="share-invite-icon" />
              <input readOnly value={state.invite} aria-label="Invite link" onFocus={(e) => e.currentTarget.select()} />
              <Button onClick={() => void copy()}>
                {copied ? <Check size={14} /> : <Copy size={14} />} {copied ? 'Copied' : 'Copy'}
              </Button>
            </div>
          )}
          <ul className="share-people" aria-label="People in this document">
            <li>
              <Avatar peer={you} you />
              <span>{you.name}</span>
              <span className="share-role">{host ? 'You · sharing' : 'You'}</span>
            </li>
            {state.peers.map((p) => (
              <li key={p.client}>
                <Avatar peer={p} />
                <span>{p.name || 'Someone'}</span>
              </li>
            ))}
            {!state.peers.length && <li className="share-empty">No one else is here yet.</li>}
          </ul>
        </div>
        <footer className="share-foot">
          <Button variant={host ? 'danger' : 'default'} onClick={() => stop(tabId)}>
            {host ? 'Stop sharing' : 'Leave'}
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

/** Asks for an invite link and joins. */
export function JoinDialog() {
  const open = useSharePanel((s) => s.joining)
  const [name, setName] = useName()
  const [link, setLink] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [network, setNetwork] = useState<Network>(null)
  if (!open) return null
  const close = () => {
    if (busy && network !== 'waiting') return
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
    try {
      await askForNetwork()
      const cancelled = () => !useSharePanel.getState().joining
      const joined = await whileNetworkAllowed(
        () => join(link, who, cancelled).then(() => true),
        (on) => setNetwork(on ? 'waiting' : null),
        cancelled,
      )
      if (!joined) {
        if (useSharePanel.getState().joining) setNetwork('denied')
        return
      }
      setNetwork(null)
      setLink('')
      useSharePanel.setState({ joining: false })
    } catch (err) {
      setNetwork(null)
      setError(String((err as Error)?.message ?? err))
    } finally {
      setBusy(false)
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
          <p>Paste the link the person sharing sent you. You both need to be on the same network.</p>
        </div>
        <div className="share-body">
          <TextField label="Link" value={link} placeholder="ws://…" autoFocus onChange={(e) => setLink(e.target.value)} />
          <TextField label="Your name" value={name} placeholder="How others see you" onChange={(e) => setName(e.target.value)} />
          <NetworkNote state={network} />
          {error && <p className="share-error">{error}</p>}
        </div>
        <div className="dialog-actions">
          <Button onClick={close} disabled={busy}>
            Cancel
          </Button>
          <button type="submit" className="btn primary" disabled={busy || !link.trim()}>
            {busy ? <Spinner size={14} /> : null} Join
          </button>
        </div>
      </form>
    </Modal>
  )
}
