// A note in the top right corner, under the title bar's share button, when someone joins or leaves
// a shared document, a person or an AI agent. Clicks go through it and it takes no focus, so it
// never gets in the way of typing; it goes away by itself (session.ts `arrive`).

import type { CSSProperties } from 'react'
import { stripExtension } from '../lib/kinds'
import { useWorkspace } from '../state/workspace'
import { PeerAvatar } from './cursors'
import { useArrivals } from './session'

export function Arrivals() {
  const list = useArrivals()
  const tabs = useWorkspace((s) => s.tabs)
  if (!list.length) return null
  return (
    <div className="arrivals" role="status" aria-live="polite">
      {list.map((a) => {
        const title = tabs.find((t) => t.id === a.tabId)?.title
        return (
          <div key={a.id} className={`arrival ${a.joined ? 'joined' : 'left'}`} style={{ '--peer': a.peer.color } as CSSProperties}>
            <PeerAvatar peer={a.peer} />
            <span className="arrival-text">
              <span className="arrival-who">
                <strong>{a.peer.name || 'Someone'}</strong> {a.joined ? 'joined' : 'left'}
              </span>
              {title && <span className="arrival-doc">{stripExtension(title)}</span>}
            </span>
          </div>
        )
      })}
    </div>
  )
}
