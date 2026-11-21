// Where everyone else is in a shared document: their caret or picked frames in their color,
// with their name. The editors place these; session.ts keeps where each person is.

import type { CSSProperties, ReactNode } from 'react'
import type { Peer, Place } from './session'
import './cursors.css'

const nameOf = (peer: Peer) => peer.name || 'Someone'

/**
 * Name tags in the people's colors, side by side when several are at one place. They show while
 * someone moves and for a few seconds after, and again when the pointer is over their caret.
 * `moves` remounts them, which starts that again. `scale` undoes a zoom the tags are drawn inside
 * (a slide's overlay is scaled).
 */
export function PeerTags({ peers, moves, scale = 1, below = false }: { peers: Peer[]; moves: number; scale?: number; below?: boolean }) {
  return (
    <span key={moves} className={`peer-tags ${below ? 'below' : ''}`} style={{ '--peer-scale': scale } as CSSProperties}>
      {peers.map((p) => (
        <span key={p.client} className="peer-tag" style={{ '--peer': p.color } as CSSProperties}>
          {nameOf(p)}
        </span>
      ))}
    </span>
  )
}

/** Places that are the same spot (by `key`), so the people there share one marker. */
export function together<T>(items: { place: Place; key: string; item: T }[]): { peers: Peer[]; moves: number; item: T }[] {
  const groups = new Map<string, { peers: Peer[]; moves: number; item: T }>()
  for (const { place, key, item } of items) {
    const g = groups.get(key)
    if (g) {
      g.peers.push(place.peer)
      g.moves += place.moves
    } else groups.set(key, { peers: [place.peer], moves: place.moves, item })
  }
  return [...groups.values()]
}

/** Someone's caret: a bar in their color, with their name above it (everyone's, when several
 * people's carets are at one place). */
export function PeerCaret({ peers, moves, style, scale = 1 }: { peers: Peer[]; moves: number; style: CSSProperties; scale?: number }) {
  return (
    <div className="peer-caret" data-peer={peers[0].client} style={{ ...style, '--peer': peers[0].color, '--peer-scale': scale } as CSSProperties}>
      <PeerTags peers={peers} moves={moves} scale={scale} />
    </div>
  )
}

/** Part of someone's selection: their color, lightly. */
export function PeerSelection({ peer, style }: { peer: Peer; style: CSSProperties }) {
  return <div className="peer-sel" style={{ ...style, '--peer': peer.color } as CSSProperties} />
}

/** A frame or range people have picked: outlined in the first one's color, with their names on top. */
export function PeerBox({
  peers,
  moves,
  style,
  scale = 1,
  named = true,
  below = false,
  children,
}: {
  peers: Peer[]
  moves: number
  style: CSSProperties
  scale?: number
  named?: boolean
  /** The tags go under the box, for one at the top edge of what's shown. */
  below?: boolean
  children?: ReactNode
}) {
  return (
    <div className="peer-box" data-peer={peers[0].client} style={{ ...style, '--peer': peers[0].color, '--peer-scale': scale } as CSSProperties}>
      {named && <PeerTags peers={peers} moves={moves} scale={scale} below={below} />}
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
