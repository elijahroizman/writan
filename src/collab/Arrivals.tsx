// A small note in the top left corner when someone joins or leaves a shared document, a person
// or an AI agent. It goes away by itself (session.ts `arrive`).

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
          <div key={a.id} className={`arrival ${a.joined ? 'joined' : 'left'}`}>
            <PeerAvatar peer={a.peer} />
            <span>
              <strong>{a.peer.name || 'Someone'}</strong> {a.joined ? 'joined' : 'left'}
              {title ? ` “${stripExtension(title)}”` : ''}
            </span>
          </div>
        )
      })}
    </div>
  )
}
