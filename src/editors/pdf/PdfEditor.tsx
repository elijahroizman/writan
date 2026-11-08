import type { PdfAnnotationObject } from '@embedpdf/models'
import { PdfAnnotationSubtype, PdfBlendMode, PdfStandardFont, PdfTextAlignment, PdfVerticalAlignment } from '@embedpdf/models'
import {
  ArrowDownToLine,
  ArrowUpToLine,
  Copy,
  FileOutput,
  FilePlus2,
  Highlighter,
  ImagePlus,
  Minus,
  MousePointer2,
  MoveHorizontal,
  PenLine,
  Plus,
  Redo2,
  RotateCcw,
  RotateCw,
  Scaling,
  Signature,
  SquareDashed,
  StepForward,
  Stamp,
  Trash2,
  Type,
  Undo2,
} from 'lucide-react'
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { EditorProps } from '../../App'
import { FindBar, InspectorHeader, Workbench, formatZoom } from '../../components/Workbench'
import type { MenuEntry } from '../../components/ui'
import { Button, CenterState, IconButton, PopoverMenu, Section, Segmented, TextField, ToolButton } from '../../components/ui'
import * as platform from '../../lib/platform'
import { registerEditor, useWorkspace } from '../../state/workspace'
import { isCancelled, PdfEngine, Priority } from './engine/client'
import type { Annot, Box, DocMeta, OutlineItem, PageInfo } from './engine/protocol'
import { DocStore, drawBitmap } from './engine/store'
import { History } from './history'
import { Outline, Thumbnails } from './Sidebar'
import type { SavedSignature, SignChoice } from './signatures'
import { markPng, PlaceLayer, pngBytes, SignaturePad, SignPanel, useSignatures } from './signatures'
import type { AnnotationActions, Stroke } from './viewer/layers'
import { AnnotationLayer, clearTextSelection, DrawLayer, InkPreview, SearchLayer, selectedCharacters, TextDraft, TextLayer, TextToolLayer } from './viewer/layers'
import type { PageSlot, ViewerHandle } from './viewer/Viewer'
import { Viewer } from './viewer/Viewer'
import './pdf.css'

const HIGHLIGHT_COLORS = ['#FFFF98', '#53FFBC', '#80EBFF', '#FFCBE6', '#FF4F5F']
const INK_COLORS = ['#000000', '#E5484D', '#2F6FED', '#1F9D55', '#F5A524']
/** Drawing pauses longer than this finish the drawing (strokes before it become one annotation). */
const INK_IDLE_MS = 1200
/** Background tabs let go of their page drawings after this long. */
const SLEEP_AFTER_MS = 30_000

type ToolId = 'select' | 'highlight' | 'text' | 'draw' | 'image' | 'sign'
const TOOLS: { id: ToolId; label: string; icon: React.ReactNode; hint: string }[] = [
  { id: 'select', label: 'Select', icon: <MousePointer2 size={15} />, hint: 'Select text and annotations' },
  { id: 'highlight', label: 'Highlight', icon: <Highlighter size={15} />, hint: 'Select text to highlight it' },
  { id: 'text', label: 'Text', icon: <Type size={15} />, hint: 'Click on a page to add text' },
  { id: 'draw', label: 'Draw', icon: <PenLine size={15} />, hint: 'Draw freehand on a page, or sign your name' },
  { id: 'image', label: 'Image', icon: <ImagePlus size={15} />, hint: 'Place an image on a page' },
  { id: 'sign', label: 'Sign', icon: <Signature size={15} />, hint: 'Pick a signature, initials or mark, then click where it goes' },
]

/** Default widths, in points, of a placed signature and initials. */
const SIGNATURE_WIDTH = { signature: 150, initials: 54 }
const MARK_SIZE = 14

interface FindState {
  query: string
  /** Matches per page key; each match is one or more boxes. */
  hits: Map<number, Box[][]>
  done: boolean
}

const toRect = (b: Box) => ({ origin: { x: b.x, y: b.y }, size: { width: b.w, height: b.h } })
const union = (boxes: Box[]): Box => {
  const x = Math.min(...boxes.map((b) => b.x))
  const y = Math.min(...boxes.map((b) => b.y))
  return { x, y, w: Math.max(...boxes.map((b) => b.x + b.w)) - x, h: Math.max(...boxes.map((b) => b.y + b.h)) - y }
}
const isEditable = (el: EventTarget | null) => el instanceof HTMLElement && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName))

export default function PdfEditor({ tab, active, visible = false }: EditorProps) {
  const viewer = useRef<ViewerHandle>(null)
  const storeRef = useRef<DocStore | null>(null)
  const [store, setStore] = useState<DocStore | null>(null)
  const [pages, setPages] = useState<PageInfo[]>([])
  const [meta, setMeta] = useState<DocMeta>({})
  const [hasForm, setHasForm] = useState(false)
  const [pad, setPad] = useState<SavedSignature['kind'] | null>(null)
  const [showFields, setShowFields] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [password, setPassword] = useState<{ retry: boolean } | null>(null)
  const [page, setPage] = useState(0)
  const [zoom, setZoom] = useState(1)
  const [tool, setTool] = useState<ToolId>('select')
  const [highlightColor, setHighlightColor] = useState(HIGHLIGHT_COLORS[0])
  const [inkColor, setInkColor] = useState(INK_COLORS[0])
  const [inkThickness, setInkThickness] = useState(2)
  const [fontSize, setFontSize] = useState(12)
  const [sideTab, setSideTab] = useState<'pages' | 'outline'>('pages')
  const [findOpen, setFindOpen] = useState(false)
  const [find, setFind] = useState<FindState | null>(null)
  const [findIndex, setFindIndex] = useState(0)
  const [menu, setMenu] = useState<{ x: number; y: number; index: number } | null>(null)
  const [selected, setSelected] = useState<{ key: number; id: string } | null>(null)
  const [ink, setInk] = useState<{ key: number; strokes: Stroke[]; committing?: boolean } | null>(null)
  const [draft, setDraft] = useState<{ key: number; x: number; y: number } | null>(null)
  // For autosave's busy(): what's on screen, so updated once React commits it.
  const draftRef = useRef(draft)
  useLayoutEffect(() => {
    draftRef.current = draft
  }, [draft])
  const [awake, setAwake] = useState(true)
  const [, setHistoryVersion] = useState(0)
  const history = useRef(new History())
  const fieldsDirty = useRef(false)
  const passwordRef = useRef('')
  /** The document as last saved (or as opened), for reopening a suspended engine. */
  const cleanBytes = useRef<Uint8Array | null>(null)
  const lastWritten = useRef<Uint8Array | null>(null)
  const markDirty = useWorkspace((s) => s.markDirty)

  const refreshDirty = useCallback(() => {
    markDirty(tab.id, history.current.dirty || fieldsDirty.current)
    setHistoryVersion((v) => v + 1)
  }, [markDirty, tab.id])

  // ── Engine lifecycle: one PDFium worker per tab, gone when the tab closes.
  const openWith = useCallback(
    async (pw: string) => {
      const s = storeRef.current
      if (!s) return
      setError(null)
      try {
        const bytes = tab.bytes ?? new Uint8Array()
        const result = await s.engine.call('open', [bytes.slice().buffer, pw])
        if (result.pages.length === 0) throw new Error('This PDF has no pages.')
        passwordRef.current = pw
        setPassword(null)
        s.setPages(result.pages)
        setMeta(result.meta)
        setHasForm(result.hasForm)
        setStore(s)
      } catch (err) {
        if (isCancelled(err)) return
        const code = (err as { code?: number }).code
        if (code === 4) setPassword({ retry: pw !== '' })
        else setError(code === 3 || code === 2 ? 'The file is damaged or isn’t a PDF.' : (err as Error).message)
      }
    },
    [tab.bytes],
  )

  useEffect(() => {
    let s: DocStore | null = null
    let cancelled = false
    void PdfEngine.create().then((engine) => {
      if (cancelled) return engine.destroy()
      s = new DocStore(engine)
      storeRef.current = s
      void openWith('')
    })
    return () => {
      cancelled = true
      s?.destroy()
      storeRef.current = null
    }
  }, [openWith])

  useEffect(() => {
    if (!store) return
    setPages(store.pages)
    return store.subscribe(() => setPages(store.pages))
  }, [store])

  // ── Everything that changes the document goes through here, one at a time.
  const queue = useRef(Promise.resolve())
  const [busy, setBusy] = useState(false)
  const run = useCallback((fn: () => Promise<void>) => {
    const next = queue.current.then(async () => {
      setBusy(true)
      try {
        await fn()
      } catch (err) {
        if (!isCancelled(err)) await platform.showError(err instanceof Error ? err.message : String(err))
      } finally {
        setBusy(false)
      }
    })
    queue.current = next
    return next
  }, [])

  /** Do something, remember how to undo it, and mark the tab edited. */
  const perform = useCallback(
    (doIt: () => Promise<void>, undo: () => Promise<void>, redo: () => Promise<void> = doIt) =>
      run(async () => {
        await doIt()
        history.current.push({ undo, redo })
        refreshDirty()
      }),
    [run, refreshDirty],
  )

  const call = useCallback(<T,>(fn: (s: DocStore) => Promise<T>) => fn(storeRef.current!), [])
  const indexOfKey = (key: number) => storeRef.current?.pages.findIndex((p) => p.key === key) ?? -1
  const touch = (key: number) => storeRef.current?.touch(indexOfKey(key))
  const setDocPages = (p: PageInfo[]) => storeRef.current!.setPages(p)

  const undo = useCallback(() => run(async () => void (await history.current.undo(), refreshDirty())), [run, refreshDirty])
  const redo = useCallback(() => run(async () => void (await history.current.redo(), refreshDirty())), [run, refreshDirty])

  // ── Page operations: in place in PDFium, so they're instant even on big files.
  const keyAt = (index: number) => storeRef.current!.pages[index].key
  const goToKey = (key: number) => requestAnimationFrame(() => viewer.current?.goTo(Math.max(0, indexOfKey(key))))

  const rotatePage = (index: number, turns: number) => {
    const key = keyAt(index)
    const rotate = (t: number) => async () => {
      const pages = await call((s) => s.engine.call('rotate', [[key], t]))
      viewer.current?.hold(indexOfKey(key))
      setDocPages(pages)
    }
    void perform(rotate(turns), rotate(-turns))
  }

  const movePage = (from: number, to: number) => {
    if (from === to || to < 0 || to >= pages.length) return
    const key = keyAt(from)
    const move = (at: number) => async () => {
      setDocPages(await call((s) => s.engine.call('move', [key, at])))
      goToKey(key)
    }
    void perform(move(to), move(from))
  }

  /** Add pages with `add` (which returns the new page list and where the new pages are); undo deletes them. */
  const addPages = (add: () => Promise<{ pages: PageInfo[]; at: number; count: number }>) => {
    let added: number[] = []
    const doIt = async () => {
      const r = await add()
      setDocPages(r.pages)
      added = r.pages.slice(r.at, r.at + r.count).map((p) => p.key)
      goToKey(added[0])
    }
    void perform(doIt, async () => setDocPages((await call((s) => s.engine.call('deletePages', [added]))).pages))
  }

  const duplicatePage = (index: number) => {
    const key = keyAt(index)
    addPages(async () => {
      const p = await call((s) => s.engine.call('duplicate', [key]))
      return { pages: p, at: p.findIndex((x) => x.key === key) + 1, count: 1 }
    })
  }

  const insertBlank = (afterIndex: number) => {
    const key = keyAt(Math.max(0, afterIndex))
    addPages(async () => {
      const ref = storeRef.current!.pages.find((p) => p.key === key)!
      const at = indexOfKey(key) + 1
      const [w, h] = ref.rotation % 2 ? [ref.height, ref.width] : [ref.width, ref.height]
      return { pages: await call((s) => s.engine.call('insertBlank', [at, w, h])), at, count: 1 }
    })
  }

  const insertFromFile = async (afterIndex: number) => {
    const files = (await platform.pickFiles()).filter((f) => f.name.toLowerCase().endsWith('.pdf'))
    if (!files.length) return
    const key = keyAt(afterIndex)
    for (const f of files) {
      addPages(async () => {
        const at = indexOfKey(key) + 1
        const r = await call((s) => s.engine.call('insertPdf', [at, f.bytes.slice().buffer]))
        return { pages: r.pages, at, count: r.count }
      })
    }
  }

  const deletePage = (index: number) => {
    if (pages.length <= 1) return
    let keys = [keyAt(index)]
    let token = 0
    const doIt = async () => {
      const r = await call((s) => s.engine.call('deletePages', [keys]))
      token = r.token
      setDocPages(r.pages)
    }
    void perform(doIt, async () => {
      const restored = await call((s) => s.engine.call('restorePages', [token]))
      setDocPages(restored)
      // Restored pages are copies with new keys.
      keys = [restored[index].key]
      goToKey(keys[0])
    })
  }

  const extract = async (indices: number[]) => {
    try {
      const keys = indices.map(keyAt)
      const bytes = new Uint8Array(await call((s) => s.engine.call('extract', [keys])))
      const name = `${tab.title} (page${indices.length > 1 ? 's' : ''} ${indices.map((i) => i + 1).join(', ')})`
      const path = await platform.pickSavePath(name, 'pdf')
      if (path) await platform.writeFile(path, bytes)
    } catch (err) {
      await platform.showError(String(err))
    }
  }

  const pageMenu = (index: number): MenuEntry[] => [
    { label: 'Rotate right', icon: <RotateCw size={14} />, onSelect: () => rotatePage(index, 1) },
    { label: 'Rotate left', icon: <RotateCcw size={14} />, onSelect: () => rotatePage(index, -1) },
    { label: 'Duplicate', icon: <Copy size={14} />, onSelect: () => duplicatePage(index), separatorBefore: true },
    { label: 'Insert blank page after', icon: <FilePlus2 size={14} />, onSelect: () => insertBlank(index) },
    { label: 'Insert pages from file…', icon: <ArrowDownToLine size={14} />, onSelect: () => void insertFromFile(index) },
    { label: 'Extract as PDF…', icon: <FileOutput size={14} />, onSelect: () => void extract([index]), separatorBefore: true },
    ...(pages.length > 1 ? [{ label: 'Delete page', icon: <Trash2 size={14} />, danger: true, onSelect: () => deletePage(index), separatorBefore: true }] : []),
  ]

  // ── Annotations

  /** Resolves once a page has been redrawn, so a preview can stay up until the real thing shows. */
  const afterRender = (key: number) =>
    new Promise<void>((resolve) => {
      const timer = window.setTimeout(done, 1500)
      function done() {
        window.clearTimeout(timer)
        document.removeEventListener('pagerendered', onRendered)
        resolve()
      }
      function onRendered(e: Event) {
        if ((e as CustomEvent<{ key: number }>).detail.key === key) done()
      }
      document.addEventListener('pagerendered', onRendered)
    })

  const createAnnot = (key: number, annot: PdfAnnotationObject, image?: ArrayBuffer, select = false) => {
    let id = ''
    let token = 0
    return perform(
      async () => {
        id = await call((s) => s.engine.call('createAnnotation', [key, annot, image ?? null], { priority: Priority.action }))
        touch(key)
        if (select) {
          setTool('select')
          setSelected({ key, id })
        }
      },
      async () => {
        token = await call((s) => s.engine.call('removeAnnotation', [key, { ...annot, id }]))
        touch(key)
      },
      async () => {
        id = await call((s) => s.engine.call('restoreAnnotation', [token]))
        touch(key)
      },
    )
  }

  // Set below, once the signing helpers exist; the actions are made once and must see the latest.
  const signIntoRef = useRef<(key: number, annot: Annot) => Promise<void>>(async () => {})
  const actions = useMemo<AnnotationActions>(
    () => ({
      select: (sel) => setSelected(sel),
      update: (index, before, after) => {
        const key = keyAt(index)
        const apply = (a: PdfAnnotationObject) => async () => {
          await call((s) => s.engine.call('updateAnnotation', [key, a]))
          touch(key)
        }
        void perform(apply(after), apply(before))
      },
      remove: (index, annot) => {
        const key = keyAt(index)
        let token = 0
        setSelected(null)
        const remove = async () => {
          token = await call((s) => s.engine.call('removeAnnotation', [key, annot.raw]))
          touch(key)
        }
        void perform(remove, async () => {
          await call((s) => s.engine.call('restoreAnnotation', [token]))
          touch(key)
        })
      },
      setField: (index, annot, value) => {
        const key = keyAt(index)
        void run(async () => {
          await call((s) => s.engine.call('setField', [key, annot.raw, value]))
          fieldsDirty.current = true
          touch(key)
          refreshDirty()
        })
      },
      signField: (index, annot) => {
        void signIntoRef.current(keyAt(index), annot)
      },
      openLink: (link) => {
        if (link.url) {
          if (/^(https?:|mailto:)/i.test(link.url)) void platform.openExternal(link.url)
        } else if (link.page !== undefined) viewer.current?.goTo(link.page, link.y)
      },
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [perform, run, refreshDirty],
  )

  const highlightSelection = useCallback(async () => {
    const s = storeRef.current
    if (!s) return
    const ranges = await selectedCharacters(s)
    if (!ranges.length) return
    window.getSelection()?.removeAllRanges()
    for (const r of ranges) {
      const boxes = await s.engine.call('textBoxes', [r.key, r.start, r.end - r.start + 1])
      if (!boxes.length) continue
      void createAnnot(r.key, {
        type: PdfAnnotationSubtype.HIGHLIGHT,
        id: '',
        pageIndex: r.index,
        rect: toRect(union(boxes)),
        segmentRects: boxes.map(toRect),
        strokeColor: highlightColor,
        color: highlightColor,
        opacity: 1,
        blendMode: PdfBlendMode.Multiply,
      } as PdfAnnotationObject)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [highlightColor])

  // Highlight tool: selecting text highlights it.
  useEffect(() => {
    if (tool !== 'highlight' || !active) return
    const onUp = () => window.setTimeout(() => void highlightSelection(), 0)
    document.addEventListener('pointerup', onUp)
    return () => document.removeEventListener('pointerup', onUp)
  }, [tool, active, highlightSelection])

  // Drawing: strokes collect into one annotation until a pause, another page, or another tool.
  const inkTimer = useRef(0)
  const inkRef = useRef(ink)
  inkRef.current = ink
  const commitInk = useCallback(() => {
    window.clearTimeout(inkTimer.current)
    const session = inkRef.current
    if (!session || session.committing || !session.strokes.length) return
    setInk({ ...session, committing: true })
    const half = inkThickness / 2
    const points = session.strokes.flat()
    const box = union(points.map((p) => ({ x: p.x - half, y: p.y - half, w: inkThickness, h: inkThickness })))
    void createAnnot(session.key, {
      type: PdfAnnotationSubtype.INK,
      id: '',
      pageIndex: indexOfKey(session.key),
      rect: toRect(box),
      inkList: session.strokes.map((s) => ({ points: s })),
      strokeColor: inkColor,
      color: inkColor,
      strokeWidth: inkThickness,
      opacity: 1,
    } as PdfAnnotationObject).then(async () => {
      await afterRender(session.key)
      setInk((cur) => (cur === null || cur.committing ? null : cur))
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inkColor, inkThickness])

  const addStroke = (key: number, stroke: Stroke) => {
    if (inkRef.current && inkRef.current.key !== key) commitInk()
    const session = inkRef.current && inkRef.current.key === key && !inkRef.current.committing ? inkRef.current : { key, strokes: [] }
    const next = { key, strokes: [...session.strokes, stroke] }
    inkRef.current = next
    setInk(next)
    window.clearTimeout(inkTimer.current)
    inkTimer.current = window.setTimeout(commitInk, INK_IDLE_MS)
  }

  // Text tool
  const finishDraft = (key: number, text: string, box: Box) => {
    if (!text.trim()) {
      setDraft(null)
      return
    }
    void createAnnot(key, {
      type: PdfAnnotationSubtype.FREETEXT,
      id: '',
      pageIndex: indexOfKey(key),
      rect: toRect(box),
      contents: text,
      fontFamily: PdfStandardFont.Helvetica,
      fontSize,
      fontColor: inkColor,
      textAlign: PdfTextAlignment.Left,
      verticalAlign: PdfVerticalAlignment.Top,
      opacity: 1,
    } as PdfAnnotationObject).then(async () => {
      await afterRender(key)
      setDraft((d) => (d?.key === key && d.x === box.x ? null : d))
    })
  }

  // Image tool: pick a picture, drop it in the middle of the current page, selected so it can be moved.
  const placeImage = async () => {
    const s = storeRef.current
    const file = (await platform.pickImages())[0]
    setTool('select')
    if (!s || !file) return
    try {
      const { bytes, width, height } = await imageForPdf(file)
      const target = s.pages[page]
      const scale = Math.min(1, (target.width * 0.5) / (width * 0.75), (target.height * 0.5) / (height * 0.75))
      const w = width * 0.75 * scale
      const h = height * 0.75 * scale
      const box = { x: (target.width - w) / 2, y: (target.height - h) / 2, w, h }
      await createAnnot(target.key, { type: PdfAnnotationSubtype.STAMP, id: '', pageIndex: page, rect: toRect(box) } as PdfAnnotationObject, bytes, true)
    } catch (err) {
      await platform.showError(`Couldn’t add that image. ${err instanceof Error ? err.message : ''}`)
    }
  }

  // Sign tool: what's picked in the inspector goes where you click, or fills a signature field.
  /** A signature field clicked before there was a signature to put in it; filled once one is made. */
  const pendingField = useRef<{ key: number; box: Box } | null>(null)

  const place = async (key: number, choice: SignChoice, at: { x: number; y: number } | { fit: Box }) => {
    const s = storeRef.current
    const target = s?.pages.find((p) => p.key === key)
    if (!s || !target) return
    const index = indexOfKey(key)
    /** w × h placed with its left edge and middle at the click, or centered in a field. */
    const boxFor = (w: number, h: number): Box => {
      if ('fit' in at) {
        const f = at.fit
        const scale = Math.min((f.w * 0.94) / w, (f.h * 0.86) / h)
        return { x: f.x + (f.w - w * scale) / 2, y: f.y + (f.h - h * scale) / 2, w: w * scale, h: h * scale }
      }
      const x = Math.max(0, Math.min(at.x, target.width - w))
      const y = Math.max(0, Math.min(at.y - h / 2, target.height - h))
      return { x, y, w, h }
    }
    if (choice.type === 'date') {
      const text = new Date().toLocaleDateString()
      const size = 'fit' in at ? Math.max(6, Math.min(fontSize, at.fit.h * 0.6)) : fontSize
      const w = text.length * size * 0.56 + 4
      const h = size * 1.3
      const box = 'fit' in at ? { x: at.fit.x + 2, y: at.fit.y + (at.fit.h - h) / 2, w, h } : boxFor(w, h)
      await createAnnot(
        key,
        {
          type: PdfAnnotationSubtype.FREETEXT,
          id: '',
          pageIndex: index,
          rect: toRect(box),
          contents: text,
          fontFamily: PdfStandardFont.Helvetica,
          fontSize: size,
          fontColor: inkColor,
          textAlign: PdfTextAlignment.Left,
          verticalAlign: PdfVerticalAlignment.Middle,
          opacity: 1,
        } as PdfAnnotationObject,
        undefined,
        true,
      )
      return
    }
    let image: { bytes: ArrayBuffer; w: number; h: number }
    if (choice.type === 'saved') {
      const sig = useSignatures.getState().list.find((x) => x.id === choice.id)
      if (!sig) return
      const w = SIGNATURE_WIDTH[sig.kind]
      image = { bytes: pngBytes(sig.png), w, h: (w * sig.height) / sig.width }
    } else {
      const mark = markPng(choice.type, inkColor)
      image = { bytes: mark.bytes, w: MARK_SIZE, h: MARK_SIZE }
    }
    const box = boxFor(image.w, image.h)
    await createAnnot(key, { type: PdfAnnotationSubtype.STAMP, id: '', pageIndex: index, rect: toRect(box) } as PdfAnnotationObject, image.bytes, true)
  }

  /** Clicked a signature field: sign it with the picked signature, or the newest one, or make one. */
  const signInto = async (key: number, annot: Annot) => {
    const { choice, list } = useSignatures.getState()
    const pick: SignChoice | null =
      choice?.type === 'saved' && list.some((x) => x.id === choice.id) ? choice : list.find((x) => x.kind === 'signature') ? { type: 'saved', id: list.find((x) => x.kind === 'signature')!.id } : null
    if (!pick) {
      pendingField.current = { key, box: annot.box }
      setPad('signature')
      return
    }
    await place(key, pick, { fit: annot.box })
  }

  signIntoRef.current = signInto

  const placeOnPage = (key: number, x: number, y: number) => {
    const { choice, list } = useSignatures.getState()
    if (!choice) {
      // Nothing picked yet: the newest signature, or make the first one.
      const first = list.find((x) => x.kind === 'signature') ?? list[0]
      if (!first) {
        setPad('signature')
        return
      }
      useSignatures.getState().choose({ type: 'saved', id: first.id })
      void place(key, { type: 'saved', id: first.id }, { x, y })
      return
    }
    void place(key, choice, { x, y })
  }

  const padClosed = (saved: SavedSignature | null) => {
    setPad(null)
    const field = pendingField.current
    pendingField.current = null
    if (saved && field) void place(field.key, { type: 'saved', id: saved.id }, { fit: field.box })
    else if (saved && tool !== 'sign') setTool('sign')
  }

  // ── Forms

  /** Every fillable field in page order, read when asked (a long form's pages load lazily). */
  const formFields = async () => {
    const s = storeRef.current
    if (!s) return []
    const out: { index: number; annot: Annot }[] = []
    for (let i = 0; i < s.pages.length; i++) {
      for (const a of await s.getAnnotations(i)) {
        const f = a.field
        if (f && !f.readOnly && f.kind !== 'button' && f.kind !== 'other') out.push({ index: i, annot: a })
      }
    }
    return out
  }

  const [formStats, setFormStats] = useState<{ total: number; filled: number } | null>(null)
  const isFilled = (a: Annot) => {
    const f = a.field!
    if (f.kind === 'checkbox' || f.kind === 'radio') return f.checked
    if (f.kind === 'combo' || f.kind === 'list') return f.options.some((o) => o.selected)
    if (f.kind === 'signature') return false
    return !!f.value.trim()
  }
  // Recount after each change; radio groups count once.
  const revSig = useMemo(() => pages.map((p) => storeRef.current?.rev(p.key) ?? 0).join(','), [pages])
  useEffect(() => {
    if (!hasForm || !store) return
    let live = true
    void formFields().then((fields) => {
      if (!live) return
      const groups = new Map<string, boolean>()
      for (const { annot } of fields) {
        if (annot.field!.kind === 'signature') continue
        const name = annot.field!.name || annot.id
        groups.set(name, (groups.get(name) ?? false) || isFilled(annot))
      }
      setFormStats({ total: groups.size, filled: [...groups.values()].filter(Boolean).length })
    })
    return () => {
      live = false
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasForm, store, revSig])

  /** Go to the next field after the current page that's still empty, and put the cursor in it. */
  const nextEmptyField = async () => {
    selectTool('select')
    const fields = (await formFields()).filter(({ annot }) => !isFilled(annot) && annot.field!.kind !== 'signature')
    if (!fields.length) return
    const focused = document.activeElement
    const after = fields.findIndex(({ index, annot }) => index > page || (index === page && !(focused instanceof HTMLElement && focused.getAttribute('data-annot') === annot.id)))
    const next = fields[after === -1 ? 0 : after]
    viewer.current?.reveal(next.index, next.annot.box)
    // The page's fields appear once it's drawn.
    for (let tries = 0; tries < 20; tries++) {
      await new Promise((r) => setTimeout(r, 60))
      const el = stageRef.current?.querySelector<HTMLElement>(`[data-annot="${CSS.escape(next.annot.id)}"]`)
      if (el) {
        el.focus({ preventScroll: true })
        return
      }
    }
  }

  const saveFlattened = async () => {
    try {
      commitInk()
      await queue.current
      const bytes = new Uint8Array(await call((s) => s.engine.call('flatten', [], { priority: Priority.action })))
      const path = await platform.pickSavePath(`${tab.title} (flattened)`, 'pdf')
      if (path) await platform.writeFile(path, bytes)
    } catch (err) {
      await platform.showError(`Couldn’t make a flattened copy. ${err instanceof Error ? err.message : String(err)}`)
    }
  }

  const selectTool = (id: ToolId) => {
    commitInk()
    ;(document.activeElement as HTMLElement | null)?.blur?.()
    setSelected(null)
    // Picking Highlight with text already selected highlights it right away.
    if (id === 'highlight') void highlightSelection()
    else clearTextSelection()
    if (id === 'image') {
      setTool('image')
      void placeImage()
      return
    }
    setTool(id)
  }

  // ── Find

  const findJob = useRef<AbortController | null>(null)
  const runFind = useCallback(
    (query: string) => {
      findJob.current?.abort()
      const s = storeRef.current
      if (!query.trim() || !s) {
        setFind(null)
        return
      }
      const job = new AbortController()
      findJob.current = job
      const hits = new Map<number, Box[][]>()
      setFind({ query, hits, done: false })
      setFindIndex(-1)
      const order = s.pages.map((_, i) => (i + page) % s.pages.length)
      let lastFlush = performance.now()
      void (async () => {
        for (const i of order) {
          const p = s.pages[i]
          if (!p) break
          try {
            const found = await s.engine.call('search', [p.key, query, false], { priority: Priority.search, signal: job.signal })
            if (found.length) hits.set(p.key, found)
          } catch (err) {
            if (isCancelled(err) && job.signal.aborted) return
          }
          if (performance.now() - lastFlush > 120) {
            lastFlush = performance.now()
            setFind({ query, hits: new Map(hits), done: false })
          }
        }
        if (!job.signal.aborted) setFind({ query, hits: new Map(hits), done: true })
      })()
    },
    [page],
  )

  /** Matches in document order: [page index, match number on that page]. */
  const matchList = useMemo(() => {
    const list: [number, number][] = []
    if (!find) return list
    pages.forEach((p, i) => {
      const h = find.hits.get(p.key)
      if (h) for (let m = 0; m < h.length; m++) list.push([i, m])
    })
    return list
  }, [find, pages])

  // Jump to the first match at or after the current page as soon as there is one.
  useEffect(() => {
    if (findIndex !== -1 || !matchList.length) return
    const first = Math.max(0, matchList.findIndex(([i]) => i >= page))
    setFindIndex(first)
    revealMatch(first)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [matchList, findIndex])

  const revealMatch = (n: number) => {
    const m = matchList[n]
    if (!m || !find) return
    const boxes = find.hits.get(pages[m[0]].key)?.[m[1]]
    if (boxes?.length) viewer.current?.reveal(m[0], union(boxes))
  }
  const stepMatch = (dir: 1 | -1) => {
    if (!matchList.length) return
    const n = (Math.max(0, findIndex) + dir + matchList.length) % matchList.length
    setFindIndex(n)
    revealMatch(n)
  }

  // Page edits move or reshape pages, so search again.
  const layoutSig = useMemo(() => pages.map((p) => `${p.key}:${p.rotation}`).join(','), [pages])
  const lastSig = useRef(layoutSig)
  useEffect(() => {
    if (lastSig.current === layoutSig) return
    lastSig.current = layoutSig
    if (find) runFind(find.query)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [layoutSig])

  // ── Shell integration
  const saveBytes = async () => {
    // Rewriting an unchanged PDF breaks digital signatures and can alter unsupported content.
    // The clean bytes also cover repeated saves after an edit and undo back to the saved state.
    if (!history.current.dirty && !fieldsDirty.current) {
      return cleanBytes.current ?? tab.bytes ?? new Uint8Array()
    }
    return new Uint8Array(await call((s) => s.engine.call('save', [])))
  }
  useEffect(
    () =>
      registerEditor(tab.id, {
        getBytes: async () => {
          commitInk()
          ;(document.activeElement as HTMLElement | null)?.blur?.()
          await queue.current
          const bytes = await saveBytes()
          lastWritten.current = bytes
          return bytes
        },
        // Autosave waits while you're drawing or typing into a form field or text box, since
        // saving commits them (getBytes above) and would take the focus away mid-word.
        busy: () => {
          const focus = document.activeElement
          return !!(inkRef.current && !inkRef.current.committing) || !!draftRef.current || (isEditable(focus) && !!stageRef.current?.contains(focus))
        },
        markSaved: () => {
          cleanBytes.current = lastWritten.current
          history.current.markSaved()
          fieldsDirty.current = false
          refreshDirty()
        },
        find: () => setFindOpen(true),
        zoom: (dir) => {
          if (dir === 'in') viewer.current?.step(1)
          else if (dir === 'out') viewer.current?.step(-1)
          else viewer.current?.setZoom(1)
        },
        print: async () => {
          const s = storeRef.current
          if (s) await printPdf(s)
        },
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tab.id, tab.bytes],
  )

  // Undo/redo and deleting the selected annotation from the keyboard.
  useEffect(() => {
    if (!active) return
    const onKey = (e: KeyboardEvent) => {
      if (isEditable(e.target)) return
      const mod = platform.isMac ? e.metaKey : e.ctrlKey
      const key = e.key.toLowerCase()
      if (mod && key === 'z') {
        e.preventDefault()
        void (e.shiftKey ? redo() : undo())
      } else if (mod && key === 'y' && !platform.isMac) {
        e.preventDefault()
        void redo()
      } else if (e.key === 'Escape') {
        if (selected) setSelected(null)
        else if (tool !== 'select') selectTool('select')
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, undo, redo, selected, tool])

  // Background tabs give their page drawings back after a while; they redraw when shown. One in
  // split view's other pane is still on screen, so it stays awake.
  useEffect(() => {
    if (active || visible) {
      setAwake(true)
      return
    }
    commitInk()
    const timer = window.setTimeout(() => {
      setAwake(false)
      // With nothing unsaved, stop the engine too; it reopens the file when the tab is shown.
      const s = storeRef.current
      if (s && store && !history.current.dirty && !fieldsDirty.current) {
        history.current = new History()
        refreshDirty()
        s.engine.suspend(cleanBytes.current ?? tab.bytes ?? new Uint8Array(), passwordRef.current, (r) => s.reopened(r.pages))
      }
    }, SLEEP_AFTER_MS)
    return () => window.clearTimeout(timer)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, visible])

  // Arrow keys and Page Down scroll the pages as soon as the tab is shown.
  const stageRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (active && store) stageRef.current?.querySelector<HTMLElement>('.pdf-container')?.focus({ preventScroll: true })
  }, [active, store])

  const onPageChange = useCallback((i: number) => setPage(i), [])
  const onZoomChange = useCallback((z: number) => setZoom(z), [])

  const renderPage = ({ index, page: p, unit }: PageSlot) => {
    const s = storeRef.current!
    const hits = find?.hits.get(p.key)
    const current = matchList[findIndex]
    const pendingInk = ink?.key === p.key ? ink.strokes : []
    return (
      <>
        {hits && <SearchLayer boxes={hits} current={current && current[0] === index ? current[1] : -1} />}
        <TextLayer store={s} index={index} page={p} />
        <AnnotationLayer
          store={s}
          index={index}
          page={p}
          rev={s.rev(p.key)}
          unit={unit}
          interactive={tool === 'select' || tool === 'highlight'}
          selectable={tool === 'select'}
          selectedId={selected?.key === p.key ? selected.id : null}
          actions={actions}
        />
        {tool === 'draw' ? (
          <DrawLayer unit={unit} color={inkColor} width={inkThickness} strokes={pendingInk} onStroke={(stroke) => addStroke(p.key, stroke)} />
        ) : (
          pendingInk.length > 0 && (
            <div className="tool-layer passive">
              <InkPreview strokes={pendingInk} color={inkColor} width={inkThickness} />
            </div>
          )
        )}
        {tool === 'sign' && <PlaceLayer unit={unit} onPlace={(x, y) => placeOnPage(p.key, x, y)} />}
        {tool === 'text' && (
          <TextToolLayer
            unit={unit}
            onPlace={(x, y) => {
              ;(document.activeElement as HTMLElement | null)?.blur?.()
              setDraft({ key: p.key, x, y })
            }}
          />
        )}
        {draft?.key === p.key && (
          <TextDraft key={`${draft.x},${draft.y}`} x={draft.x} y={draft.y} unit={unit} fontSize={fontSize} color={inkColor} onDone={(text, box) => finishDraft(p.key, text, box)} />
        )}
      </>
    )
  }

  const numPages = pages.length
  const currentTool = TOOLS.find((t) => t.id === tool)!
  const currentPage = pages[page]
  const hist = history.current

  const left = (
    <>
      <div className="sidebar-tabs">
        <Segmented
          value={sideTab}
          onChange={setSideTab}
          options={[
            { value: 'pages', label: 'Pages' },
            { value: 'outline', label: 'Outline' },
          ]}
        />
      </div>
      {store &&
        awake &&
        (sideTab === 'pages' ? (
          <Thumbnails
            store={store}
            pages={pages}
            current={page}
            onSelect={(n) => viewer.current?.goTo(n)}
            onMenu={(index, x, y) => setMenu({ index, x, y })}
            onMove={movePage}
          />
        ) : (
          <Outline
            store={store}
            onNavigate={(item: OutlineItem) => {
              if (item.page !== undefined) viewer.current?.goTo(item.page, item.y)
              else if (item.url && /^(https?:|mailto:)/i.test(item.url)) void platform.openExternal(item.url)
            }}
          />
        ))}
    </>
  )

  const right = (
    <>
      <InspectorHeader kind="pdf">PDF</InspectorHeader>
      <Section title="Annotate">
        <div className="tool-grid">
          {TOOLS.map((t) => (
            <ToolButton key={t.id} icon={t.icon} label={t.label} active={tool === t.id} title={t.hint} onClick={() => selectTool(t.id)} disabled={!store} />
          ))}
        </div>
        <p className="tool-hint faint">{currentTool.hint}.</p>
        {tool === 'highlight' && (
          <div className="swatches">
            {HIGHLIGHT_COLORS.map((c) => (
              <button key={c} type="button" aria-label={`Highlight color ${c}`} className={`swatch ${highlightColor === c ? 'on' : ''}`} style={{ background: c }} onClick={() => setHighlightColor(c)} />
            ))}
          </div>
        )}
        {tool === 'sign' && <SignPanel onNew={setPad} />}
        {(tool === 'draw' || tool === 'text') && (
          <>
            <div className="swatches">
              {INK_COLORS.map((c) => (
                <button key={c} type="button" aria-label={`Color ${c}`} className={`swatch ${inkColor === c ? 'on' : ''}`} style={{ background: c }} onClick={() => setInkColor(c)} />
              ))}
            </div>
            {tool === 'draw' ? (
              <label className="slider-row">
                <span className="muted">Thickness</span>
                <input type="range" min={1} max={20} value={inkThickness} onChange={(e) => setInkThickness(Number(e.currentTarget.value))} />
                <span className="value">{inkThickness}</span>
              </label>
            ) : (
              <label className="slider-row">
                <span className="muted">Size</span>
                <input type="range" min={6} max={72} value={fontSize} onChange={(e) => setFontSize(Number(e.currentTarget.value))} />
                <span className="value">{fontSize}</span>
              </label>
            )}
          </>
        )}
      </Section>
      {hasForm && (
        <Section title="Form">
          {formStats && (
            <p className="form-progress">
              <span className="form-progress-bar">
                <span style={{ width: `${formStats.total ? (formStats.filled / formStats.total) * 100 : 0}%` }} />
              </span>
              <span className="muted">
                {formStats.filled} of {formStats.total} filled
              </span>
            </p>
          )}
          <div className="grid-2">
            <Button disabled={!store} onClick={() => void nextEmptyField()}>
              <StepForward size={14} /> Next empty
            </Button>
            <Button className="pdf-inspector-toggle" aria-pressed={showFields} onClick={() => setShowFields(!showFields)}>
              <SquareDashed size={14} /> Show fields
            </Button>
          </div>
          <Button block disabled={!store} onClick={() => void saveFlattened()} title="A copy whose fields, signatures and marks are part of the page, so they can’t be changed">
            <Stamp size={14} /> Save flattened copy…
          </Button>
        </Section>
      )}
      <Section title={numPages ? `Page ${page + 1} of ${numPages}` : 'Page'}>
        <div className="grid-2">
          <Button disabled={!store} title="Turn the page a quarter turn counterclockwise" onClick={() => rotatePage(page, -1)}>
            <RotateCcw size={14} /> Rotate left
          </Button>
          <Button disabled={!store} title="Turn the page a quarter turn clockwise" onClick={() => rotatePage(page, 1)}>
            <RotateCw size={14} /> Rotate right
          </Button>
          <Button disabled={!store || page <= 0} onClick={() => movePage(page, page - 1)}>
            <ArrowUpToLine size={14} /> Move up
          </Button>
          <Button disabled={!store || page >= numPages - 1} onClick={() => movePage(page, page + 1)}>
            <ArrowDownToLine size={14} /> Move down
          </Button>
          <Button disabled={!store} onClick={() => duplicatePage(page)}>
            <Copy size={14} /> Duplicate
          </Button>
          <Button variant="danger" disabled={!store || numPages <= 1} onClick={() => deletePage(page)}>
            <Trash2 size={14} /> Delete
          </Button>
        </div>
        <Button block disabled={!store} onClick={() => void insertFromFile(page)}>
          <FilePlus2 size={14} /> Insert pages from file…
        </Button>
        <Button block disabled={!store} onClick={() => void extract([page])}>
          <FileOutput size={14} /> Extract page as PDF…
        </Button>
        {!hasForm && (
          <Button block disabled={!store} onClick={() => void saveFlattened()} title="A copy whose annotations, signatures and marks are part of the page, so they can’t be changed">
            <Stamp size={14} /> Save flattened copy…
          </Button>
        )}
      </Section>
      <Section title="Document" collapsible>
        <dl className="stats">
          {meta.title && (
            <>
              <dt>Title</dt>
              <dd className="ellipsis" title={meta.title}>
                {meta.title}
              </dd>
            </>
          )}
          {meta.author && (
            <>
              <dt>Author</dt>
              <dd className="ellipsis">{meta.author}</dd>
            </>
          )}
          <dt>Pages</dt>
          <dd>{numPages}</dd>
          {currentPage && (
            <>
              <dt>Page size</dt>
              <dd>{pageSizeName(currentPage.width, currentPage.height)}</dd>
            </>
          )}
          <dt>File size</dt>
          <dd>{formatBytes(tab.bytes?.byteLength ?? 0)}</dd>
          {meta.version && (
            <>
              <dt>PDF version</dt>
              <dd>{meta.version}</dd>
            </>
          )}
          {meta.producer && (
            <>
              <dt>Created with</dt>
              <dd className="wrap">{meta.producer}</dd>
            </>
          )}
        </dl>
      </Section>
    </>
  )

  return (
    // A file that didn't open has no pages to show or tools to use.
    <Workbench tab={tab} left={error ? undefined : left} right={error ? undefined : right} leftFooter={numPages ? <span>{numPages === 1 ? '1 page' : `${numPages} pages`}</span> : undefined}>
      <div ref={stageRef} className={`pdf-stage tool-${tool} ${hasForm && showFields ? 'show-fields' : ''}`}>
        {store && (
          <Viewer ref={viewer} store={store} pages={pages} awake={awake} onPageChange={onPageChange} onZoomChange={onZoomChange} renderPage={renderPage} positionKey={tab.id} />
        )}
      </div>
      {!store && !error && !password && <CenterState spinner />}
      {error && (
        <CenterState title="Couldn’t open this PDF">
          <p className="muted">{error}</p>
        </CenterState>
      )}
      {password && <PasswordPrompt retry={password.retry} onSubmit={(pw) => void openWith(pw)} />}
      <FindBar
        open={findOpen}
        onClose={() => {
          setFindOpen(false)
          runFind('')
        }}
        onQuery={runFind}
        onNext={() => stepMatch(1)}
        onPrev={() => stepMatch(-1)}
        count={find ? (find.done || matchList.length ? matchList.length : null) : null}
        current={matchList.length ? Math.max(0, findIndex) : null}
        placeholder="Find in PDF"
      />
      {store && (
        <div className="canvas-bar">
          <IconButton label="Undo (⌘Z)" size="sm" disabled={!hist.canUndo || busy} onClick={() => void undo()}>
            <Undo2 size={14} />
          </IconButton>
          <IconButton label="Redo (⇧⌘Z)" size="sm" disabled={!hist.canRedo || busy} onClick={() => void redo()}>
            <Redo2 size={14} />
          </IconButton>
          <span className="sep" />
          <input
            className="readout"
            aria-label="Page number"
            key={page}
            defaultValue={page + 1}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                const n = Number(e.currentTarget.value)
                if (n >= 1 && n <= numPages) viewer.current?.goTo(n - 1)
                e.currentTarget.blur()
              }
            }}
          />
          <span className="faint" style={{ paddingRight: 8 }}>
            / {numPages}
          </span>
          <span className="sep" />
          <IconButton label="Zoom out (⌘−)" size="sm" onClick={() => viewer.current?.step(-1)}>
            <Minus size={14} />
          </IconButton>
          <IconButton label="Actual size (⌘0)" size="sm" className="readout" aria-label={formatZoom(zoom)} onClick={() => viewer.current?.setZoom(1)}>
            {formatZoom(zoom)}
          </IconButton>
          <IconButton label="Zoom in (⌘+)" size="sm" onClick={() => viewer.current?.step(1)}>
            <Plus size={14} />
          </IconButton>
          <span className="sep" />
          <IconButton label="Fit width" size="sm" onClick={() => viewer.current?.setZoom('page-width')}>
            <MoveHorizontal size={14} />
          </IconButton>
          <IconButton label="Fit page" size="sm" onClick={() => viewer.current?.setZoom('page-fit')}>
            <Scaling size={14} />
          </IconButton>
        </div>
      )}
      {menu && <PopoverMenu anchor={menu} items={pageMenu(menu.index)} onClose={() => setMenu(null)} />}
      {pad && <SignaturePad kind={pad} onClose={padClosed} />}
    </Workbench>
  )
}

function PasswordPrompt({ retry, onSubmit }: { retry: boolean; onSubmit: (pw: string) => void }) {
  const [value, setValue] = useState('')
  return (
    <div className="center-state">
      <form
        className="box password-box"
        onSubmit={(e) => {
          e.preventDefault()
          onSubmit(value)
        }}
      >
        <h3>This PDF is password protected</h3>
        {retry && <p style={{ color: 'var(--danger)', margin: 0 }}>That password didn’t work. Try again.</p>}
        <TextField type="password" autoFocus placeholder="Password" value={value} onChange={(e) => setValue(e.currentTarget.value)} />
        <Button variant="primary" type="submit">
          Unlock
        </Button>
      </form>
    </div>
  )
}

function pageSizeName(w: number, h: number) {
  const [a, b] = [Math.min(w, h), Math.max(w, h)]
  const near = (x: number, y: number) => Math.abs(x - y) < 3
  if (near(a, 612) && near(b, 792)) return 'US Letter'
  if (near(a, 612) && near(b, 1008)) return 'US Legal'
  if (near(a, 595) && near(b, 842)) return 'A4'
  if (near(a, 420) && near(b, 595)) return 'A5'
  if (near(a, 842) && near(b, 1191)) return 'A3'
  return `${Math.round((w / 72) * 25.4)} × ${Math.round((h / 72) * 25.4)} mm`
}

function formatBytes(n: number) {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`
  return `${(n / 1024 / 1024).toFixed(1)} MB`
}

/** PNG and JPEG go into the PDF as they are; anything else is converted to PNG. */
async function imageForPdf(file: File): Promise<{ bytes: ArrayBuffer; width: number; height: number }> {
  const url = URL.createObjectURL(file)
  try {
    const img = new Image()
    img.src = url
    await img.decode()
    const width = img.naturalWidth || 300
    const height = img.naturalHeight || 300
    if (file.type === 'image/png' || file.type === 'image/jpeg') return { bytes: await file.arrayBuffer(), width, height }
    const canvas = document.createElement('canvas')
    canvas.width = width
    canvas.height = height
    canvas.getContext('2d')!.drawImage(img, 0, 0, width, height)
    const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, 'image/png'))
    canvas.width = canvas.height = 0
    if (!blob) throw new Error('The image couldn’t be read.')
    return { bytes: await blob.arrayBuffer(), width, height }
  } finally {
    URL.revokeObjectURL(url)
  }
}

/** Draw each page into the print area, one at a time, then open the system print panel. */
async function printPdf(store: DocStore) {
  const root = document.getElementById('print-root')!
  for (const img of root.querySelectorAll('img')) URL.revokeObjectURL(img.src)
  root.innerHTML = ''
  const canvas = document.createElement('canvas')
  try {
    for (const p of store.pages) {
      // 150 dpi is plenty for paper and keeps a long document's print job a sensible size.
      const bitmap = await store.engine.call('render', [p.key, 150 / 72, null], { priority: Priority.action })
      canvas.width = bitmap.width
      canvas.height = bitmap.height
      drawBitmap(canvas, bitmap)
      const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, 'image/jpeg', 0.92))
      const div = document.createElement('div')
      div.className = 'print-page'
      const img = new Image()
      img.src = blob ? URL.createObjectURL(blob) : canvas.toDataURL()
      div.appendChild(img)
      root.appendChild(div)
    }
    await Promise.all(Array.from(root.querySelectorAll('img')).map((img) => img.decode().catch(() => {})))
    window.print()
  } finally {
    canvas.width = canvas.height = 0
  }
}
