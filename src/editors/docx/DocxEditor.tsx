import {
  AlignCenter,
  AlignJustify,
  AlignLeft,
  AlignRight,
  ArrowDownToLine,
  ArrowLeftToLine,
  ArrowRightToLine,
  ArrowUpToLine,
  Bold,
  Crop,
  FileStack,
  Hash,
  Image as ImageIcon,
  IndentDecrease,
  IndentIncrease,
  Italic,
  Link,
  List,
  ListOrdered,
  Minus,
  PanelBottom,
  PanelTop,
  PilcrowLeft,
  PilcrowRight,
  Plus,
  Redo2,
  RemoveFormatting,
  Strikethrough,
  Subscript,
  Superscript,
  Table,
  Trash2,
  Underline,
  Undo2,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { EditorProps } from '../../App'
import type { Rect } from '../../components/CropOverlay'
import { CropOverlay } from '../../components/CropOverlay'
import { FindBar, InspectorHeader, Workbench, formatZoom } from '../../components/Workbench'
import { Button, ColorField, IconButton, NumberField, Section, Segmented, SelectField, TableSizePicker, ToolButton } from '../../components/ui'
import type { CommentEditor } from '../../engine/word/comments'
import { commentEditor } from '../../engine/word/comments'
import type { WordEngine } from '../../engine/word/engine'
import { itemsHtml } from '../../engine/paint'
import {
  DocxEditor,
  ImageInsertTrigger,
  slotCommand,
  useDocumentOutline,
  useDocumentSearch,
  useDocxEditor,
  useEditorCommand,
  useEditorEvent,
  useEditorState,
  useEditorValueCommand,
  useFontFamily,
  useHeaderFooterState,
  useHyperlinkPopup,
  usePageSetup,
  useParagraphStyle,
  useWordSelector,
  useZoom,
} from '../../engine/word/react'
import { usePinchZoom } from '../../lib/pinch'
import { isMac, openExternal, shortcut } from '../../lib/platform'
import { withKeys } from '../../state/shortcuts'
import { registerEditor, useWorkspace } from '../../state/workspace'
import { CommentPeek, CommentStore, CommentsSection } from './Comments'
import { EXTRA_FONTS, useExtraFonts } from '../../state/fonts'
import './docx.css'

// The link popover; the page view shows it beside the pages.
const POPUPS = { hyperlink: () => <LinkPopover /> }

export default function DocxTab({ tab, active }: EditorProps) {
  const commentsChanged = useRef(false)
  const docChanged = useRef(false)
  const markDirty = useWorkspace((s) => s.markDirty)
  const comments = useMemo(
    () =>
      new CommentStore(() => {
        commentsChanged.current = true
        markDirty(tab.id, true)
      }),
    [tab.id, markDirty],
  )

  return (
    <DocxEditor.Root
      document={tab.bytes}
      popups={POPUPS}
      onReady={(editor) => {
        if (import.meta.env.DEV || import.meta.env.VITE_TEST_HOOKS) Object.assign(window, { __docxEditor: editor })
        commentsChanged.current = false
        docChanged.current = false
        comments.load(commentEditor(editor), tab.bytes)
      }}
      onChange={(change) => {
        if (change.source === 'load') return
        docChanged.current = change.revision !== 0
        markDirty(tab.id, docChanged.current || commentsChanged.current)
      }}
    >
      <DocxWorkbench tab={tab} active={active} comments={comments} commentsChanged={commentsChanged} />
    </DocxEditor.Root>
  )
}

function DocxWorkbench({ tab, active, comments, commentsChanged }: EditorProps & { comments: CommentStore; commentsChanged: React.MutableRefObject<boolean> }) {
  const editor = useDocxEditor()
  const page = useEditorState((s) => s.page)
  const zoom = useZoom()
  const search = useDocumentSearch()
  const [findOpen, setFindOpen] = useState<false | 'find' | 'replace'>(false)
  const editorRef = useRef<WordEngine | null>(null)
  editorRef.current = editor
  const host = useMemo(() => (editor ? commentEditor(editor) : null), [editor])
  const hostRef = useRef<CommentEditor | null>(host)
  hostRef.current = host
  useEffect(() => {
    if (!editor) return
    let revision = editor.revision
    return editor.subscribe(() => {
      if (editor.revision !== revision) {
        revision = editor.revision
        useWorkspace.getState().markDirty(tab.id, editor.state.dirty || commentsChanged.current)
      }
    })
  }, [editor, tab.id, commentsChanged])
  const zoomRef = useRef(zoom)
  zoomRef.current = zoom
  const stageRef = useRef<HTMLDivElement>(null)
  const { openAtCaret } = useHyperlinkPopup()
  const openLinkRef = useRef(openAtCaret)
  openLinkRef.current = openAtCaret
  const rightOpen = useWorkspace((s) => (s.split?.includes(tab.id) ? !!s.splitPanels[tab.id]?.right : s.rightOpen.docx))
  const rightOpenRef = useRef(rightOpen)
  rightOpenRef.current = rightOpen
  usePinchZoom(stageRef, {
    getZoom: () => zoomRef.current.zoom,
    setZoom: (z) => zoomRef.current.setZoom(z),
    target: () => stageRef.current?.querySelector<HTMLElement>('.docx-content-mount') ?? null,
    min: 0.25,
    max: 4,
  })

  useEffect(
    () =>
      registerEditor(tab.id, {
        getRevision: () => `${editorRef.current?.revision}:${comments.getEditVersion()}`,
        getBytes: async () => {
          const ed = editorRef.current
          const h = hostRef.current
          if (!ed || !h) throw new Error('The document is still loading.')
          return comments.write(h, ed.save())
        },
        markSaved: () => {
          editorRef.current?.run({ cmd: 'markSaved' })
          commentsChanged.current = false
        },
        find: () => setFindOpen('find'),
        replace: () => setFindOpen('replace'),
        zoom: (dir) => {
          const z = zoomRef.current
          if (dir === 'in') z.zoomIn()
          else if (dir === 'out') z.zoomOut()
          else z.auto()
        },
        print: () => printPages(editorRef.current),
        actions: {
          'text.bold': () => runSlot(editorRef.current, 'text.bold'),
          'text.italic': () => runSlot(editorRef.current, 'text.italic'),
          'text.underline': () => runSlot(editorRef.current, 'text.underline'),
          'docx.link': () => {
            if (!editorRef.current?.state.hasSelection) return false
            openLinkRef.current()
          },
          'docx.comment': () => {
            const h = hostRef.current
            if (!h || !comments.begin(h)) return false
            // The comment is written in the inspector.
            if (!rightOpenRef.current) useWorkspace.getState().toggleRight('docx', tab.id)
          },
          'docx.header': () => openHeaderFooter(editorRef.current, 'header'),
          'docx.footer': () => openHeaderFooter(editorRef.current, 'footer'),
        },
      }),
    [tab.id, comments, commentsChanged],
  )

  // Comments follow their text as the document changes.
  const relocateTimer = useRef(0)
  useEditorEvent('change', () => {
    window.clearTimeout(relocateTimer.current)
    relocateTimer.current = window.setTimeout(() => hostRef.current && comments.relocate(hostRef.current), 300)
  })
  useEffect(() => () => window.clearTimeout(relocateTimer.current), [])

  useEffect(() => {
    if (active) editorRef.current?.focus()
  }, [active, editor])

  useLinks(stageRef)

  // Cropping the selected picture, on the page.
  const [crop, setCrop] = useState<PictureCrop | null>(null)
  const startCrop = () => {
    const next = pictureCrop(editorRef.current)
    if (next) setCrop(next)
  }
  const endCrop = (frame?: Rect, image?: Rect) => {
    const ed = editorRef.current
    setCrop(null)
    if (crop && ed && frame && image) applyCrop(ed, crop, frame, image)
    ed?.focus()
  }

  // Resting the pointer on commented text shows the comment.
  const [peek, setPeek] = useState<{ id: string; at: { left: number; top: number; bottom: number } } | null>(null)
  const peekTimer = useRef(0)
  const hover = (e: React.MouseEvent) => {
    const h = hostRef.current
    window.clearTimeout(peekTimer.current)
    if (!h || !comments.threads.length || e.buttons) return setPeek(null)
    const hit = comments.hitAt(h, e.clientX, e.clientY)
    if (!hit) return setPeek(null)
    if (peek?.id === hit.id) return
    const at = { left: hit.rect.left, top: hit.rect.top, bottom: hit.rect.top + hit.rect.height }
    peekTimer.current = window.setTimeout(() => setPeek({ id: hit.id, at }), peek ? 0 : 350)
  }
  useEffect(() => () => window.clearTimeout(peekTimer.current), [])

  const words = useWordCount(editor)
  const footer = [page && (page.total === 1 ? '1 page' : `${page.total} pages`), words != null && (words === 1 ? '1 word' : `${words.toLocaleString()} words`)].filter(Boolean).join(' · ')

  return (
    <Workbench
      tab={tab}
      left={<Outline />}
      leftFooter={footer ? <span>{footer}</span> : undefined}
      right={<Inspector tab={tab} comments={comments} host={host} onHeaderFooter={(position) => openHeaderFooter(editorRef.current, position)} onCrop={startCrop} />}
    >
      <div
        className={`docx-stage ${crop ? 'cropping' : ''}`}
        ref={stageRef}
        onMouseMove={hover}
        onMouseLeave={() => {
          window.clearTimeout(peekTimer.current)
          setPeek(null)
        }}
        onClick={(e) => {
          // Clicking commented text shows its thread.
          const h = hostRef.current
          if (!h || !comments.threads.length) return
          const id = comments.threadAt(h, e.clientX, e.clientY)
          if (id !== comments.active) comments.focus(h, id)
        }}
      >
        <DocxEditor.Viewport className="docx-viewport">
          <DocxEditor.Content />
        </DocxEditor.Viewport>
      </div>
      {peek && <CommentPeek store={comments} id={peek.id} at={peek.at} />}
      {crop && <CropOverlay src={crop.src} frame={crop.frame} image={crop.image} onDone={endCrop} onCancel={() => endCrop()} />}
      <FindBar
        open={!!findOpen}
        replacing={findOpen === 'replace'}
        onClose={() => {
          setFindOpen(false)
          search.clear()
          editorRef.current?.focus()
        }}
        onQuery={(q) => search.setQuery(q)}
        onNext={() => search.next()}
        onPrev={() => search.previous()}
        count={search.query ? search.matches.length : null}
        current={search.activeIndex >= 0 ? search.activeIndex : 0}
        onReplace={(text) => search.replaceCurrent(text)}
        onReplaceAll={(text) => search.replaceAll(text)}
      />
      <div className="canvas-bar">
        <UndoRedo />
        <span className="sep" />
        {page && page.total > 0 && (
          <>
            <span className="readout">
              {page.current} / {page.total}
            </span>
            <span className="sep" />
          </>
        )}
        <IconButton label="Zoom out (⌘−)" size="sm" onClick={zoom.zoomOut} disabled={!zoom.canZoomOut}>
          <Minus size={14} />
        </IconButton>
        <IconButton label="Fit to window (⌘0)" size="sm" className="readout" aria-label={formatZoom(zoom.zoom)} onClick={zoom.auto}>
          {formatZoom(zoom.zoom)}
        </IconButton>
        <IconButton label="Zoom in (⌘+)" size="sm" onClick={zoom.zoomIn} disabled={!zoom.canZoomIn}>
          <Plus size={14} />
        </IconButton>
      </div>
    </Workbench>
  )
}

function UndoRedo() {
  const undo = useEditorCommand('history.undo')
  const redo = useEditorCommand('history.redo')
  return (
    <>
      <IconButton label="Undo (⌘Z)" size="sm" disabled={!undo.isEnabled} onClick={() => undo.execute()}>
        <Undo2 size={14} />
      </IconButton>
      <IconButton label="Redo (⇧⌘Z)" size="sm" disabled={!redo.isEnabled} onClick={() => redo.execute()}>
        <Redo2 size={14} />
      </IconButton>
    </>
  )
}

/** Run a toggle command such as bold, for a keyboard shortcut. Answers false when it can't run. */
function runSlot(editor: WordEngine | null, slot: Parameters<typeof slotCommand>[1]) {
  const command = slotCommand(editor, slot)
  if (!editor || !command.isEnabled) return false
  command.execute()
}

/** Whether text is selected, kept current as the selection moves. */
function useHasSelectedText() {
  return useWordSelector((e) => e.state.hasSelection, false)
}

/** Edit the header or footer on the page in view, creating it if the page has none. */
function openHeaderFooter(editor: WordEngine | null, position: 'header' | 'footer') {
  if (!editor) return false
  const page = Math.max(0, editor.state.page.current - 1)
  const ok = editor.run<boolean>({ cmd: 'editHeaderFooter', page, which: position })
  editor.focus()
  return ok
}

/** Words in the document body, recounted shortly after each edit. */
function useWordCount(editor: WordEngine | null) {
  const [words, setWords] = useState<number | null>(null)
  const timer = useRef(0)
  const recount = () => {
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => {
      if (editor) setWords(editor.doc.wordCount())
    }, 400)
  }
  useEditorEvent('change', recount)
  useEffect(() => {
    recount()
    return () => window.clearTimeout(timer.current)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editor])
  return words
}

function Outline() {
  const outline = useDocumentOutline()
  if (outline.isEmpty) return <div className="empty-hint">Headings in the document appear here. Use the Heading styles in the inspector to add them.</div>
  return (
    <nav className="list" aria-label="Outline">
      {outline.items.map(({ heading, depth }) => (
        <button
          key={heading.blockId}
          type="button"
          className={`list-item outline-item ${outline.selectedBlockId === heading.blockId ? 'active' : ''}`}
          style={{ paddingLeft: 8 + depth * 14 }}
          onClick={() => outline.goTo(heading.blockId)}
        >
          <span className="label" style={{ fontWeight: depth === 0 ? 550 : 400 }}>
            {heading.text || 'Untitled heading'}
          </span>
        </button>
      ))}
    </nav>
  )
}

/** A toggle bound to one of the editor's command slots. */
function CommandButton({ slot, label, children }: { slot: Parameters<typeof useEditorCommand>[0]; label: string; children: ReactNode }) {
  const cmd = useEditorCommand(slot)
  return (
    <IconButton
      label={label}
      active={cmd.isActive}
      disabled={!cmd.isEnabled}
      onMouseDown={(e) => e.preventDefault() /* keep the caret in the document */}
      onClick={() => cmd.execute()}
    >
      {children}
    </IconButton>
  )
}

const HIGHLIGHTS: { value: string; name: string; color: string }[] = [
  { value: 'yellow', name: 'Yellow', color: '#ffff00' },
  { value: 'green', name: 'Green', color: '#00ff00' },
  { value: 'cyan', name: 'Turquoise', color: '#00ffff' },
  { value: 'magenta', name: 'Pink', color: '#ff00ff' },
  { value: 'red', name: 'Red', color: '#ff4d4d' },
]

function Inspector({
  tab,
  comments,
  host,
  onHeaderFooter,
  onCrop,
}: {
  tab: EditorProps['tab']
  comments: CommentStore
  host: CommentEditor | null
  onHeaderFooter: (position: 'header' | 'footer') => void
  onCrop: () => void
}) {
  const style = useParagraphStyle()
  // Downloaded extras (Michelangelus) join the font menu once they're on disk.
  const installed = useExtraFonts((s) => s.installed)
  const extraFamilies = useMemo(() => EXTRA_FONTS.filter((f) => installed[f.id]).map((f) => f.family), [installed])
  const family = useFontFamily(extraFamilies)
  const size = useEditorValueCommand('font.size')
  const color = useEditorValueCommand('text.color')
  const highlight = useEditorValueCommand('text.highlight')
  const spacing = useEditorValueCommand('list.lineSpacing')
  const alignLeft = useEditorCommand('alignment.left')
  const alignCenter = useEditorCommand('alignment.center')
  const alignRight = useEditorCommand('alignment.right')
  const alignJustify = useEditorCommand('alignment.justify')
  const pageBreak = useEditorCommand('insert.pageBreak')
  const editor = useDocxEditor()
  const [tableOpen, setTableOpen] = useState(false)
  const hasSelection = useHasSelectedText()
  const headerFooter = useHeaderFooterState()

  const alignment = alignCenter.isActive ? 'center' : alignRight.isActive ? 'right' : alignJustify.isActive ? 'justify' : alignLeft.isActive ? 'left' : null
  const alignCommands = { left: alignLeft, center: alignCenter, right: alignRight, justify: alignJustify }

  // Put the most useful styles first, the rest after.
  const styleOptions = useMemo(() => {
    const preferred = ['Normal', 'Title', 'Subtitle', 'Heading 1', 'Heading 2', 'Heading 3', 'Quote']
    const sorted = [...style.options].sort((a, b) => {
      const ia = preferred.indexOf(a.name)
      const ib = preferred.indexOf(b.name)
      return (ia === -1 ? 99 : ia) - (ib === -1 ? 99 : ib) || a.name.localeCompare(b.name)
    })
    return sorted.map((o) => ({ value: o.styleId, label: o.name }))
  }, [style.options])

  return (
    <>
      <InspectorHeader kind="docx">Document</InspectorHeader>
      <Section title="Text">
        <SelectField value={style.value} options={styleOptions} onChange={style.setValue} disabled={!style.isEnabled} placeholder="Style" />
        <div className="docx-font-row">
          <SelectField value={family.value} options={family.options.map((f) => ({ value: f, label: f }))} onChange={family.setValue} disabled={!family.isEnabled} placeholder="Font" />
          <NumberField
            label={<span className="faint">pt</span>}
            // The editor speaks half-points (Word's unit); show points.
            value={size.value != null ? Number(size.value) / 2 : null}
            precision={1}
            step={1}
            min={1}
            max={400}
            disabled={!size.isEnabled}
            onCommit={(v) => size.execute(Math.round(v * 2))}
          />
        </div>
        <div className="toggle-row">
          <CommandButton slot="text.bold" label="Bold (⌘B)">
            <Bold size={15} />
          </CommandButton>
          <CommandButton slot="text.italic" label="Italic (⌘I)">
            <Italic size={15} />
          </CommandButton>
          <CommandButton slot="text.underline" label="Underline (⌘U)">
            <Underline size={15} />
          </CommandButton>
          <CommandButton slot="text.strike" label="Strikethrough">
            <Strikethrough size={15} />
          </CommandButton>
          <CommandButton slot="script.super" label="Superscript">
            <Superscript size={15} />
          </CommandButton>
          <CommandButton slot="script.sub" label="Subscript">
            <Subscript size={15} />
          </CommandButton>
          <CommandButton slot="format.clear" label="Clear formatting">
            <RemoveFormatting size={15} />
          </CommandButton>
        </div>
        <div className="docx-color-row">
          <ColorField value={color.value} onChange={(hex) => color.execute(hex)} disabled={!color.isEnabled} label="Color" />
          <div className="swatches highlight-swatches" role="group" aria-label="Highlight">
            {HIGHLIGHTS.map((h) => (
              <button
                key={h.value}
                type="button"
                aria-label={`${h.name} highlight`}
                aria-pressed={highlight.value === h.value}
                title={highlight.value === h.value ? `${h.name} highlight (click to remove)` : `${h.name} highlight`}
                className={`swatch ${highlight.value === h.value ? 'on' : ''}`}
                style={{ background: h.color }}
                disabled={!highlight.isEnabled}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => highlight.execute(highlight.value === h.value ? 'none' : h.value)}
              />
            ))}
          </div>
        </div>
      </Section>

      <Section title="Paragraph">
        <Segmented
          value={alignment}
          onChange={(v) => alignCommands[v].execute()}
          options={[
            { value: 'left', icon: <AlignLeft size={15} />, title: 'Align left' },
            { value: 'center', icon: <AlignCenter size={15} />, title: 'Center' },
            { value: 'right', icon: <AlignRight size={15} />, title: 'Align right' },
            { value: 'justify', icon: <AlignJustify size={15} />, title: 'Justify' },
          ]}
        />
        <div className="toggle-row">
          <CommandButton slot="list.bullet" label="Bulleted list">
            <List size={15} />
          </CommandButton>
          <CommandButton slot="list.numbered" label="Numbered list">
            <ListOrdered size={15} />
          </CommandButton>
          <CommandButton slot="list.outdent" label="Decrease indent">
            <IndentDecrease size={15} />
          </CommandButton>
          <CommandButton slot="list.indent" label="Increase indent">
            <IndentIncrease size={15} />
          </CommandButton>
          <CommandButton slot="direction.ltr" label="Left-to-right paragraph">
            <PilcrowRight size={15} />
          </CommandButton>
          <CommandButton slot="direction.rtl" label="Right-to-left paragraph">
            <PilcrowLeft size={15} />
          </CommandButton>
        </div>
        <SelectField
          label="Line spacing"
          value={spacing.value != null ? String(spacing.value) : null}
          options={(spacing.options.length ? spacing.options : [1, 1.15, 1.5, 2]).map((v) => ({ value: String(v), label: String(v) }))}
          onChange={(v) => spacing.execute(Number(v))}
          disabled={!spacing.isEnabled}
          // Nothing set on the paragraph itself: it spaces lines as its style says.
          placeholder="From style"
        />
      </Section>

      <Section title="Insert">
        <div className="insert-grid">
          <ToolButton icon={<Table size={15} />} label="Table" onClick={() => setTableOpen(!tableOpen)} aria-expanded={tableOpen} />
          <ImageInsertTrigger className="tool-btn">
            <ImageIcon size={15} />
            <span>Image</span>
          </ImageInsertTrigger>
          <LinkButton />
          <ToolButton icon={<FileStack size={15} />} label="Break" disabled={!pageBreak.isEnabled} onMouseDown={(e) => e.preventDefault()} onClick={() => pageBreak.execute()} />
          <ToolButton
            icon={<PanelTop size={15} />}
            label="Header"
            title={shortcut(withKeys('Header on this page', 'docx.header'))}
            disabled={!editor}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => onHeaderFooter('header')}
          />
          <ToolButton
            icon={<PanelBottom size={15} />}
            label="Footer"
            title={shortcut(withKeys('Footer on this page', 'docx.footer'))}
            disabled={!editor}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => onHeaderFooter('footer')}
          />
        </div>
        {tableOpen && (
          <TableSizePicker
            onPick={(rows, cols) => {
              setTableOpen(false)
              editor?.run({ cmd: 'insertTable', rows, cols })
              editor?.focus()
            }}
          />
        )}
      </Section>

      {headerFooter?.editing && <HeaderFooterSection position={headerFooter.editing} />}

      <ImageSection onCrop={onCrop} />

      <CommentsSection store={comments} editor={host} canAdd={hasSelection} />

      <TableSection />

      <PageSection />
      <Section title="File" collapsible defaultOpen={false}>
        <p className="faint" style={{ margin: 0, fontSize: 12 }}>
          {tab.path ? tab.path.replace(/^\/Users\/[^/]+/, '~') : 'Not saved yet'}
        </p>
      </Section>
    </>
  )
}

/** Opens the link popover for the selection, like ⌘K. */
function LinkButton() {
  const editor = useDocxEditor()
  const { openAtCaret } = useHyperlinkPopup()
  // A link needs words to sit on; the engine would otherwise insert the address as new text.
  const hasSelection = useHasSelectedText()
  const enabled = hasSelection && !!editor
  return (
    <ToolButton
      icon={<Link size={15} />}
      label="Link"
      disabled={!enabled}
      title={enabled ? shortcut(withKeys('Link', 'docx.link')) : 'Select some text to link'}
      onMouseDown={(e) => e.preventDefault()}
      onClick={openAtCaret}
    />
  )
}

/**
 * The link popover. Addresses typed without a scheme ("example.com") get https:// (or mailto:)
 * before they're applied; the engine makes the linked text blue and underlined, as Word's
 * Hyperlink style does.
 */
function LinkPopover() {
  const editor = useDocxEditor()
  const { state, setUrl, commitEdit, close } = useHyperlinkPopup()
  // Closed from its own buttons or keys: put the caret back in the document.
  const refocus = useRef(false)
  const wrapRef = useRef<HTMLDivElement>(null)
  // Put it under the selected words, inside the view.
  const isOpen = state.mode !== 'closed'
  useLayoutEffect(() => {
    const wrap = wrapRef.current
    if (!wrap) return
    delete wrap.dataset.placed
    if (!isOpen) return
    const pop = wrap.querySelector<HTMLElement>('.docx-hyperlink-popup')
    const parent = pop?.offsetParent
    const view = pop?.closest('.docx-viewport') ?? pop?.closest('.docx-stage')
    const target = selectionRect(editor)
    if (!pop || !parent || !view || !target) return
    const box = parent.getBoundingClientRect()
    const area = view.getBoundingClientRect()
    const left = Math.max(area.left + 8, Math.min(target.left, area.right - pop.offsetWidth - 8))
    const below = target.bottom + 6
    const top = below + pop.offsetHeight > area.bottom - 8 && target.top - area.top > pop.offsetHeight + 14 ? target.top - 6 - pop.offsetHeight : below
    wrap.style.setProperty('--link-left', `${left - box.left + (parent as HTMLElement).scrollLeft}px`)
    wrap.style.setProperty('--link-top', `${top - box.top + (parent as HTMLElement).scrollTop}px`)
    wrap.dataset.placed = ''
  }, [isOpen, editor])
  useEffect(() => {
    if (state.mode !== 'closed' || !refocus.current) return
    refocus.current = false
    editor?.focus()
  }, [state.mode, editor])
  const apply = (e: React.SyntheticEvent) => {
    e.preventDefault()
    e.stopPropagation()
    refocus.current = true
    let url = state.url.trim()
    if (url && !url.startsWith('#') && !/^[a-z][a-z0-9+.-]*:/i.test(url)) url = /^[^\s@/]+@[^\s@/]+\.[^\s@/]+$/.test(url) ? `mailto:${url}` : `https://${url}`
    setUrl(url)
    commitEdit(url)
  }
  return (
    <div
      ref={wrapRef}
      className="docx-link-popover"
      onMouseDown={(e) => e.stopPropagation()}
      onClickCapture={(e) => {
        const target = e.target as HTMLElement
        if (target.closest('.docx-hyperlink-popup__apply')) apply(e)
        else if (target.closest('.docx-hyperlink-popup__url') && state.url) {
          // Open it in the browser ourselves.
          e.preventDefault()
          e.stopPropagation()
          void openExternal(state.url)
        } else if (target.closest('button')) refocus.current = true
      }}
      onKeyDownCapture={(e) => {
        if (e.key === 'Enter' && state.mode === 'editing') apply(e)
        else if (e.key === 'Escape') {
          refocus.current = true
          close()
        }
      }}
    >
      <DocxEditor.HyperLink />
    </div>
  )
}

/** Where the selection (or caret) is on screen. */
function selectionRect(editor: WordEngine | null) {
  if (!editor?.toClient) return null
  const o = editor.overlay
  const rects = o.rects
  if (!rects.length) {
    if (!o.caret) return null
    const [page, x, y, h] = o.caret
    const tl = editor.toClient(page, x, y)
    const br = editor.toClient(page, x, y + h)
    return { left: tl.x, top: tl.y, bottom: br.y }
  }
  // Under the last line, from where the selection starts on it.
  const last = rects[rects.length - 1]
  const first = rects[0]
  const start = editor.toClient(last[0], rects.filter((r) => r[0] === last[0] && Math.abs(r[2] - last[2]) < 2).reduce((a, r) => Math.min(a, r[1]), last[1]), last[2])
  const top = editor.toClient(first[0], first[1], first[2])
  const bottom = editor.toClient(last[0], last[1], last[2] + last[4])
  return { left: start.x, top: top.y, bottom: bottom.y }
}

/** Shown while a header or footer is open: page numbers, and the way back to the text. */
function HeaderFooterSection({ position }: { position: 'header' | 'footer' }) {
  const editor = useDocxEditor()
  const run = (command: Record<string, unknown> & { cmd: string }) => {
    editor?.run(command)
    editor?.focus()
  }
  const button = (label: ReactNode, command: Record<string, unknown> & { cmd: string }, variant?: 'primary' | 'danger') => (
    <Button variant={variant} disabled={!editor} onMouseDown={(e) => e.preventDefault()} onClick={() => run(command)}>
      {label}
    </Button>
  )
  return (
    <Section title={position === 'header' ? 'Header' : 'Footer'}>
      <div className="grid-2">
        {button(
          <>
            <Hash size={14} /> Page number
          </>,
          { cmd: 'pageField', field: 'PAGE' },
        )}
        {button('1 of N', { cmd: 'pageField', field: 'PAGE_X_OF_Y' })}
        {button(
          <>
            <Trash2 size={14} /> Remove
          </>,
          { cmd: 'removeHeaderFooter' },
          'danger',
        )}
        {button('Done', { cmd: 'exitHeaderFooter' }, 'primary')}
      </div>
      <p className="faint hint">It repeats on every page. Press Escape to go back to the text.</p>
    </Section>
  )
}

const EMU_PER_INCH = 914400

/** Shown while an image is selected: its size, which dragging a corner of the image also sets, and cropping. */
function ImageSection({ onCrop }: { onCrop: () => void }) {
  const editor = useDocxEditor()
  const image = useEditorState((s) => s.image)
  if (!editor || !image) return null
  const ratio = image.widthEmu ? image.heightEmu / image.widthEmu : 1
  const run = (command: Record<string, unknown> & { cmd: string }) => {
    editor.run(command)
    editor.focus()
  }
  // Stays in the field, so the image stays selected for another change.
  const resize = (widthEmu: number, heightEmu: number) => editor.run({ cmd: 'imageSize', widthEmu: Math.round(widthEmu), heightEmu: Math.round(heightEmu) })
  return (
    <Section title="Image">
      <div className="grid-2">
        <NumberField
          label="W"
          suffix="in"
          value={image.widthEmu / EMU_PER_INCH}
          precision={2}
          step={0.25}
          min={0.1}
          max={50}
          disabled={!image.canResize}
          onCommit={(w) => resize(w * EMU_PER_INCH, w * EMU_PER_INCH * ratio)}
        />
        <NumberField
          label="H"
          suffix="in"
          value={image.heightEmu / EMU_PER_INCH}
          precision={2}
          step={0.25}
          min={0.1}
          max={50}
          disabled={!image.canResize}
          onCommit={(h) => resize((h * EMU_PER_INCH) / ratio, h * EMU_PER_INCH)}
        />
        <Button
          disabled={!image.canCrop || !!image.rotationDegrees}
          title={image.rotationDegrees ? 'A rotated picture can’t be cropped here' : 'Crop'}
          onMouseDown={(e) => e.preventDefault()}
          onClick={onCrop}
        >
          <Crop size={14} /> Crop
        </Button>
        <Button disabled={!image.canResize} onMouseDown={(e) => e.preventDefault()} onClick={() => run({ cmd: 'resetImageSize' })}>
          Original size
        </Button>
      </div>
      <Button variant="danger" block onMouseDown={(e) => e.preventDefault()} onClick={() => run({ cmd: 'delete', word: false })}>
        <Trash2 size={14} /> Delete
      </Button>
      <p className="faint hint">Drag a corner of the image to resize it.</p>
    </Section>
  )
}

const CELL_FILLS = ['FFFFFF', 'F2F2F2', 'D9D9D9', 'DCE6F2', 'DFF0D8', 'FCF3CF', 'F8D7DA', '1F2937']

/** Shown while the caret is in a table: add and remove rows and columns, shading and borders. */
function TableSection() {
  const editor = useDocxEditor()
  const table = useEditorState((s) => s.table)
  if (!editor || !table) return null
  const run = (op: string, color?: string | null) => {
    editor.run({ cmd: 'table', op, color: color ?? null })
    editor.focus()
  }
  const button = (op: string, label: string, icon: ReactNode) => (
    <IconButton label={label} onMouseDown={(e) => e.preventDefault()} onClick={() => run(op)}>
      {icon}
    </IconButton>
  )
  return (
    <Section title={`Table · ${table.rows} × ${table.columns}`}>
      <span className="field-caption">Rows</span>
      <div className="toggle-row">
        {button('insertRowAbove', 'Insert row above', <ArrowUpToLine size={15} />)}
        {button('insertRowBelow', 'Insert row below', <ArrowDownToLine size={15} />)}
        {button('deleteRow', 'Delete row', <Minus size={15} />)}
      </div>
      <span className="field-caption">Columns</span>
      <div className="toggle-row">
        {button('insertColumnLeft', 'Insert column left', <ArrowLeftToLine size={15} />)}
        {button('insertColumnRight', 'Insert column right', <ArrowRightToLine size={15} />)}
        {button('deleteColumn', 'Delete column', <Minus size={15} />)}
      </div>
      <span className="field-caption">Shading and borders (selected cells)</span>
      <div className="swatches">
        {CELL_FILLS.map((c) => (
          <button
            key={c}
            type="button"
            aria-label={`Shade #${c}`}
            className="swatch"
            style={{ background: `#${c}` }}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => run('fill', c === 'FFFFFF' ? null : c)}
          />
        ))}
      </div>
      <div className="grid-3">
        <Button onMouseDown={(e) => e.preventDefault()} onClick={() => run('bordersAll')}>
          All
        </Button>
        <Button onMouseDown={(e) => e.preventDefault()} onClick={() => run('bordersOutside')}>
          Outside
        </Button>
        <Button onMouseDown={(e) => e.preventDefault()} onClick={() => run('bordersNone')}>
          None
        </Button>
      </div>
      <Button variant="danger" block onMouseDown={(e) => e.preventDefault()} onClick={() => run('delete')}>
        <Trash2 size={14} /> Delete table
      </Button>
      <p className="faint hint">Drag a column edge in the table to resize it.</p>
    </Section>
  )
}

const TWIPS_PER_INCH = 1440
const PAPER: Record<string, [number, number]> = {
  letter: [12240, 15840],
  a4: [11906, 16838],
  legal: [12240, 20160],
  a5: [8391, 11906],
}

function PageSection() {
  const { pageSetup, apply, isEnabled } = usePageSetup()
  if (!pageSetup) return null
  const [short, long] = [Math.min(pageSetup.pageWidthTwips, pageSetup.pageHeightTwips), Math.max(pageSetup.pageWidthTwips, pageSetup.pageHeightTwips)]
  const paper = Object.entries(PAPER).find(([, [w, h]]) => Math.abs(w - short) < 40 && Math.abs(h - long) < 40)?.[0] ?? null
  const m = pageSetup.marginsTwips
  const inches = (t: number) => t / TWIPS_PER_INCH

  return (
    <Section title="Page" collapsible>
      <div className="grid-1">
        <SelectField
          value={paper}
          placeholder="Custom"
          disabled={!isEnabled}
          options={[
            { value: 'letter', label: 'US Letter' },
            { value: 'a4', label: 'A4' },
            { value: 'legal', label: 'US Legal' },
            { value: 'a5', label: 'A5' },
          ]}
          onChange={(v) => {
            const [w, h] = PAPER[v]
            const landscape = pageSetup.orientation === 'landscape'
            apply({ pageWidthTwips: landscape ? h : w, pageHeightTwips: landscape ? w : h, scope: 'document' })
          }}
        />
        <Segmented
          value={pageSetup.orientation}
          disabled={!isEnabled}
          onChange={(o) => apply({ orientation: o, pageWidthTwips: pageSetup.pageHeightTwips, pageHeightTwips: pageSetup.pageWidthTwips, scope: 'document' })}
          options={[
            { value: 'portrait', label: 'Portrait' },
            { value: 'landscape', label: 'Landscape' },
          ]}
        />
      </div>
      <span className="field-caption">Margins (inches)</span>
      <div className="grid-2">
        <NumberField label="T" value={inches(m.top)} precision={2} step={0.25} min={0} onCommit={(v) => apply({ marginTopTwips: Math.round(v * TWIPS_PER_INCH), scope: 'document' })} />
        <NumberField label="B" value={inches(m.bottom)} precision={2} step={0.25} min={0} onCommit={(v) => apply({ marginBottomTwips: Math.round(v * TWIPS_PER_INCH), scope: 'document' })} />
        <NumberField label="L" value={inches(m.left)} precision={2} step={0.25} min={0} onCommit={(v) => apply({ marginLeftTwips: Math.round(v * TWIPS_PER_INCH), scope: 'document' })} />
        <NumberField label="R" value={inches(m.right)} precision={2} step={0.25} min={0} onCommit={(v) => apply({ marginRightTwips: Math.round(v * TWIPS_PER_INCH), scope: 'document' })} />
      </div>
    </Section>
  )
}

/** The selected picture on screen: the part that shows, the whole of it, and what it was cropped to. */
interface PictureCrop {
  src: string
  frame: Rect
  image: Rect
  crop: { left: number; top: number; right: number; bottom: number }
  /** EMUs per screen pixel at the current zoom, across and down. */
  scale: { x: number; y: number }
}

function pictureCrop(editor: WordEngine | null): PictureCrop | null {
  const picture = editor?.state.image
  const box = editor?.overlay.image
  if (!editor || !picture?.canCrop || picture.rotationDegrees || !box || !editor.toClient) return null
  const tl = editor.toClient(box[0], box[1], box[2])
  const br = editor.toClient(box[0], box[1] + box[3], box[2] + box[4])
  const frame = { x: tl.x, y: tl.y, width: br.x - tl.x, height: br.y - tl.y }
  const src = editor.media.url(picture.media)
  if (!src || !frame.width || !frame.height) return null
  const { left, top, right, bottom } = picture.crop
  const width = frame.width / Math.max(0.01, 1 - (left + right) / 100)
  const height = frame.height / Math.max(0.01, 1 - (top + bottom) / 100)
  const image = { x: frame.x - (left / 100) * width, y: frame.y - (top / 100) * height, width, height }
  return { src, frame, image, crop: picture.crop, scale: { x: picture.widthEmu / frame.width, y: picture.heightEmu / frame.height } }
}

/** Crop the picture to `frame`, a part of `image` (both on screen), in one undoable step. */
function applyCrop(editor: WordEngine, start: PictureCrop, frame: Rect, image: Rect) {
  const percent = (n: number) => Math.round(Math.min(100, Math.max(0, n * 100)) * 100) / 100
  const crop = {
    left: percent((frame.x - image.x) / image.width),
    top: percent((frame.y - image.y) / image.height),
    right: percent((image.x + image.width - frame.x - frame.width) / image.width),
    bottom: percent((image.y + image.height - frame.y - frame.height) / image.height),
  }
  const sides = ['left', 'top', 'right', 'bottom'] as const
  if (sides.every((side) => Math.abs(crop[side] - start.crop[side]) < 0.01)) return
  editor.run({ cmd: 'cropImage', ...crop, widthEmu: Math.round(frame.width * start.scale.x), heightEmu: Math.round(frame.height * start.scale.y) })
}

/** While ⌘ (Ctrl on Windows) is down, links in the document show a pointing hand: a click opens them. */
function useLinks(stageRef: React.RefObject<HTMLDivElement | null>) {
  useEffect(() => {
    const stage = stageRef.current
    if (!stage) return
    const modifier = (e: KeyboardEvent) => (isMac ? e.metaKey : e.ctrlKey)
    const onKey = (e: KeyboardEvent) => stage.classList.toggle('follow-links', modifier(e))
    const onBlur = () => stage.classList.remove('follow-links')
    window.addEventListener('keydown', onKey)
    window.addEventListener('keyup', onKey)
    window.addEventListener('blur', onBlur)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('keyup', onKey)
      window.removeEventListener('blur', onBlur)
    }
  }, [stageRef])
}

/** Draw every page into the print root at its true size, then open the print panel. */
async function printPages(editor: WordEngine | null) {
  if (!editor) return
  const root = document.getElementById('print-root')!
  root.innerHTML = ''
  const sizes = editor.sizes
  if (!sizes.length) return
  // Match the paper size of the document (layout is at 96 px per inch, as is CSS).
  const style = document.createElement('style')
  style.textContent = `@page { size: ${sizes[0][0]}px ${sizes[0][1]}px; margin: 0; }`
  root.appendChild(style)
  for (let i = 0; i < sizes.length; i++) {
    const p = editor.paint(i, true)!
    const wrap = document.createElement('div')
    wrap.className = 'print-page docx-print-page'
    const page = document.createElement('div')
    page.className = 'docx-page'
    page.style.cssText = `position:relative;width:${p.width}px;height:${p.height}px;box-shadow:none`
    const content = document.createElement('div')
    content.className = 'docx-page-content'
    content.innerHTML = itemsHtml(p.items, editor.styles, editor.media.url)
    page.appendChild(content)
    wrap.appendChild(page)
    root.appendChild(wrap)
  }
  await Promise.all(Array.from(root.querySelectorAll('img')).map((img) => img.decode().catch(() => {})))
  window.print()
}
