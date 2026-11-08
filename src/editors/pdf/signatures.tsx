import { Calendar, Check, Image as ImageIcon, PenLine, Plus, Type, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { create } from 'zustand'
import { Button, Modal, Segmented } from '../../components/ui'
import * as platform from '../../lib/platform'
import { clearTextSelection } from './viewer/layers'

/**
 * Signatures and initials you've made, kept on this computer (in the app's storage, never in a
 * file until you place one). Each is a trimmed PNG with a transparent background, so it sits on
 * the page like ink.
 */
export interface SavedSignature {
  id: string
  kind: 'signature' | 'initials'
  /** data:image/png;base64,… */
  png: string
  width: number
  height: number
}

/** What a click on the page places. */
export type SignChoice = { type: 'saved'; id: string } | { type: 'date' } | { type: 'check' } | { type: 'cross' }

const KEY = 'workspace.signatures'
const MAX_PER_KIND = 6

function load(): SavedSignature[] {
  try {
    const list = JSON.parse(localStorage.getItem(KEY) || '[]') as unknown
    if (!Array.isArray(list)) return []
    return list.filter(
      (s): s is SavedSignature =>
        !!s && typeof s.id === 'string' && (s.kind === 'signature' || s.kind === 'initials') && typeof s.png === 'string' && s.png.startsWith('data:image/png') && s.width > 0 && s.height > 0,
    )
  } catch {
    return []
  }
}

interface SignatureState {
  list: SavedSignature[]
  choice: SignChoice | null
  add(sig: Omit<SavedSignature, 'id'>): SavedSignature
  remove(id: string): void
  choose(choice: SignChoice | null): void
}

export const useSignatures = create<SignatureState>((set, get) => ({
  list: load(),
  choice: null,
  add: (sig) => {
    const saved = { ...sig, id: Math.random().toString(36).slice(2, 10) }
    // Newest first; the oldest of that kind goes once there are too many.
    const same = get().list.filter((s) => s.kind === sig.kind)
    const drop = same.length >= MAX_PER_KIND ? same[same.length - 1].id : null
    const list = [saved, ...get().list.filter((s) => s.id !== drop)]
    set({ list, choice: { type: 'saved', id: saved.id } })
    persist(list)
    return saved
  },
  remove: (id) => {
    const list = get().list.filter((s) => s.id !== id)
    const choice = get().choice
    set({ list, choice: choice?.type === 'saved' && choice.id === id ? null : choice })
    persist(list)
  },
  choose: (choice) => set({ choice }),
}))

function persist(list: SavedSignature[]) {
  try {
    localStorage.setItem(KEY, JSON.stringify(list))
  } catch {
    void platform.showError('Couldn’t remember that signature: the app’s storage is full. Remove one you don’t use.')
  }
}

/** The PNG's bytes, for the PDF. */
export function pngBytes(dataUrl: string): ArrayBuffer {
  const bin = atob(dataUrl.slice(dataUrl.indexOf(',') + 1))
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
  return bytes.buffer
}

/** Cut a canvas down to what's drawn on it, plus a little margin. Null when it's empty. */
function trimmed(canvas: HTMLCanvasElement, pad = 6): { png: string; width: number; height: number } | null {
  const ctx = canvas.getContext('2d')!
  const { width, height } = canvas
  const data = ctx.getImageData(0, 0, width, height).data
  let x0 = width
  let y0 = height
  let x1 = -1
  let y1 = -1
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      if (data[(y * width + x) * 4 + 3] > 8) {
        if (x < x0) x0 = x
        if (x > x1) x1 = x
        if (y < y0) y0 = y
        if (y > y1) y1 = y
      }
    }
  }
  if (x1 < 0) return null
  x0 = Math.max(0, x0 - pad)
  y0 = Math.max(0, y0 - pad)
  x1 = Math.min(width - 1, x1 + pad)
  y1 = Math.min(height - 1, y1 + pad)
  const out = document.createElement('canvas')
  out.width = x1 - x0 + 1
  out.height = y1 - y0 + 1
  out.getContext('2d')!.drawImage(canvas, x0, y0, out.width, out.height, 0, 0, out.width, out.height)
  return { png: out.toDataURL('image/png'), width: out.width, height: out.height }
}

/** A check mark or cross as a PNG, drawn rather than typed: the PDF's standard fonts don't have them. */
export function markPng(mark: 'check' | 'cross', color = '#000000'): { bytes: ArrayBuffer; width: number; height: number } {
  const size = 96
  const c = document.createElement('canvas')
  c.width = c.height = size
  const ctx = c.getContext('2d')!
  ctx.strokeStyle = color
  ctx.lineWidth = 11
  ctx.lineCap = 'round'
  ctx.lineJoin = 'round'
  ctx.beginPath()
  if (mark === 'check') {
    ctx.moveTo(14, 52)
    ctx.lineTo(38, 78)
    ctx.lineTo(84, 18)
  } else {
    ctx.moveTo(18, 18)
    ctx.lineTo(78, 78)
    ctx.moveTo(78, 18)
    ctx.lineTo(18, 78)
  }
  ctx.stroke()
  return { bytes: pngBytes(c.toDataURL('image/png')), width: size, height: size }
}

/** Handwriting faces most computers have. The first one installed wins. */
const SCRIPT_FONTS = [
  { label: 'Flowing', family: "'Snell Roundhand', 'Segoe Script', 'URW Chancery L', cursive" },
  { label: 'Neat', family: "'Bradley Hand', 'Segoe Print', 'Comic Neue', cursive" },
  { label: 'Formal', family: "'Apple Chancery', 'Palace Script MT', 'Lucida Handwriting', cursive" },
  { label: 'Marker', family: "'Noteworthy', 'Ink Free', 'Chalkboard SE', cursive" },
]
const INKS = ['#111111', '#1f3fbf', '#0b6b3a']

type PadMode = 'draw' | 'type' | 'image'

/** Make a signature or initials: draw it, type it in a handwriting face, or use a picture of it. */
export function SignaturePad({ kind, onClose }: { kind: SavedSignature['kind']; onClose(saved: SavedSignature | null): void }) {
  const [mode, setMode] = useState<PadMode>('draw')
  const [ink, setInk] = useState(INKS[0])
  const [typed, setTyped] = useState('')
  const [font, setFont] = useState(0)
  const [image, setImage] = useState<HTMLImageElement | null>(null)
  const [knockOut, setKnockOut] = useState(true)
  const [drawn, setDrawn] = useState(false)
  /** Bumped after each stroke, so the pad redraws it smoothly in one piece. */
  const [rev, setRev] = useState(0)
  const canvas = useRef<HTMLCanvasElement>(null)
  const strokes = useRef<{ x: number; y: number }[][]>([])
  const W = 640
  const H = 220

  // Everything is drawn from scratch whenever something changes, so switching modes or ink is free.
  useEffect(() => {
    const c = canvas.current
    if (!c) return
    const ctx = c.getContext('2d')!
    ctx.clearRect(0, 0, c.width, c.height)
    if (mode === 'draw') {
      ctx.strokeStyle = ink
      ctx.lineWidth = 3.2
      ctx.lineCap = 'round'
      ctx.lineJoin = 'round'
      for (const s of strokes.current) drawStroke(ctx, s)
    } else if (mode === 'type' && typed.trim()) {
      ctx.fillStyle = ink
      let size = kind === 'initials' ? 120 : 96
      ctx.font = `${size}px ${SCRIPT_FONTS[font].family}`
      while (ctx.measureText(typed).width > c.width - 40 && size > 24) {
        size -= 4
        ctx.font = `${size}px ${SCRIPT_FONTS[font].family}`
      }
      ctx.textBaseline = 'middle'
      ctx.textAlign = 'center'
      ctx.fillText(typed, c.width / 2, c.height / 2)
    } else if (mode === 'image' && image) {
      const scale = Math.min(c.width / image.naturalWidth, c.height / image.naturalHeight, 1)
      const w = image.naturalWidth * scale
      const h = image.naturalHeight * scale
      ctx.drawImage(image, (c.width - w) / 2, (c.height - h) / 2, w, h)
      if (knockOut) removePaper(ctx, c.width, c.height)
    }
  }, [mode, ink, typed, font, image, knockOut, rev, kind])

  const pickImage = async () => {
    const file = (await platform.pickImages())[0]
    if (!file) return
    const url = URL.createObjectURL(file)
    const img = new Image()
    img.src = url
    try {
      await img.decode()
      setImage(img)
    } catch {
      await platform.showError('That picture couldn’t be read.')
    } finally {
      URL.revokeObjectURL(url)
    }
  }

  const ready = mode === 'draw' ? drawn : mode === 'type' ? !!typed.trim() : !!image
  const save = () => {
    const c = canvas.current
    const result = c && trimmed(c)
    if (!result) return
    onClose(useSignatures.getState().add({ kind, ...result }))
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onClose(null)
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [onClose])

  const what = kind === 'initials' ? 'initials' : 'signature'
  return (
    <Modal onClose={() => onClose(null)} placement="top" dim="light" layer="dialog">
      <div className="dialog signature-dialog" role="dialog" aria-modal="true" aria-label={`New ${what}`}>
        <div className="signature-head">
          <h2>New {what}</h2>
          <Segmented
            value={mode}
            onChange={setMode}
            options={[
              { value: 'draw', label: 'Draw', icon: <PenLine size={13} /> },
              { value: 'type', label: 'Type', icon: <Type size={13} /> },
              { value: 'image', label: 'Picture', icon: <ImageIcon size={13} /> },
            ]}
          />
        </div>
        <div className="signature-canvas-wrap">
          <canvas
            ref={canvas}
            width={W}
            height={H}
            className={`signature-canvas ${mode}`}
            onPointerDown={(e) => {
              if (mode !== 'draw' || e.button !== 0) return
              const el = e.currentTarget
              el.setPointerCapture(e.pointerId)
              const r = el.getBoundingClientRect()
              const at = (ev: { clientX: number; clientY: number }) => ({ x: ((ev.clientX - r.left) / r.width) * W, y: ((ev.clientY - r.top) / r.height) * H })
              const stroke = [at(e)]
              strokes.current.push(stroke)
              const ctx = el.getContext('2d')!
              ctx.strokeStyle = ink
              ctx.lineWidth = 3.2
              ctx.lineCap = 'round'
              ctx.lineJoin = 'round'
              const move = (ev: PointerEvent) => {
                for (const c of ev.getCoalescedEvents?.() ?? [ev]) stroke.push(at(c))
                drawStroke(ctx, stroke.slice(-4))
              }
              const up = () => {
                el.removeEventListener('pointermove', move)
                el.removeEventListener('pointerup', up)
                el.removeEventListener('pointercancel', up)
                if (stroke.length === 1) stroke.push({ x: stroke[0].x + 0.5, y: stroke[0].y + 0.5 })
                setDrawn(true)
                setRev((r) => r + 1)
              }
              el.addEventListener('pointermove', move)
              el.addEventListener('pointerup', up)
              el.addEventListener('pointercancel', up)
            }}
          />
          {mode === 'draw' && !drawn && <span className="signature-placeholder">Sign here with the trackpad or mouse</span>}
          {mode === 'image' && !image && (
            <Button className="signature-placeholder-btn" onClick={() => void pickImage()}>
              <ImageIcon size={14} /> Choose a picture of your {what}…
            </Button>
          )}
          <span className="signature-line" />
        </div>
        <div className="signature-options">
          {mode === 'type' && (
            <>
              <input className="signature-input" autoFocus placeholder={kind === 'initials' ? 'Your initials' : 'Your name'} value={typed} onChange={(e) => setTyped(e.currentTarget.value)} />
              <div className="signature-fonts">
                {SCRIPT_FONTS.map((f, i) => (
                  <button key={f.label} type="button" className={`signature-font ${font === i ? 'on' : ''}`} style={{ fontFamily: f.family }} onClick={() => setFont(i)}>
                    {typed.trim() || f.label}
                  </button>
                ))}
              </div>
            </>
          )}
          {mode === 'image' && image && (
            <label className="signature-check">
              <input type="checkbox" checked={knockOut} onChange={(e) => setKnockOut(e.currentTarget.checked)} /> Remove the paper (make white see-through)
            </label>
          )}
          <div className="signature-toolbar">
            {mode !== 'image' && (
              <div className="swatches">
                {INKS.map((c) => (
                  <button key={c} type="button" aria-label={`Ink ${c}`} className={`swatch ${ink === c ? 'on' : ''}`} style={{ background: c }} onClick={() => setInk(c)} />
                ))}
              </div>
            )}
            {mode === 'draw' && drawn && (
              <Button
                onClick={() => {
                  strokes.current = []
                  setDrawn(false)
                  setRev((r) => r + 1)
                }}
              >
                Clear
              </Button>
            )}
            {mode === 'image' && image && <Button onClick={() => void pickImage()}>Another picture…</Button>}
          </div>
        </div>
        <p className="signature-note faint">Kept on this computer only, for signing PDFs in Writan.</p>
        <div className="dialog-actions">
          <Button onClick={() => onClose(null)}>Cancel</Button>
          <Button variant="primary" disabled={!ready} onClick={save}>
            Save {what}
          </Button>
        </div>
      </div>
    </Modal>
  )
}

/** A smooth line through the points (midpoint quadratic curves). */
function drawStroke(ctx: CanvasRenderingContext2D, pts: { x: number; y: number }[]) {
  if (!pts.length) return
  ctx.beginPath()
  ctx.moveTo(pts[0].x, pts[0].y)
  for (let i = 1; i < pts.length - 1; i++) {
    const mx = (pts[i].x + pts[i + 1].x) / 2
    const my = (pts[i].y + pts[i + 1].y) / 2
    ctx.quadraticCurveTo(pts[i].x, pts[i].y, mx, my)
  }
  const last = pts[pts.length - 1]
  ctx.lineTo(last.x, last.y)
  ctx.stroke()
}

/** Paper becomes see-through: light pixels fade out, ink stays. */
function removePaper(ctx: CanvasRenderingContext2D, w: number, h: number) {
  const img = ctx.getImageData(0, 0, w, h)
  const d = img.data
  for (let i = 0; i < d.length; i += 4) {
    const light = (d[i] * 0.299 + d[i + 1] * 0.587 + d[i + 2] * 0.114) / 255
    // Above 0.85 is paper; below 0.55 is ink; between fades.
    const alpha = light > 0.85 ? 0 : light < 0.55 ? 1 : (0.85 - light) / 0.3
    d[i + 3] = Math.round(d[i + 3] * alpha)
  }
  ctx.putImageData(img, 0, 0)
}

/** The inspector's Sign section: your signatures, initials and quick marks. */
export function SignPanel({ onNew }: { onNew(kind: SavedSignature['kind']): void }) {
  const list = useSignatures((s) => s.list)
  const choice = useSignatures((s) => s.choice)
  const { choose, remove } = useSignatures.getState()
  const isOn = (c: SignChoice) => choice?.type === c.type && (c.type !== 'saved' || (choice.type === 'saved' && choice.id === c.id))

  const group = (kind: SavedSignature['kind'], title: string) => (
    <div className="sign-group">
      <div className="sign-group-title faint">{title}</div>
      <div className="sign-list">
        {list
          .filter((s) => s.kind === kind)
          .map((s) => (
            <div key={s.id} className={`sign-item ${isOn({ type: 'saved', id: s.id }) ? 'on' : ''}`}>
              <button type="button" className="sign-pick" title="Click a page to place it" onClick={() => choose({ type: 'saved', id: s.id })}>
                <img src={s.png} alt={title} draggable={false} />
              </button>
              <button type="button" className="sign-remove" aria-label={`Forget this ${kind}`} title="Forget" onClick={() => remove(s.id)}>
                <X size={11} />
              </button>
            </div>
          ))}
        <button type="button" className="sign-item sign-new" onClick={() => onNew(kind)}>
          <Plus size={14} />
          <span>{kind === 'initials' ? 'Add initials' : 'Add signature'}</span>
        </button>
      </div>
    </div>
  )

  return (
    <>
      {group('signature', 'Signatures')}
      {group('initials', 'Initials')}
      <div className="sign-group">
        <div className="sign-group-title faint">Marks</div>
        <div className="sign-marks">
          <Button className={isOn({ type: 'date' }) ? 'on' : ''} onClick={() => choose({ type: 'date' })}>
            <Calendar size={14} /> Date
          </Button>
          <Button className={isOn({ type: 'check' }) ? 'on' : ''} title="A check mark" onClick={() => choose({ type: 'check' })}>
            <Check size={14} /> Check
          </Button>
          <Button className={isOn({ type: 'cross' }) ? 'on' : ''} title="A cross" onClick={() => choose({ type: 'cross' })}>
            <X size={14} /> Cross
          </Button>
        </div>
      </div>
    </>
  )
}

/** Click anywhere on a page to place what's picked in the Sign section. */
export function PlaceLayer({ unit, onPlace }: { unit: number; onPlace(x: number, y: number): void }) {
  return (
    <div
      className="tool-layer place"
      onPointerDown={(e) => {
        if (e.button !== 0) return
        e.preventDefault()
        clearTextSelection()
        const r = e.currentTarget.getBoundingClientRect()
        onPlace((e.clientX - r.left) / unit, (e.clientY - r.top) / unit)
      }}
    />
  )
}
