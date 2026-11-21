import {
  ArrowDown,
  ArrowUp,
  BringToFront,
  ChevronLeft,
  ChevronRight,
  ClipboardPaste,
  Copy,
  Crop,
  EyeOff,
  Group,
  Minus,
  MoveDown,
  MoveUp,
  Play,
  Plus,
  Redo2,
  Scissors,
  SendToBack,
  StickyNote,
  Trash2,
  Undo2,
  Ungroup,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { createRoot } from 'react-dom/client'
import type { EditorProps } from '../../App'
import type { Rect } from '../../components/CropOverlay'
import { CropOverlay } from '../../components/CropOverlay'
import { FindBar, Workbench, formatZoom } from '../../components/Workbench'
import type { MenuEntry } from '../../components/ui'
import { CenterState, IconButton, PopoverMenu } from '../../components/ui'
import type { Element } from '../../engine/slides/engine'
import { SlidesEngine } from '../../engine/slides/engine'
import { SlideCanvas, SLIDES_CLIPBOARD, toggleText } from '../../engine/slides/SlideCanvas'
import { SlideImage } from '../../engine/slides/SlideImage'
import { usePinchZoom } from '../../lib/pinch'
import * as platform from '../../lib/platform'
import { keysFor, withKeys } from '../../state/shortcuts'
import { registerEditor, useWorkspace } from '../../state/workspace'
import type { Restack } from './arrange'
import { Inspector } from './Inspector'
import { NotesPane } from './Notes'
import { PresentMode } from './PresentMode'
import type { PickMode } from './SlideStrip'
import { SlideStrip } from './SlideStrip'
import { NEW_SLIDE_TEMPLATES, templateLabel } from './templates'
import type { Deck } from './types'
import { parseVideoLink, videoPoster } from './video'
import { VideoFrame } from './VideoFrame'
import './pptx.css'
import { useShared } from '../../collab/useShared'

export default function PptxEditor({ tab, active }: EditorProps) {
  const [engine, setEngine] = useState<SlidesEngine | null>(null)
  const [error, setError] = useState<string | null>(null)
  useShared(tab.id, engine)
  useEffect(() => {
    let live = true
    let opened: SlidesEngine | null = null
    SlidesEngine.open(tab.bytes)
      .then((e) => {
        if (!live) return e.dispose()
        opened = e
        if (import.meta.env.DEV || import.meta.env.VITE_TEST_HOOKS) Object.assign(window, { __slides: e })
        setEngine(e)
      })
      .catch((e) => setError(String(e?.message ?? e)))
    return () => {
      live = false
      opened?.dispose()
    }
  }, [tab.bytes])
  if (error) {
    return (
      <CenterState title="Couldn’t open this presentation">
        <p className="muted">{error}</p>
      </CenterState>
    )
  }
  if (!engine) return <CenterState spinner />
  return <DeckEditor tab={tab} active={active} engine={engine} />
}

function DeckEditor({ tab, active, engine }: EditorProps & { engine: SlidesEngine }) {
  useSyncExternalStore(engine.subscribe, engine.getTick)
  const slides = engine.slides
  const st = engine.state
  const activeIndex = st.active
  const markDirty = useWorkspace((s) => s.markDirty)
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null)
  const [presenting, setPresenting] = useState<number | null>(null)
  const [findOpen, setFindOpen] = useState<false | 'find' | 'replace'>(false)
  const [matches, setMatches] = useState<{ slide: number; id: string }[]>([])
  const [matchIndex, setMatchIndex] = useState(0)
  const [scale, setScale] = useState(1)
  const scaleRef = useRef(scale)
  scaleRef.current = scale
  const stageRef = useRef<HTMLDivElement>(null)
  const viewportRef = useRef<HTMLDivElement | null>(null)
  const wrapperRef = useRef<HTMLDivElement | null>(null)
  const pixelScale = useRef(1)
  /** Frames copied here, for the menu's Paste. */
  const copied = useRef<string | null>(null)

  // ── Unsaved changes follow the engine's history.
  useEffect(() => {
    // Notes and transitions can change without invalidating a slide's drawing revision.
    let revision = JSON.stringify(engine.slides)
    let dirty = engine.state.dirty
    return engine.subscribe(() => {
      const next = JSON.stringify(engine.slides)
      if (next !== revision || engine.state.dirty !== dirty) {
        revision = next
        dirty = engine.state.dirty
        markDirty(tab.id, dirty)
      }
    })
  }, [engine, markDirty, tab.id])

  // ── Slides picked in the sidebar, by id so they survive moves. The active slide is always picked.
  const [picked, setPicked] = useState<string[]>([])
  const pickAnchor = useRef(0)
  const selectedSlides = useMemo(() => {
    const ids = new Set(picked)
    const out = slides.flatMap((s, i) => (ids.has(s.id) ? [i] : []))
    return out.includes(activeIndex) ? out : [activeIndex]
  }, [picked, slides, activeIndex])
  const go = (index: number) => engine.run({ cmd: 'setActive', index })
  const pickSlide = (index: number, mode: PickMode) => {
    const id = slides[index]?.id
    if (!id) return
    if (mode === 'range') {
      const [a, b] = [Math.min(pickAnchor.current, index), Math.max(pickAnchor.current, index)]
      setPicked(slides.slice(a, b + 1).map((s) => s.id))
    } else if (mode === 'toggle' && selectedSlides.includes(index)) {
      if (selectedSlides.length === 1) return
      const rest = selectedSlides.filter((i) => i !== index)
      setPicked(rest.map((i) => slides[i].id))
      pickAnchor.current = rest[0]
      // Put down the active slide, and the nearest picked one takes over.
      if (index === activeIndex) go(rest.reduce((best, i) => (Math.abs(i - index) < Math.abs(best - index) ? i : best)))
      return
    } else {
      setPicked(mode === 'toggle' ? [...selectedSlides.map((i) => slides[i].id), id] : [id])
      pickAnchor.current = index
    }
    go(index)
  }
  const deleteSlides = (indexes: readonly number[]) => {
    if (!indexes.length) return
    setPicked([])
    engine.run({ cmd: 'deleteSlides', indexes: [...indexes] })
  }

  const elements = engine.elements
  const selectedIds = st.frames
  const selectedElement = selectedIds.length === 1 ? (elements.find((e) => e.id === selectedIds[0]) ?? null) : null

  // ── A video link playing over its poster while editing. It plays while its frame, with that link, is the one picked.
  const [playing, setPlaying] = useState<{ slide: string; id: string; url: string; embed: string } | null>(null)
  const playVideo = (id: string) => {
    const url = engine.elements.find((x) => x.id === id)?.link?.url
    const video = url ? parseVideoLink(url) : null
    const slide = engine.slides[engine.state.active]?.id
    if (!url || !video || video.provider === 'other' || !slide || engine.state.frames.join() !== id) return false
    setPlaying({ slide, id, url, embed: video.embedUrl })
    return true
  }
  const shownVideo =
    playing && presenting === null && slides[activeIndex]?.id === playing.slide && selectedElement?.id === playing.id && selectedElement.link?.url === playing.url ? playing : null
  useEffect(() => {
    if (playing && !shownVideo) setPlaying(null)
  }, [playing, shownVideo])

  // ── Cropping the selected picture, on the slide.
  const [crop, setCrop] = useState<{ el: Element; src: string; frame: Rect; image: Rect } | null>(null)
  const croppable = (el: Element | null, count: number) => !!el && count === 1 && el.type === 'image' && !el.rotation && !el.flipHorizontal && !el.flipVertical
  const startCrop = () => {
    const el = selectedElement
    const slide = wrapperRef.current?.getBoundingClientRect()
    if (!el || !slide || !el.media || !croppable(el, selectedIds.length)) return
    const src = engine.media.url(el.media)
    if (!src) return
    const k = pixelScale.current
    const frame = { x: slide.left + el.x * k, y: slide.top + el.y * k, width: el.width * k, height: el.height * k }
    const [l, t, r, b] = el.crop ?? [0, 0, 0, 0]
    const iw = frame.width / Math.max(0.01, 1 - l - r)
    const ih = frame.height / Math.max(0.01, 1 - t - b)
    setCrop({ el, src, frame, image: { x: frame.x - l * iw, y: frame.y - t * ih, width: iw, height: ih } })
  }
  const endCrop = (frame?: Rect, image?: Rect) => {
    setCrop(null)
    if (!crop || !frame || !image) return
    const { el, frame: start } = crop
    // From screen pixels to the slide's.
    const k = start.width / el.width
    const shares = [(frame.x - image.x) / image.width, (frame.y - image.y) / image.height, (image.x + image.width - frame.x - frame.width) / image.width, (image.y + image.height - frame.y - frame.height) / image.height]
    const before = el.crop ?? [0, 0, 0, 0]
    if (shares.every((v, i) => Math.abs(v - before[i]) < 1e-4)) return
    engine.run({
      cmd: 'crop',
      id: Number(el.id),
      left: shares[0],
      top: shares[1],
      right: shares[2],
      bottom: shares[3],
      x: el.x + (frame.x - start.x) / k,
      y: el.y + (frame.y - start.y) / k,
      w: frame.width / k,
      h: frame.height / k,
    })
  }
  const deck: Deck = { engine, slides, activeIndex, elements, selectedSlides, selectedIds, selectedElement }
  const deckRef = useRef(deck)
  deckRef.current = deck

  // Trackpad pinch / ⌘-scroll zoom with a live preview, committed when the gesture ends.
  usePinchZoom(stageRef, {
    getZoom: () => scaleRef.current,
    setZoom: (z) => setScale(z),
    commit: (z, anchor) => zoomAround(() => setScale(z), anchor),
    target: () => wrapperRef.current,
    min: 0.25,
    max: 4,
  })

  /**
   * Zoom while keeping one point of the slide where it is on screen: the pointer for a pinch,
   * the middle of the view for buttons and keys.
   */
  const zoomAround = (apply: () => void, at?: { x: number; y: number }) => {
    const viewport = viewportRef.current
    const wrapper = wrapperRef.current
    if (!viewport || !wrapper) return apply()
    const view = viewport.getBoundingClientRect()
    const before = wrapper.getBoundingClientRect()
    const point = at ?? { x: view.left + view.width / 2, y: view.top + view.height / 2 }
    const fx = (point.x - before.left) / before.width
    const fy = (point.y - before.top) / before.height
    apply()
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        const after = wrapper.getBoundingClientRect()
        viewport.scrollLeft += after.left + fx * after.width - point.x
        viewport.scrollTop += after.top + fy * after.height - point.y
      }),
    )
  }
  const zoomStep = (dir: 'in' | 'out' | 'fit' | 'reset') => {
    if (dir === 'fit' || dir === 'reset') return setScale(1)
    const s = scaleRef.current
    const next = dir === 'in' ? ZOOM_STEPS.find((z) => z > s + 0.001) : [...ZOOM_STEPS].reverse().find((z) => z < s - 0.001)
    if (next) zoomAround(() => setScale(next))
  }

  // ── Inserting things
  const addImages = async (files: readonly File[]) => {
    for (const file of files) {
      const { bytes, type, width, height } = await loadImage(file, engine)
      engine.doc.insertImage(bytes, type, width, height, file.name)
      engine.refresh()
    }
  }
  const insertImage = async () => addImages(await platform.pickImages({ multiple: true }))

  // Pasting an image anywhere in the deck (not in a text field) puts it on the slide.
  const addImagesRef = useRef(addImages)
  addImagesRef.current = addImages
  useEffect(() => {
    if (!active) return
    const onPaste = (e: ClipboardEvent) => {
      if (isTextTarget(e.target) || document.querySelector('.present')) return
      const files = Array.from(e.clipboardData?.files ?? []).filter((f) => f.type.startsWith('image/'))
      if (!files.length) return
      e.preventDefault()
      e.stopImmediatePropagation()
      void addImagesRef.current(files)
    }
    window.addEventListener('paste', onPaste, true)
    return () => window.removeEventListener('paste', onPaste, true)
  }, [active])

  // ── Arranging objects
  const arrange = (how: Restack) => {
    if (deckRef.current.selectedIds.length) engine.run({ cmd: 'restack', how })
  }
  const canArrange = (how: Restack) => st.restack.includes(how)

  const insertTable = (rows: number, cols: number) => engine.run({ cmd: 'insertTable', rows, cols })

  const insertVideo = async (raw: string) => {
    const link = parseVideoLink(raw)
    if (!link) {
      await platform.showError('That doesn’t look like a video link. Paste a YouTube or Vimeo address.')
      return false
    }
    const poster = await videoPoster(link)
    const blob = await (await fetch(poster)).blob()
    const bytes = new Uint8Array(await blob.arrayBuffer())
    const id = engine.doc.insertImage(bytes, blob.type || 'image/png', 1280, 720, `Video: ${link.url}`)
    engine.refresh()
    engine.run({ cmd: 'link', id: Number(id), url: link.url })
    return true
  }

  const newSlideMenu = (anchor: { x: number; y: number }) => {
    const after = deckRef.current.activeIndex
    const items: MenuEntry[] = NEW_SLIDE_TEMPLATES.map((id) => ({ label: templateLabel(id), onSelect: () => engine.run({ cmd: 'addTemplate', after, template: id }) }))
    // A deck made in PowerPoint brings its own layouts; one generated by a script has a single unnamed one.
    if (engine.layouts.length > 1) {
      engine.layouts.forEach((l, i) => items.push({ label: l.name, separatorBefore: i === 0, onSelect: () => engine.run({ cmd: 'addSlide', after, layout: l.path }) }))
    }
    setMenu({ ...anchor, items })
  }

  const slideMenu = (index: number, x: number, y: number) => {
    const group = selectedSlides.includes(index) && selectedSlides.length > 1 ? selectedSlides : null
    if (group) {
      const allHidden = group.every((i) => slides[i]?.hidden)
      setMenu({
        x,
        y,
        items: [
          { label: `Duplicate ${group.length} slides`, icon: <Copy size={14} />, onSelect: () => engine.run({ cmd: 'duplicateSlides', indexes: group }) },
          { label: allHidden ? 'Show in slideshow' : 'Hide in slideshow', icon: <EyeOff size={14} />, onSelect: () => engine.run({ cmd: 'toggleHidden', indexes: group }) },
          { label: `Delete ${group.length} slides`, icon: <Trash2 size={14} />, danger: true, separatorBefore: true, onSelect: () => deleteSlides(group) },
        ],
      })
      return
    }
    setMenu({
      x,
      y,
      items: [
        {
          label: 'New slide after',
          icon: <Plus size={14} />,
          onSelect: () => {
            // New slides go after the current one.
            go(index)
            newSlideMenu({ x, y })
          },
        },
        { label: 'Duplicate slide', icon: <Copy size={14} />, onSelect: () => engine.run({ cmd: 'duplicateSlides', indexes: [index] }) },
        { label: slides[index]?.hidden ? 'Show in slideshow' : 'Hide in slideshow', icon: <EyeOff size={14} />, onSelect: () => engine.run({ cmd: 'toggleHidden', indexes: [index] }) },
        ...(index > 0 ? [{ label: 'Move up', icon: <ArrowUp size={14} />, onSelect: () => engine.run({ cmd: 'moveSlide', from: index, to: index - 1 }) }] : []),
        ...(index < slides.length - 1 ? [{ label: 'Move down', icon: <ArrowDown size={14} />, onSelect: () => engine.run({ cmd: 'moveSlide', from: index, to: index + 1 }) }] : []),
        { label: 'Delete slide', icon: <Trash2 size={14} />, danger: true, separatorBefore: true, onSelect: () => deleteSlides([index]) },
      ],
    })
  }

  // ── Right-click on the slide: the canvas picks what was clicked; the menu is ours.
  const objectMenu = (x: number, y: number) => {
    const ids = engine.state.frames
    const items: MenuEntry[] = []
    const copy = () => {
      const frames = engine.doc.copyFrames()
      if (frames === '[]') return false
      copied.current = frames
      return true
    }
    if (ids.length) {
      items.push(
        {
          label: 'Cut',
          icon: <Scissors size={14} />,
          shortcut: '⌘X',
          onSelect: () => {
            if (copy()) engine.run({ cmd: 'delete' })
          },
        },
        { label: 'Copy', icon: <Copy size={14} />, shortcut: '⌘C', onSelect: copy },
      )
    }
    if (copied.current) items.push({ label: 'Paste', icon: <ClipboardPaste size={14} />, shortcut: '⌘V', onSelect: () => engine.run({ cmd: 'pasteFrames', json: copied.current! }) })
    if (!ids.length) {
      if (items.length) setMenu({ x, y, items })
      return
    }
    const el = ids.length === 1 ? engine.elements.find((e) => e.id === ids[0]) ?? null : null
    items.push({ label: 'Duplicate', icon: <Copy size={14} />, shortcut: keysFor('pptx.duplicate'), onSelect: () => engine.run({ cmd: 'duplicate' }) })
    if (croppable(el, ids.length)) items.push({ label: 'Crop', icon: <Crop size={14} />, onSelect: startCrop })
    if (ids.length > 1) items.push({ label: 'Group', icon: <Group size={14} />, shortcut: keysFor('pptx.group'), separatorBefore: true, onSelect: () => engine.run({ cmd: 'group' }) })
    else if (el?.type === 'group') items.push({ label: 'Ungroup', icon: <Ungroup size={14} />, shortcut: keysFor('pptx.ungroup'), separatorBefore: true, onSelect: () => engine.run({ cmd: 'ungroup' }) })
    const can = engine.state.restack
    const layer = (label: string, how: Restack, icon: React.ReactNode, first = false): MenuEntry => ({
      label,
      icon,
      shortcut: keysFor(`pptx.${how}`),
      separatorBefore: first,
      disabled: !can.includes(how),
      onSelect: () => arrange(how),
    })
    items.push(
      layer('Bring to Front', 'front', <BringToFront size={14} />, true),
      layer('Bring Forward', 'forward', <MoveUp size={14} />),
      layer('Send Backward', 'backward', <MoveDown size={14} />),
      layer('Send to Back', 'back', <SendToBack size={14} />),
      { label: 'Delete', icon: <Trash2 size={14} />, danger: true, separatorBefore: true, onSelect: () => engine.run({ cmd: 'delete' }) },
    )
    setMenu({ x, y, items })
  }

  // Shortcut actions (see state/shortcuts.ts); they read the deck through refs.
  const arrangeKey = (how: Restack) => {
    if (engine.state.editing || !deckRef.current.selectedIds.length) return false
    arrange(how)
  }
  const textKey = (key: 'b' | 'i' | 'u') => {
    if (!engine.state.editing && !engine.state.frames.length) return false
    toggleText(engine, key)
  }

  // ── Speaker notes
  const [notesOpen, setNotesOpen] = useState(true)

  // ── Find across every slide's text.
  const findQuery = useRef('')
  const runFind = (query: string, from?: { index: number; skip: string }) => {
    findQuery.current = query.trim()
    const found = findQuery.current ? engine.run<{ slide: number; id: string }[]>({ cmd: 'find', query: findQuery.current }) : []
    // After a replace, carry on from the same place, past the object just changed.
    let index = 0
    if (from) {
      index = Math.min(from.index, found.length)
      if (found[index]?.id === from.skip) index++
      if (index >= found.length) index = 0
    }
    setMatches(found)
    setMatchIndex(index)
    if (found[index]) goToMatch(found[index])
  }
  const replaceCurrent = (by: string) => {
    const m = matches[matchIndex]
    if (!m || !findQuery.current) return
    engine.run({ cmd: 'replaceMatch', index: matchIndex, text: by })
    runFind(findQuery.current, { index: matchIndex, skip: m.id })
  }
  const replaceAll = (by: string) => {
    if (!matches.length || !findQuery.current) return
    engine.run({ cmd: 'replaceAll', text: by })
    runFind(findQuery.current, { index: 0, skip: '' })
  }
  const goToMatch = (m: { slide: number; id: string }) => {
    go(m.slide)
    engine.run({ cmd: 'selectFrames', ids: [Number(m.id)] })
  }
  const stepFind = (delta: number) => {
    if (!matches.length) return
    const next = (matchIndex + delta + matches.length) % matches.length
    setMatchIndex(next)
    goToMatch(matches[next])
  }

  // ── Shell integration
  useEffect(
    () =>
      registerEditor(tab.id, {
        getRevision: () => JSON.stringify(engine.slides),
        getBytes: async () => engine.save(),
        markSaved: () => {
          engine.run({ cmd: 'markSaved' })
          markDirty(tab.id, false)
        },
        find: () => setFindOpen('find'),
        replace: () => setFindOpen('replace'),
        zoom: (dir) => zoomStep(dir),
        actions: {
          'text.bold': () => textKey('b'),
          'text.italic': () => textKey('i'),
          'text.underline': () => textKey('u'),
          'pptx.new-slide': () => engine.run({ cmd: 'addSlide', after: deckRef.current.activeIndex, layout: null }),
          'pptx.duplicate': () => (deckRef.current.selectedIds.length ? engine.run({ cmd: 'duplicate' }) : engine.run({ cmd: 'duplicateSlides', indexes: [...deckRef.current.selectedSlides] })),
          'pptx.group': () => deckRef.current.selectedIds.length > 1 && (engine.run({ cmd: 'group' }), true),
          'pptx.ungroup': () => deckRef.current.selectedElement?.type === 'group' && (engine.run({ cmd: 'ungroup' }), true),
          'pptx.front': () => arrangeKey('front'),
          'pptx.forward': () => arrangeKey('forward'),
          'pptx.backward': () => arrangeKey('backward'),
          'pptx.back': () => arrangeKey('back'),
          'pptx.notes': () => setNotesOpen((open) => !open),
          'pptx.present': () => setPresenting(deckRef.current.activeIndex),
          'pptx.present-start': () => setPresenting(0),
        },
        print: () => printSlides(engine),
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tab.id, markDirty, engine],
  )

  const left = (
    <SlideStrip
      engine={engine}
      slides={slides}
      activeIndex={activeIndex}
      selected={selectedSlides}
      onSelect={pickSlide}
      onMove={(from, to) => engine.run({ cmd: 'moveSlide', from, to })}
      onMenu={slideMenu}
      onDelete={() => deleteSlides(selectedSlides)}
    />
  )

  return (
    <Workbench
      tab={tab}
      left={left}
      leftFooter={slides.length ? <span>{slides.length === 1 ? '1 slide' : `${slides.length} slides`}</span> : undefined}
      right={
        <Inspector
          deck={deck}
          onInsertImage={() => void insertImage()}
          onInsertTable={insertTable}
          onInsertVideo={insertVideo}
          onNewSlide={newSlideMenu}
          onPresent={(fromStart) => setPresenting(fromStart ? 0 : activeIndex)}
          onDeleteSlides={deleteSlides}
          onArrange={arrange}
          canArrange={canArrange}
          onCrop={croppable(selectedElement, selectedIds.length) ? startCrop : null}
          onPlayVideo={(id) => void playVideo(id)}
        />
      }
    >
      <div className={`deck-stage ${crop ? 'cropping' : ''} ${scale > 1.01 ? 'slide-enlarged' : ''}`} ref={stageRef}>
        <SlideCanvas engine={engine} scale={scale} padding={40} active={active} onContextMenu={objectMenu} viewportRef={viewportRef} wrapperRef={wrapperRef} onPixelScale={(s) => (pixelScale.current = s)}
          onOpenFrame={playVideo}
          frameOverlay={shownVideo && { id: shownVideo.id, node: <VideoFrame embed={shownVideo.embed} style={{}} onClose={() => setPlaying(null)} /> }}
        />
      </div>
      {notesOpen && slides[activeIndex] && <NotesPane key={slides[activeIndex].id} slide={slides[activeIndex]} onChange={(text) => engine.run({ cmd: 'notes', index: activeIndex, text })} />}
      <FindBar
        open={!!findOpen}
        replacing={findOpen === 'replace'}
        onClose={() => setFindOpen(false)}
        onQuery={(q) => runFind(q)}
        onReplace={replaceCurrent}
        onReplaceAll={replaceAll}
        onNext={() => stepFind(1)}
        onPrev={() => stepFind(-1)}
        count={matches.length}
        current={matchIndex}
        placeholder="Find in slides"
      />
      {slides.length > 0 && (
        <div className="canvas-bar" style={notesOpen ? { bottom: 'calc(var(--notes-h, 120px) + 12px)' } : undefined}>
          <IconButton label="Undo (⌘Z)" size="sm" disabled={!st.canUndo} onClick={() => engine.run({ cmd: 'undo' })}>
            <Undo2 size={14} />
          </IconButton>
          <IconButton label="Redo (⇧⌘Z)" size="sm" disabled={!st.canRedo} onClick={() => engine.run({ cmd: 'redo' })}>
            <Redo2 size={14} />
          </IconButton>
          <span className="sep" />
          <IconButton label="Previous slide" size="sm" disabled={activeIndex === 0} onClick={() => go(activeIndex - 1)}>
            <ChevronLeft size={14} />
          </IconButton>
          <span className="readout">
            {activeIndex + 1} / {slides.length}
          </span>
          <IconButton label="Next slide" size="sm" disabled={activeIndex >= slides.length - 1} onClick={() => go(activeIndex + 1)}>
            <ChevronRight size={14} />
          </IconButton>
          <span className="sep" />
          <IconButton label="Zoom out (⌘−)" size="sm" onClick={() => zoomStep('out')} disabled={scale <= ZOOM_STEPS[0] + 0.001}>
            <Minus size={14} />
          </IconButton>
          <IconButton label="Fit to window (⌘0)" size="sm" className="readout" aria-label={formatZoom(scale)} onClick={() => zoomStep('fit')}>
            {formatZoom(scale)}
          </IconButton>
          <IconButton label="Zoom in (⌘+)" size="sm" onClick={() => zoomStep('in')} disabled={scale >= ZOOM_STEPS[ZOOM_STEPS.length - 1] - 0.001}>
            <Plus size={14} />
          </IconButton>
          <span className="sep" />
          <IconButton label={withKeys(notesOpen ? 'Hide speaker notes' : 'Show speaker notes', 'pptx.notes')} size="sm" active={notesOpen} onClick={() => setNotesOpen(!notesOpen)}>
            <StickyNote size={14} />
          </IconButton>
          <IconButton label={withKeys('Present', 'pptx.present')} size="sm" onClick={() => setPresenting(activeIndex)}>
            <Play size={14} />
          </IconButton>
        </div>
      )}
      {menu && <PopoverMenu anchor={menu} items={menu.items} onClose={() => setMenu(null)} />}
      {crop && <CropOverlay src={crop.src} frame={crop.frame} image={crop.image} onDone={endCrop} onCancel={() => endCrop()} />}
      {presenting !== null && (
        <PresentMode
          engine={engine}
          startIndex={presenting}
          onExit={(last) => {
            setPresenting(null)
            go(last)
          }}
        />
      )}
    </Workbench>
  )
}

/** Zoom levels for the buttons and keys, relative to fitting the slide in the window. */
const ZOOM_STEPS = [0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4]

/** Typing somewhere: a text box, a field, or text being edited on the slide. */
function isTextTarget(target: EventTarget | null) {
  const el = target instanceof HTMLElement ? target : null
  return !!el && !el.closest('[data-engine-input]') && (el.isContentEditable || !!el.closest('input, textarea, select'))
}

/**
 * Read an image for the slide. One far bigger than the slide could ever show (a phone photo,
 * a Retina screenshot) is scaled down to twice the slide's size, which keeps the slide, its
 * thumbnail and the saved file quick. Smaller ones are kept byte for byte.
 */
async function loadImage(file: File, deck: { width: number; height: number }) {
  const bitmap = await createImageBitmap(file).catch(() => null)
  const own = async () => ({ bytes: new Uint8Array(await file.arrayBuffer()), type: file.type || 'image/png' })
  if (!bitmap) return { ...(await own()), width: 400, height: 300 }
  const { width, height } = bitmap
  const scale = Math.min(1, (deck.width * 2) / width, (deck.height * 2) / height)
  if (scale > 0.75 || file.type === 'image/gif' || file.type === 'image/svg+xml') {
    bitmap.close()
    return { ...(await own()), width, height }
  }
  const out = document.createElement('canvas')
  out.width = Math.round(width * scale)
  out.height = Math.round(height * scale)
  out.getContext('2d')!.drawImage(bitmap, 0, 0, out.width, out.height)
  bitmap.close()
  // Photos stay JPEG; anything that may have transparency stays PNG.
  const type = file.type === 'image/jpeg' ? 'image/jpeg' : 'image/png'
  const blob = await new Promise<Blob>((resolve) => out.toBlob((b) => resolve(b!), type, 0.9))
  return { bytes: new Uint8Array(await blob.arrayBuffer()), type, width: out.width, height: out.height }
}

/** Render every visible slide into the print root at landscape page size, then print. */
async function printSlides(engine: SlidesEngine) {
  const root = document.getElementById('print-root')!
  root.innerHTML = '<style>@page { size: landscape; margin: 0.4in; }</style>'
  const host = document.createElement('div')
  root.appendChild(host)
  const width = 10 * 96 // fits Letter and A4 landscape inside the margins
  const reactRoot = createRoot(host)
  reactRoot.render(
    <>
      {engine.slides.map((s, i) =>
        s.hidden ? null : (
          <div key={s.id} className="print-page pptx-print-page">
            <SlideImage engine={engine} index={i} revision={s.revision} width={width} lazy={false} />
          </div>
        ),
      )}
    </>,
  )
  await new Promise((r) => setTimeout(r, 400))
  await Promise.all(Array.from(root.querySelectorAll('img')).map((img) => img.decode().catch(() => {})))
  window.print()
  setTimeout(() => reactRoot.unmount(), 60_000)
}

export { SLIDES_CLIPBOARD }
