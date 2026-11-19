import { X } from 'lucide-react'
import type { CSSProperties } from 'react'

/** A linked video playing over its poster, with a button to close it. */
export function VideoFrame({ embed, style, onClose }: { embed: string; style: CSSProperties; onClose: () => void }) {
  return (
    // The slide under it handles clicks and presses; the player keeps its own.
    <div className="present-video" style={style} onClick={(e) => e.stopPropagation()} onMouseDown={(e) => e.stopPropagation()}>
      <iframe src={embed} title="Video" allow="autoplay; encrypted-media; picture-in-picture; fullscreen" allowFullScreen referrerPolicy="strict-origin-when-cross-origin" />
      <button type="button" className="present-video-close" aria-label="Close video" onClick={onClose}>
        <X size={16} />
      </button>
    </div>
  )
}
