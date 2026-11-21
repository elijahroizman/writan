// Where everyone else is in a shared document: their caret or picked frames in their color,
// with their name. The editors place these; session.ts keeps where each person is.

import type { CSSProperties, ReactNode } from 'react'
import type { Peer } from './session'
import './cursors.css'

const nameOf = (peer: Peer) => peer.name || 'Someone'

/**
 * A name tag in the person's color. It shows while they move and for a few seconds after, and
 * again when the pointer is over their caret or box. `moves` remounts it, which starts that
 * again. `scale` undoes a zoom the tag is drawn inside (a slide's overlay is scaled).
 */
export function PeerTag({ peer, moves, scale = 1, below = false }: { peer: Peer; moves: number; scale?: number; below?: boolean }) {
  return (
    <span key={moves} className={`peer-tag ${below ? 'below' : ''}`} style={{ '--peer-scale': scale } as CSSProperties}>
      {nameOf(peer)}
    </span>
  )
}

/** Someone's caret: a bar in their color, with their name above it. */
export function PeerCaret({ peer, moves, style, scale = 1 }: { peer: Peer; moves: number; style: CSSProperties; scale?: number }) {
  return (
    <div className="peer-caret" data-peer={peer.client} style={{ ...style, '--peer': peer.color, '--peer-scale': scale } as CSSProperties}>
      <PeerTag peer={peer} moves={moves} scale={scale} />
    </div>
  )
}

/** Part of someone's selection: their color, lightly. */
export function PeerSelection({ peer, style }: { peer: Peer; style: CSSProperties }) {
  return <div className="peer-sel" style={{ ...style, '--peer': peer.color } as CSSProperties} />
}

/** A frame or range someone has picked: outlined in their color, with their name on top. */
export function PeerBox({
  peer,
  moves,
  style,
  scale = 1,
  named = true,
  below = false,
  children,
}: {
  peer: Peer
  moves: number
  style: CSSProperties
  scale?: number
  named?: boolean
  /** The tag goes under the box, for one at the top edge of what's shown. */
  below?: boolean
  children?: ReactNode
}) {
  return (
    <div className="peer-box" data-peer={peer.client} style={{ ...style, '--peer': peer.color, '--peer-scale': scale } as CSSProperties}>
      {named && <PeerTag peer={peer} moves={moves} scale={scale} below={below} />}
      {children}
    </div>
  )
}

/** Small dots in the colors of the people at a place (a slide, a sheet), for lists and tabs. */
export function PeerDots({ peers }: { peers: Peer[] }) {
  if (!peers.length) return null
  return (
    <span className="peer-dots" aria-label={peers.map(nameOf).join(', ')}>
      {peers.slice(0, 4).map((p) => (
        <span key={p.client} className="peer-dot" title={nameOf(p)} style={{ background: p.color }} />
      ))}
    </span>
  )
}
