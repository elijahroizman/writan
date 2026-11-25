// A shared document's chat, for the people in it: what the AI agents in the document said to
// each other (agents/messages.ts). It's there to read when someone wants to, from a button
// beside Share; nothing pops up when a message comes, and a dot on the button says there's one
// the person hasn't seen.

import { MessagesSquare } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { PeerAvatar } from '../collab/cursors'
import { AGENTS, type AgentId } from '../collab/agents/identity'
import { seenChat, useChat, useCollab, type Said } from '../collab/session'
import { useWorkspace } from '../state/workspace'

const time = (at: number) => new Date(at).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })

export function ChatButton() {
  const activeId = useWorkspace((s) => s.activeId)
  const chat = useChat((s) => s[activeId])
  const on = useCollab((s) => s[activeId]?.status === 'live')
  const [open, setOpen] = useState<string | null>(null)
  const button = useRef<HTMLButtonElement>(null)
  // Only once there's something to read, and only while the document is shared.
  if (!chat?.said.length || !on) return null
  const unseen = chat.said.length - chat.seen
  const label = unseen ? `Agents’ chat, ${unseen} new` : 'Agents’ chat'
  return (
    <>
      <button
        ref={button}
        type="button"
        className="share-btn chat-btn"
        aria-haspopup="dialog"
        aria-expanded={open === activeId}
        aria-label={label}
        title={label}
        onClick={() => setOpen(open === activeId ? null : activeId)}
      >
        <MessagesSquare size={14} />
        {unseen > 0 && <span className="share-btn-dot" />}
      </button>
      {open === activeId && <ChatPanel tabId={activeId} anchor={button} onClose={() => setOpen(null)} />}
    </>
  )
}

function ChatPanel({ tabId, anchor, onClose }: { tabId: string; anchor: React.RefObject<HTMLButtonElement | null>; onClose: () => void }) {
  const said = useChat((s) => s[tabId]?.said ?? [])
  const peers = useCollab((s) => s[tabId]?.peers ?? [])
  const panel = useRef<HTMLDivElement>(null)
  const list = useRef<HTMLOListElement>(null)
  const [pos, setPos] = useState({ x: 0, y: 0 })

  // While it's open, what comes is seen, and the newest stays in view.
  useEffect(() => {
    seenChat(tabId)
    const el = list.current
    if (el) el.scrollTop = el.scrollHeight
  }, [tabId, said.length])

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
  }, [anchor])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose()
    const onDown = (e: PointerEvent) => {
      const target = e.target as Node
      if (!panel.current?.contains(target) && !anchor.current?.contains(target)) onClose()
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('pointerdown', onDown, true)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('pointerdown', onDown, true)
    }
  }, [anchor, onClose])

  /** The sender as the avatar draws them: as they are here now, or as the message names them. */
  const sender = (m: Said) => peers.find((p) => p.client === m.client) ?? { client: m.client, name: m.name, agent: m.agent, color: AGENTS[m.agent as AgentId]?.color ?? '#6E56CF' }

  return createPortal(
    <div ref={panel} className="share-panel chat-panel" role="dialog" aria-labelledby="chat-title" style={{ left: pos.x, top: pos.y, maxHeight: `calc(100vh - ${pos.y + 8}px)` }}>
      <header className="share-head">
        <h2 id="chat-title">Agents’ chat</h2>
        <p>What the AI agents in this document say to each other. Everyone in it sees the same chat.</p>
      </header>
      <ol ref={list} className="chat-list" aria-label="Messages">
        {said.map((m, i) => (
          <li key={i}>
            <PeerAvatar peer={sender(m)} />
            <div>
              <div className="chat-meta">
                <span className="chat-name">{m.name}</span>
                {!m.earlier && <time>{time(m.at)}</time>}
              </div>
              <p className="chat-text">{m.text}</p>
            </div>
          </li>
        ))}
      </ol>
    </div>,
    document.body,
  )
}
