// A small viewer for PNG, JPEG, HEIC and WebP, with a few touch-ups: highlight, draw, lines and arrows,
// box, crop and rotate, and copy to the clipboard. Edits are a list replayed over the decoded picture, so undo
// is exact and saving re-encodes once from the original pixels. An unedited file is saved back
// byte for byte. HEIC can't be written, so HEIC photos open for viewing, with a button that makes
// a JPEG copy to mark up.

import type { ImageOcrResult, PluginEditor, PluginEditorContext } from '../../src/plugins/api'
import type { IconName } from './icons'
import { icon } from './icons'
import css from './images.css?inline'

type Format = 'png' | 'jpeg' | 'heic' | 'webp'
type Tool = 'pan' | 'highlight' | 'pen' | 'line' | 'arrow' | 'box' | 'crop'
/** The tools that draw in a colour. */
type Marker = 'highlight' | 'pen' | 'line' | 'arrow' | 'box'
type Point = { x: number; y: number }
type Rect = { x: number; y: number; w: number; h: number }
type Edit =
  | { type: 'stroke'; tool: 'highlight' | 'pen'; color: string; width: number; points: Point[] }
  | { type: 'line'; arrow: boolean; color: string; width: number; from: Point; to: Point }
  | { type: 'box'; color: string; width: number; rect: Rect }
  | { type: 'crop'; rect: Rect }
  | { type: 'rotate'; turns: 1 | -1 }

const COLORS = [
  { value: '#ffd60a', name: 'Yellow' },
  { value: '#ff3b30', name: 'Red' },
  { value: '#0a84ff', name: 'Blue' },
  { value: '#30d158', name: 'Green' },
  { value: '#1c1c1e', name: 'Black' },
]
/** Marks are this many screen pixels wide at the zoom they're drawn at. */
const WIDTHS: Record<Marker, number> = { highlight: 18, pen: 3, line: 3, arrow: 3, box: 3 }
const HIGHLIGHT_ALPHA = 0.38
const MIN_ZOOM = 0.02
const MAX_ZOOM = 16
const JPEG_QUALITY = 0.92
const isMac = /Mac|iPhone|iPad/.test(navigator.platform)

let styled = false

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className?: string, text?: string) {
  const node = document.createElement(tag)
  if (className) node.className = className
  if (text) node.textContent = text
  return node
}

function canvas(width: number, height: number) {
  const c = document.createElement('canvas')
  c.width = Math.max(1, Math.round(width))
  c.height = Math.max(1, Math.round(height))
  return c
}

const context2d = (c: HTMLCanvasElement) => c.getContext('2d')!

/** What the bytes are, from their first few (a file's extension can be wrong). */
export function sniff(bytes: Uint8Array): Format | null {
  if (bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) return 'png'
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return 'jpeg'
  if (String.fromCharCode(...bytes.subarray(0, 4)) === 'RIFF' && String.fromCharCode(...bytes.subarray(8, 12)) === 'WEBP') return 'webp'
  const box = String.fromCharCode(...bytes.subarray(4, 12))
  if (box.startsWith('ftyp') && /^(hei[cmsx]|hev[cmsx]|mif1|msf1)$/.test(box.slice(4))) return 'heic'
  return null
}

const MIME: Record<Format, string> = { png: 'image/png', jpeg: 'image/jpeg', heic: 'image/heic', webp: 'image/webp' }

/** The picture as a canvas. The web view decodes PNG and JPEG (turning them as their EXIF says); HEIC falls back to libheif. */
async function decode(bytes: Uint8Array, format: Format | null): Promise<HTMLCanvasElement> {
  try {
    const url = URL.createObjectURL(new Blob([bytes.slice()], format ? { type: MIME[format] } : {}))
    try {
      const img = new Image()
      img.src = url
      await img.decode()
      const c = canvas(img.naturalWidth, img.naturalHeight)
      context2d(c).drawImage(img, 0, 0)
      return c
    } finally {
      URL.revokeObjectURL(url)
    }
  } catch (err) {
    if (format !== 'heic') throw new Error(format ? 'This picture is damaged, or in a form Writan can’t read.' : 'Writan couldn’t read this picture.', { cause: err })
  }
  const { decodeHeic } = await import('./heic')
  const data = await decodeHeic(bytes)
  const c = canvas(data.width, data.height)
  context2d(c).putImageData(data, 0, 0)
  return c
}

function encode(c: HTMLCanvasElement, format: 'png' | 'jpeg'): Promise<Uint8Array> {
  return new Promise((resolve, reject) => {
    if (format === 'jpeg') {
      // JPEG has no transparency: put a transparent PNG on white rather than black.
      const flat = canvas(c.width, c.height)
      const ctx = context2d(flat)
      ctx.fillStyle = '#ffffff'
      ctx.fillRect(0, 0, flat.width, flat.height)
      ctx.drawImage(c, 0, 0)
      c = flat
    }
    c.toBlob(
      (blob) => (blob ? blob.arrayBuffer().then((b) => resolve(new Uint8Array(b)), reject) : reject(new Error('Couldn’t encode the picture.'))),
      MIME[format],
      JPEG_QUALITY,
    )
  })
}

type Mark = Extract<Edit, { type: 'stroke' | 'line' | 'box' }>

function drawEdit(ctx: CanvasRenderingContext2D, edit: Mark) {
  ctx.save()
  ctx.strokeStyle = edit.color
  ctx.fillStyle = edit.color
  ctx.lineWidth = edit.width
  ctx.lineCap = 'round'
  ctx.lineJoin = 'round'
  if (edit.type === 'line') {
    const { from, to, width } = edit
    const angle = Math.atan2(to.y - from.y, to.x - from.x)
    const length = Math.hypot(to.x - from.x, to.y - from.y)
    // The head grows with the line's width but never outgrows a short arrow.
    const head = edit.arrow ? Math.min(width * 4.5, length * 0.6) : 0
    // The shaft stops inside the head, so its round end doesn't poke through the point.
    const end = { x: to.x - Math.cos(angle) * head * 0.7, y: to.y - Math.sin(angle) * head * 0.7 }
    ctx.beginPath()
    ctx.moveTo(from.x, from.y)
    ctx.lineTo(end.x, end.y)
    ctx.stroke()
    if (edit.arrow && head > 0) {
      const spread = Math.PI / 7
      ctx.beginPath()
      ctx.moveTo(to.x, to.y)
      ctx.lineTo(to.x - Math.cos(angle - spread) * head, to.y - Math.sin(angle - spread) * head)
      ctx.lineTo(to.x - Math.cos(angle + spread) * head, to.y - Math.sin(angle + spread) * head)
      ctx.closePath()
      ctx.lineWidth = width / 2
      ctx.fill()
      ctx.stroke()
    }
  } else if (edit.type === 'box') {
    const { x, y, w, h } = edit.rect
    ctx.strokeRect(x, y, w, h)
  } else {
    // One path drawn once, so a highlighter stroke crossing itself doesn't get darker.
    if (edit.tool === 'highlight') ctx.globalAlpha = HIGHLIGHT_ALPHA
    const pts = edit.points
    ctx.beginPath()
    ctx.moveTo(pts[0].x, pts[0].y)
    if (pts.length === 1) ctx.lineTo(pts[0].x + 0.01, pts[0].y)
    // Through the midpoints, so the line is smooth rather than a polygon.
    for (let i = 1; i < pts.length - 1; i++) ctx.quadraticCurveTo(pts[i].x, pts[i].y, (pts[i].x + pts[i + 1].x) / 2, (pts[i].y + pts[i + 1].y) / 2)
    if (pts.length > 1) ctx.lineTo(pts[pts.length - 1].x, pts[pts.length - 1].y)
    ctx.stroke()
  }
  ctx.restore()
}

/** `c` with one edit made: marks draw onto it, crops and turns make a new canvas. */
function apply(c: HTMLCanvasElement, edit: Edit): HTMLCanvasElement {
  if (edit.type === 'stroke' || edit.type === 'line' || edit.type === 'box') {
    drawEdit(context2d(c), edit)
    return c
  }
  if (edit.type === 'crop') {
    const out = canvas(edit.rect.w, edit.rect.h)
    context2d(out).drawImage(c, -edit.rect.x, -edit.rect.y)
    return out
  }
  const out = canvas(c.height, c.width)
  const ctx = context2d(out)
  ctx.translate(out.width / 2, out.height / 2)
  ctx.rotate((edit.turns * Math.PI) / 2)
  ctx.drawImage(c, -c.width / 2, -c.height / 2)
  return out
}

/** `to`, moved onto the nearest 45° line through `from` (Shift while drawing a line). */
function snap(from: Point, to: Point): Point {
  const step = Math.PI / 4
  const angle = Math.round(Math.atan2(to.y - from.y, to.x - from.x) / step) * step
  const length = Math.hypot(to.x - from.x, to.y - from.y)
  return { x: from.x + Math.cos(angle) * length, y: from.y + Math.sin(angle) * length }
}

const isMarker = (t: Tool): t is Marker => t !== 'pan' && t !== 'crop'

function normalize(a: Point, b: Point): Rect {
  return { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) }
}

function clampRect(r: Rect, width: number, height: number): Rect {
  const x = Math.max(0, Math.min(width, Math.round(r.x)))
  const y = Math.max(0, Math.min(height, Math.round(r.y)))
  return { x, y, w: Math.max(0, Math.min(width, Math.round(r.x + r.w)) - x), h: Math.max(0, Math.min(height, Math.round(r.y + r.h)) - y) }
}

export function mountViewer(element: HTMLElement, context: PluginEditorContext): PluginEditor {
  if (!styled) {
    styled = true
    const style = document.createElement('style')
    style.dataset.workspacePlugin = 'images'
    style.textContent = css
    document.head.appendChild(style)
  }

  const original = context.bytes ?? new Uint8Array()
  const format = sniff(original)
  const editable = format === 'png' || format === 'jpeg'

  const root = el('div', 'img-viewer')
  root.tabIndex = -1
  const stage = el('div', 'img-stage tool-pan')
  const page = el('div', 'img-page')
  const overlay = el('canvas', 'img-overlay')
  const status = el('div', 'img-status')
  const toolbar = el('div', 'img-toolbar')
  toolbar.setAttribute('role', 'toolbar')
  stage.appendChild(page)
  root.append(stage, toolbar, status)
  element.appendChild(root)

  let source: HTMLCanvasElement | null = null
  /** The picture with every edit made, on screen. */
  let work: HTMLCanvasElement | null = null
  const edits: Edit[] = []
  const undone: Edit[] = []
  let tool: Tool = 'pan'
  const colors: Record<Marker, string> = { highlight: COLORS[0].value, pen: COLORS[1].value, line: COLORS[1].value, arrow: COLORS[1].value, box: COLORS[1].value }
  /** Null: fit the window. */
  let zoom: number | null = null
  let scale = 1
  let selection: Rect | null = null
  let active = true
  let destroyed = false
  let note: { text: string; timer: number } | null = null

  // ── Layout and zoom

  function fitScale() {
    if (!work) return 1
    // Inside the stage's padding (the toolbar's room), with a little air.
    const style = getComputedStyle(stage)
    const width = stage.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight) - 8
    const height = stage.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom) - 8
    return Math.min(1, width / work.width, height / work.height)
  }

  function layout() {
    if (!work) return
    scale = Math.max(MIN_ZOOM, zoom ?? fitScale())
    const w = work.width * scale
    const h = work.height * scale
    page.style.width = `${w}px`
    page.style.height = `${h}px`
    const dpr = window.devicePixelRatio || 1
    overlay.width = Math.max(1, Math.round(w * dpr))
    overlay.height = Math.max(1, Math.round(h * dpr))
    // Pixels stay sharp when zoomed in past 100%.
    work.style.imageRendering = scale > 2 ? 'pixelated' : ''
    drawOverlay()
    updateStatus()
  }

  /** Zoom to `next`, keeping the spot under (clientX, clientY), or the middle, still. */
  function setZoom(next: number | null, at?: { clientX: number; clientY: number }) {
    if (!work) return
    const box = stage.getBoundingClientRect()
    const anchor = at ?? { clientX: box.left + box.width / 2, clientY: box.top + box.height / 2 }
    const before = page.getBoundingClientRect()
    const spot = { x: (anchor.clientX - before.left) / scale, y: (anchor.clientY - before.top) / scale }
    zoom = next === null ? null : Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, next))
    layout()
    const after = page.getBoundingClientRect()
    stage.scrollLeft += after.left + spot.x * scale - anchor.clientX
    stage.scrollTop += after.top + spot.y * scale - anchor.clientY
  }

  const step = (dir: 1 | -1) => setZoom(scale * (dir > 0 ? 1.25 : 0.8))

  stage.addEventListener(
    'wheel',
    (e) => {
      // Pinch on a trackpad arrives as a wheel with ctrlKey.
      if (!e.ctrlKey && !e.metaKey) return
      e.preventDefault()
      setZoom(scale * Math.exp(-e.deltaY * 0.01), e)
    },
    { passive: false },
  )
  stage.addEventListener('dblclick', (e) => {
    if (tool !== 'pan') return
    setZoom(zoom === null && scale < 1 ? 1 : null, e)
  })
  const resize = new ResizeObserver(() => zoom === null && layout())
  resize.observe(stage)

  function showWork(next: HTMLCanvasElement) {
    if (work !== next) {
      work?.remove()
      next.className = 'img-picture'
      page.replaceChildren(next, overlay)
      work = next
    }
    layout()
  }

  function updateStatus() {
    if (!work) return
    status.textContent = note?.text ?? `${work.width} × ${work.height}${format ? ` · ${format.toUpperCase()}` : ''} · ${Math.round(scale * 100)}%`
  }

  function flash(text: string) {
    if (note) window.clearTimeout(note.timer)
    note = { text, timer: window.setTimeout(() => ((note = null), updateStatus()), 2400) }
    updateStatus()
  }

  // ── Edits

  function commit(edit: Edit) {
    if (!work) return
    edits.push(edit)
    undone.length = 0
    showWork(apply(work, edit))
    changed()
  }

  /** Everything again from the decoded picture (after undo). */
  function rebuild() {
    if (!source) return
    let c = canvas(source.width, source.height)
    context2d(c).drawImage(source, 0, 0)
    for (const edit of edits) c = apply(c, edit)
    showWork(c)
  }

  function undo() {
    const edit = edits.pop()
    if (!edit) return
    undone.push(edit)
    rebuild()
    changed()
  }

  function redo() {
    const edit = undone.pop()
    if (!edit || !work) return
    edits.push(edit)
    showWork(apply(work, edit))
    changed()
  }

  function changed() {
    context.onChange()
    refreshToolbar()
  }

  function rotate(turns: 1 | -1) {
    cancelSelection()
    // The view turns with the picture, so a fitted picture fits again.
    commit({ type: 'rotate', turns })
  }

  function applyCrop() {
    if (!selection || !work) return
    const rect = clampRect(selection, work.width, work.height)
    selection = null
    if (rect.w < 2 || rect.h < 2) return drawOverlay()
    commit({ type: 'crop', rect })
    setTool('pan')
  }

  function cancelSelection() {
    selection = null
    drawOverlay()
    refreshToolbar()
  }

  // ── Pointer: pan, draw, select

  /** Where a pointer event lands on the picture, in its pixels. */
  function toImage(e: { clientX: number; clientY: number }): Point {
    const r = page.getBoundingClientRect()
    return { x: (e.clientX - r.left) / scale, y: (e.clientY - r.top) / scale }
  }

  /** A line or arrow drawn now, as wide on screen as the other marks. */
  const lineEdit = (t: 'line' | 'arrow', from: Point, to: Point): Edit & { type: 'line' } => ({ type: 'line', arrow: t === 'arrow', color: colors[t], width: WIDTHS[t] / scale, from, to })

  let drag:
    | { kind: 'pan'; x: number; y: number; left: number; top: number }
    | { kind: 'stroke'; edit: Extract<Edit, { type: 'stroke' }> }
    | { kind: 'rect'; from: Point; to: Point }
    | null = null

  stage.addEventListener('pointerdown', (e) => {
    if (e.button !== 0 || !work) return
    root.focus({ preventScroll: true })
    stage.setPointerCapture(e.pointerId)
    if (tool === 'pan') {
      drag = { kind: 'pan', x: e.clientX, y: e.clientY, left: stage.scrollLeft, top: stage.scrollTop }
      stage.classList.add('panning')
    } else if (tool === 'highlight' || tool === 'pen') {
      drag = { kind: 'stroke', edit: { type: 'stroke', tool, color: colors[tool], width: WIDTHS[tool] / scale, points: [toImage(e)] } }
    } else {
      const at = toImage(e)
      drag = { kind: 'rect', from: at, to: at }
      if (tool === 'crop') selection = null
    }
    drawOverlay()
  })

  stage.addEventListener('pointermove', (e) => {
    if (!drag) return
    if (drag.kind === 'pan') {
      stage.scrollLeft = drag.left - (e.clientX - drag.x)
      stage.scrollTop = drag.top - (e.clientY - drag.y)
      return
    }
    const at = toImage(e)
    if (drag.kind === 'stroke') {
      const pts = drag.edit.points
      // Shift draws a straight line from where the stroke began.
      if (e.shiftKey) drag.edit.points = [pts[0], at]
      else pts.push(at)
    } else {
      // Shift keeps a line or arrow level, upright or at 45°.
      drag.to = e.shiftKey && (tool === 'line' || tool === 'arrow') ? snap(drag.from, at) : at
      if (tool === 'crop') selection = normalize(drag.from, drag.to)
    }
    drawOverlay()
  })

  const endDrag = () => {
    const done = drag
    drag = null
    stage.classList.remove('panning')
    if (!done || done.kind === 'pan') return
    if (done.kind === 'stroke') commit(done.edit)
    else if (tool === 'line' || tool === 'arrow') {
      const line = lineEdit(tool, done.from, done.to)
      if (Math.hypot(line.to.x - line.from.x, line.to.y - line.from.y) * scale > 3) commit(line)
    } else if (tool === 'box') {
      const rect = normalize(done.from, done.to)
      if (rect.w * scale > 3 && rect.h * scale > 3) commit({ type: 'box', color: colors.box, width: WIDTHS.box / scale, rect })
    } else if (tool === 'crop' && selection && work) {
      selection = clampRect(selection, work.width, work.height)
      if (selection.w < 2 || selection.h < 2) selection = null
    }
    drawOverlay()
    refreshToolbar()
  }
  stage.addEventListener('pointerup', endDrag)
  stage.addEventListener('pointercancel', endDrag)

  /** The mark being drawn, and the crop selection with the rest of the picture dimmed. */
  function drawOverlay() {
    const ctx = context2d(overlay)
    ctx.setTransform(1, 0, 0, 1, 0, 0)
    ctx.clearRect(0, 0, overlay.width, overlay.height)
    if (!work) return
    const k = (window.devicePixelRatio || 1) * scale
    ctx.setTransform(k, 0, 0, k, 0, 0)
    if (drag?.kind === 'stroke') drawEdit(ctx, drag.edit)
    if (drag?.kind === 'rect' && (tool === 'line' || tool === 'arrow')) drawEdit(ctx, lineEdit(tool, drag.from, drag.to))
    if (drag?.kind === 'rect' && tool === 'box') drawEdit(ctx, { type: 'box', color: colors.box, width: WIDTHS.box / scale, rect: normalize(drag.from, drag.to) })
    if (selection) {
      const { x, y, w, h } = selection
      ctx.fillStyle = 'rgba(0, 0, 0, 0.45)'
      ctx.beginPath()
      ctx.rect(0, 0, work.width, work.height)
      ctx.rect(x, y, w, h)
      ctx.fill('evenodd')
      ctx.strokeStyle = '#ffffff'
      ctx.lineWidth = 1.5 / scale
      ctx.setLineDash([6 / scale, 4 / scale])
      ctx.strokeRect(x, y, w, h)
    }
  }

  // ── Copy, export, HEIC copies

  async function copy() {
    if (!work) return
    let c = work
    if (selection) {
      c = canvas(selection.w, selection.h)
      context2d(c).drawImage(work, -selection.x, -selection.y)
    }
    try {
      // Safari wants the item now, with the data still coming.
      await navigator.clipboard.write([new ClipboardItem({ 'image/png': encode(c, 'png').then((b) => new Blob([b.slice()], { type: 'image/png' })) })])
      flash(selection ? 'Copied the selection.' : 'Copied the picture.')
    } catch {
      flash('Couldn’t copy the picture.')
    }
  }

  async function saveCopy(as: 'png' | 'jpeg') {
    if (!work || !context.saveFile) return
    await context.saveFile({ bytes: await encode(work, as), name: `${context.title}.${as === 'png' ? 'png' : 'jpg'}` })
  }

  async function editCopy() {
    if (!work || !context.openDocument) return
    context.openDocument({ bytes: await encode(work, 'jpeg'), name: `${context.title}.jpg` })
  }

  const ocrPanel = el('section', 'img-ocr-panel')
  ocrPanel.hidden = true
  ocrPanel.setAttribute('aria-label', 'Recognized text')
  const ocrHeading = el('h3', undefined, 'Recognized text')
  const ocrLanguages = el('p', 'muted')
  const ocrStatus = el('p', 'muted')
  ocrStatus.setAttribute('role', 'status')
  const ocrText = el('textarea')
  ocrText.readOnly = true
  ocrText.dir = 'auto'
  ocrText.hidden = true
  ocrText.setAttribute('aria-label', 'Recognized text')
  const ocrActions = el('div', 'img-ocr-actions')
  let ocrJob: AbortController | undefined
  let ocrResult: ImageOcrResult | undefined
  let ocrWork: HTMLCanvasElement | undefined
  const ocrAction = (label: string, action: () => void) => {
    const btn = el('button', 'btn', label)
    btn.type = 'button'
    btn.addEventListener('click', action)
    return btn
  }
  const copyText = ocrAction('Copy Text', () => void deliver(() => navigator.clipboard.writeText(ocrResult!.text)))
  const copyMarkdown = ocrAction('Copy Markdown', () => void deliver(() => navigator.clipboard.writeText(ocrResult!.markdown)))
  const saveDocx = ocrAction('Save as DOCX', () => void deliver(() => ocrResult!.saveDocx()))
  const savePdf = ocrAction('Save as Searchable PDF', () => void runOcr(true, true))
  const cancelOcr = ocrAction('Cancel', () => ocrJob?.abort())
  const closeOcr = ocrAction('Close', () => {
    ocrJob?.abort()
    ocrPanel.hidden = true
    recognizeButton?.focus()
  })
  ocrActions.append(copyText, copyMarkdown, saveDocx, savePdf, cancelOcr, closeOcr)
  ocrPanel.append(ocrHeading, ocrLanguages, ocrStatus, ocrText, ocrActions)
  root.append(ocrPanel)

  function showOcrStatus(message: string) {
    ocrStatus.textContent = message
  }

  function refreshOcrActions() {
    ocrText.hidden = !ocrResult
    copyText.hidden = copyMarkdown.hidden = saveDocx.hidden = savePdf.hidden = !ocrResult
    copyText.disabled = copyMarkdown.disabled = saveDocx.disabled = savePdf.disabled = !!ocrJob
    cancelOcr.hidden = !ocrJob
  }

  async function deliver(action: () => Promise<boolean | void>) {
    try {
      if (await action() !== false) showOcrStatus('Done.')
    } catch (err) {
      showOcrStatus(err instanceof Error ? err.message : String(err))
    }
  }

  async function runOcr(searchable = false, reuse = false) {
    const service = context.imageOcr
    if (!service || ocrJob || destroyed) return
    const job = new AbortController()
    ocrJob = job
    ocrPanel.hidden = false
    ocrLanguages.textContent = service.selectedLanguages().join(', ')
    showOcrStatus('Preparing picture…')
    if (!reuse || ocrWork !== work) ocrResult = undefined
    refreshOcrActions()
    try {
      await ready
      job.signal.throwIfAborted()
      if (!work) throw new Error('Open a picture to recognize text.')
      if (!ocrResult) {
        const snapshot = work
        const useOriginal = format && format !== 'heic' && !edits.length
        const bytes = useOriginal ? original : await encode(snapshot, 'png')
        ocrResult = await service.recognize(
          { bytes, mime: useOriginal ? MIME[format] : MIME.png },
          { signal: job.signal, onProgress: showOcrStatus },
        )
        ocrWork = snapshot
      }
      ocrText.value = ocrResult.text
      if (searchable) {
        showOcrStatus('Building searchable PDF…')
        const saved = await ocrResult.saveSearchablePdf(job.signal)
        showOcrStatus(saved ? 'Searchable PDF saved.' : 'Save cancelled.')
      } else {
        showOcrStatus(ocrResult.text ? 'Recognition complete.' : 'No text was found.')
      }
    } catch (err) {
      showOcrStatus(job.signal.aborted ? 'Recognition cancelled.' : err instanceof Error ? err.message : String(err))
    } finally {
      ocrJob = undefined
      refreshOcrActions()
    }
  }

  // ── Toolbar

  const buttons = new Map<string, HTMLButtonElement>()
  function button(id: string, iconName: IconName, label: string, onClick: () => void) {
    const b = el('button', 'icon-btn')
    b.type = 'button'
    b.title = label
    b.setAttribute('aria-label', label)
    b.appendChild(icon(iconName))
    b.addEventListener('click', onClick)
    buttons.set(id, b)
    return b
  }
  const divider = () => el('span', 'img-divider')
  const keyLabel = (label: string, keys: string) => context.shortcut ? `${label} (${context.shortcut(keys)})` : label

  const tools: { id: Tool; icon: IconName; label: string }[] = [
    { id: 'pan', icon: 'hand', label: 'Move around (drag), double-click to zoom' },
    { id: 'highlight', icon: 'highlighter', label: 'Highlight (hold Shift for a straight line)' },
    { id: 'pen', icon: 'pen', label: 'Draw (hold Shift for a straight line)' },
    { id: 'line', icon: 'line', label: 'Line (hold Shift to keep it level or at 45°)' },
    { id: 'arrow', icon: 'arrow', label: 'Arrow (hold Shift to keep it level or at 45°)' },
    { id: 'box', icon: 'box', label: 'Box' },
    { id: 'crop', icon: 'crop', label: 'Select, to crop or copy part of the picture' },
  ]
  const swatches = el('span', 'img-swatches')
  const cropBar = el('span', 'img-crop-bar')
  const cropApply = el('button', 'btn primary', 'Crop')
  const cropCancel = el('button', 'btn', 'Cancel')
  cropApply.type = cropCancel.type = 'button'
  cropApply.addEventListener('click', applyCrop)
  cropCancel.addEventListener('click', cancelSelection)
  cropBar.append(cropApply, cropCancel)

  const menu = el('div', 'img-menu')
  menu.hidden = true
  const menuItem = (label: string, onClick: () => void) => {
    const item = el('button', 'img-menu-item', label)
    item.type = 'button'
    item.addEventListener('click', () => {
      menu.hidden = true
      onClick()
    })
    menu.appendChild(item)
  }
  if (context.saveFile) {
    menuItem('Save a copy as PNG…', () => void saveCopy('png'))
    menuItem('Save a copy as JPEG…', () => void saveCopy('jpeg'))
  }
  if (context.imageOcr) menuItem('Save as Searchable PDF', () => void runOcr(true))
  menuItem('Print…', () => void print())

  if (editable) {
    for (const t of tools) toolbar.appendChild(button(`tool-${t.id}`, t.icon, t.label, () => setTool(t.id)))
    toolbar.append(divider(), button('rotate-left', 'rotateLeft', 'Turn left', () => rotate(-1)), button('rotate-right', 'rotateRight', 'Turn right', () => rotate(1)))
    toolbar.append(divider(), button('undo', 'undo', keyLabel('Undo', '\u2318Z'), undo), button('redo', 'redo', keyLabel('Redo', isMac ? '\u21e7\u2318Z' : '\u2318Y'), redo))
    toolbar.append(swatches, cropBar, divider())
  } else if ((format === 'heic' || format === 'webp') && context.openDocument) {
    const editButton = el('button', 'btn img-text-btn', 'Edit a JPEG copy')
    editButton.type = 'button'
    editButton.title = 'Open a JPEG copy in a new tab to mark up and save.'
    editButton.addEventListener('click', () => void editCopy())
    toolbar.append(editButton, divider())
  }
  const recognizeButton = context.imageOcr ? el('button', 'btn img-text-btn', 'Recognize Text (OCR)') : null
  if (recognizeButton) {
    recognizeButton.type = 'button'
    recognizeButton.dataset.testid = 'image-ocr-button'
    recognizeButton.addEventListener('click', () => void runOcr())
    toolbar.append(recognizeButton, divider())
  }
  toolbar.append(button('copy', 'copy', keyLabel('Copy', '\u2318C'), () => void copy()), button('more', 'more', 'More', toggleMenu))
  root.appendChild(menu)
  function toggleMenu() {
    menu.hidden = !menu.hidden
    // Under the button, lined up with its right edge.
    const at = buttons.get('more')!.getBoundingClientRect()
    menu.style.left = `${at.right - root.getBoundingClientRect().left}px`
  }
  // Clicking anywhere else closes the menu.
  const closeMenu = (e: PointerEvent) => {
    if (!menu.hidden && !menu.contains(e.target as Node) && !buttons.get('more')?.contains(e.target as Node)) menu.hidden = true
  }
  document.addEventListener('pointerdown', closeMenu, true)

  for (const c of COLORS) {
    const swatch = el('button', 'img-swatch')
    swatch.type = 'button'
    swatch.title = c.name
    swatch.setAttribute('aria-label', c.name)
    swatch.style.setProperty('--swatch', c.value)
    swatch.dataset.color = c.value
    swatch.addEventListener('click', () => {
      if (isMarker(tool)) colors[tool] = c.value
      refreshToolbar()
    })
    swatches.appendChild(swatch)
  }

  function setTool(next: Tool) {
    tool = next
    if (next !== 'crop') selection = null
    stage.className = `img-stage tool-${next}`
    drawOverlay()
    refreshToolbar()
  }

  function refreshToolbar() {
    for (const t of tools) buttons.get(`tool-${t.id}`)?.classList.toggle('active', tool === t.id)
    const undoButton = buttons.get('undo')
    const redoButton = buttons.get('redo')
    if (undoButton) undoButton.disabled = !edits.length
    if (redoButton) redoButton.disabled = !undone.length
    const marking = isMarker(tool)
    swatches.hidden = !marking
    for (const s of swatches.children as HTMLCollectionOf<HTMLElement>) s.classList.toggle('on', marking && colors[tool as Marker] === s.dataset.color)
    cropBar.hidden = !(tool === 'crop' && selection)
  }

  // ── Keyboard, while the tab is showing

  const isEditable = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))
  const onKey = (e: KeyboardEvent) => {
    if (!active || isEditable(e.target)) return
    const mod = isMac ? e.metaKey : e.ctrlKey
    const key = e.key.toLowerCase()
    if (mod && key === 'c' && !window.getSelection()?.toString()) {
      e.preventDefault()
      void copy()
    } else if (editable && mod && key === 'z') {
      e.preventDefault()
      if (e.shiftKey) redo()
      else undo()
    } else if (editable && mod && key === 'y' && !isMac) {
      e.preventDefault()
      redo()
    } else if (e.key === 'Escape') {
      if (!menu.hidden) menu.hidden = true
      else if (selection) cancelSelection()
      else if (tool !== 'pan') setTool('pan')
    } else if (e.key === 'Enter' && selection && !(e.target instanceof HTMLButtonElement)) {
      e.preventDefault()
      applyCrop()
    }
  }
  window.addEventListener('keydown', onKey)

  async function print() {
    if (!work) return
    const img = el('img', 'img-print')
    img.src = work.toDataURL('image/png')
    await img.decode().catch(() => {})
    await context.print(img)
  }

  // ── Open

  const ready = (async () => {
    try {
      if (!original.length) throw new Error('This picture is empty.')
      source = await decode(original, format)
      if (destroyed) return
      const first = canvas(source.width, source.height)
      context2d(first).drawImage(source, 0, 0)
      showWork(first)
      refreshToolbar()
      root.focus({ preventScroll: true })
    } catch (err) {
      stage.replaceChildren(el('div', 'img-error', err instanceof Error ? err.message : String(err)))
      toolbar.hidden = true
    }
  })()

  return {
    async getBytes() {
      await ready
      // Unchanged (or every change undone): the file as it was, not re-encoded.
      if (!edits.length || !work || !editable) return original
      return encode(work, format)
    },
    zoom(direction) {
      if (direction === 'reset') setZoom(null)
      else step(direction === 'in' ? 1 : -1)
    },
    setActive(on) {
      active = on
      if (!on) menu.hidden = true
    },
    print,
    recognizeText: context.imageOcr ? () => void runOcr() : undefined,
    destroy() {
      destroyed = true
      ocrJob?.abort()
      resize.disconnect()
      window.removeEventListener('keydown', onKey)
      document.removeEventListener('pointerdown', closeMenu, true)
      if (note) window.clearTimeout(note.timer)
      // Let go of the pixels now rather than whenever the canvases are collected.
      for (const c of [source, work]) if (c) c.width = c.height = 0
      root.remove()
    },
  }
}
