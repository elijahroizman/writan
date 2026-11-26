// The top right corner, under the title bar's share button: who's asking to join a document
// shared from here, a person or someone's AI agent, to let in or not; and a note when someone
// joins or leaves. A request stays until it's answered or they stop waiting. A note's clicks go
// through it and it takes no focus, so it never gets in the way of typing; it goes away by itself
// (session.ts `arrive`).

import type { CSSProperties } from 'react'
import { AGENTS, type AgentId } from './agents/identity'
import { Button } from '../components/ui'
import { stripExtension } from '../lib/kinds'
import { useWorkspace } from '../state/workspace'
import { PeerAvatar } from './cursors'
import { colorFor, letIn, useArrivals, useCollab } from './session'

/** Everyone waiting to be let in, oldest first, with the tab they ask for. */
function useRequests() {
  const shared = useCollab()
  return Object.entries(shared).flatMap(([tabId, s]) => (s.role === 'host' && (s.status === 'live' || s.status === 'connecting') ? (s.waiting ?? []).map((r) => ({ tabId, ...r })) : []))
}

export function Arrivals() {
  const list = useArrivals()
  const requests = useRequests()
  if (!list.length && !requests.length) return null
  const tabs = useWorkspace.getState().tabs
  const titleOf = (tabId: string) => {
    const title = tabs.find((t) => t.id === tabId)?.title
    return title ? stripExtension(title) : null
  }
  return (
    <div className="arrivals">
      {requests.map((r) => {
        const color = r.agent && r.agent in AGENTS ? AGENTS[r.agent as AgentId].color : colorFor(r.name)
        const title = titleOf(r.tabId)
        return (
          <div key={r.request} className="arrival join-request" role="alertdialog" aria-label={`${r.name} wants to join`} style={{ '--peer': color } as CSSProperties}>
            <PeerAvatar peer={{ client: r.request, name: r.name, color, ...(r.agent ? { agent: r.agent } : {}) }} />
            <span className="arrival-text">
              <span className="arrival-who">
                <strong>{r.name}</strong> wants to join
              </span>
              {title && <span className="arrival-doc">{title}</span>}
            </span>
            <span className="join-request-actions">
              <Button onClick={() => void letIn(r.tabId, r.request, false)}>Deny</Button>
              <Button variant="primary" onClick={() => void letIn(r.tabId, r.request, true)}>
                Let in
              </Button>
            </span>
          </div>
        )
      })}
      <div className="arrivals-notes" role="status" aria-live="polite">
        {list.map((a) => {
          const title = titleOf(a.tabId)
          return (
            <div key={a.id} className={`arrival ${a.joined ? 'joined' : 'left'}`} style={{ '--peer': a.peer.color } as CSSProperties}>
              <PeerAvatar peer={a.peer} />
              <span className="arrival-text">
                <span className="arrival-who">
                  <strong>{a.peer.name || 'Someone'}</strong> {a.joined ? 'joined' : 'left'}
                </span>
                {title && <span className="arrival-doc">{title}</span>}
              </span>
            </div>
          )
        })}
      </div>
    </div>
  )
}
