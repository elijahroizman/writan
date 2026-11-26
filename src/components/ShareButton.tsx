import { Users } from 'lucide-react'
import { lazy, Suspense, useRef } from 'react'
import { PeerAvatar } from '../collab/cursors'
import { SHAREABLE, useCollab } from '../collab/session'
import { isTauri } from '../lib/platform'
import { useSharePanel } from '../state/modals'
import { useWorkspace } from '../state/workspace'

const SharePanel = lazy(() => import('./Share').then((m) => ({ default: m.SharePanel })))

/** Whether the tab in front can be shared, or is shared. */
export function canShare(tabId: string) {
  const tab = useWorkspace.getState().tabs.find((t) => t.id === tabId)
  return !!tab && (SHAREABLE.has(tab.kind) || !!useCollab.getState()[tabId])
}

export function ShareButton() {
  const activeId = useWorkspace((s) => s.activeId)
  const kind = useWorkspace((s) => s.tabs.find((t) => t.id === s.activeId)?.kind)
  const state = useCollab((s) => s[activeId])
  const openFor = useSharePanel((s) => s.tabId)
  const button = useRef<HTMLButtonElement>(null)
  if (!kind || (!SHAREABLE.has(kind) && !state)) return null
  const on = state?.status === 'live' || state?.status === 'connecting'
  const live = on && !state.private
  const peers = on ? state.peers : []
  const label = live ? (state.role === 'host' ? (state.internet ? 'Shared' : 'Shared on your network') : 'Editing a shared document') : isTauri ? 'Share' : 'Share on your network'
  const waiting = live ? (state.waiting?.length ?? 0) : 0
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
              <PeerAvatar key={p.client} peer={p} />
            ))}
          </span>
        ) : (
          <Users size={14} />
        )}
        {live && (!peers.length || waiting > 0) && <span className="share-btn-dot" />}
      </button>
      {openFor === activeId && (
        <Suspense fallback={null}>
          <SharePanel tabId={activeId} anchor={button} />
        </Suspense>
      )}
    </>
  )
}
