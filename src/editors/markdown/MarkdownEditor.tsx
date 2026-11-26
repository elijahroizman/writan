import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { markdown, markdownLanguage } from '@codemirror/lang-markdown'
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { languages } from '@codemirror/language-data'
import { openSearchPanel, search, searchKeymap } from '@codemirror/search'
import { EditorState } from '@codemirror/state'
import { drawSelection, EditorView, highlightActiveLine, keymap, placeholder } from '@codemirror/view'
import { tags } from '@lezer/highlight'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { EditorProps } from '../../App'
import { FindBar, InspectorHeader, Workbench } from '../../components/Workbench'
import { Button, Section } from '../../components/ui'
import type { ViewMode } from '../../components/ViewSwitch'
import { initialView, nextView, ViewSwitch } from '../../components/ViewSwitch'
import { focusReplace } from '../../lib/codemirror'
import { kindFromPath } from '../../lib/kinds'
import { TextFile } from '../../lib/textcodec'
import * as platform from '../../lib/platform'
import { getEditor, registerEditor, useWorkspace } from '../../state/workspace'
import { schedulePrintCleanup } from '../../lib/print'
import type { Heading } from './render'
import { renderMarkdown } from './render'
import './markdown.css'

const NEW_NOTE = '# Untitled\n\n'

const highlight = HighlightStyle.define([
  { tag: tags.heading1, fontSize: '1.45em', fontWeight: '700' },
  { tag: tags.heading2, fontSize: '1.25em', fontWeight: '700' },
  { tag: tags.heading3, fontSize: '1.1em', fontWeight: '650' },
  { tag: [tags.heading4, tags.heading5, tags.heading6], fontWeight: '650' },
  { tag: tags.strong, fontWeight: '700' },
  { tag: tags.emphasis, fontStyle: 'italic' },
  { tag: tags.strikethrough, textDecoration: 'line-through' },
  { tag: [tags.link, tags.url], color: 'var(--accent)' },
  { tag: tags.monospace, fontFamily: 'var(--font-mono)', color: 'var(--md-code)' },
  { tag: [tags.processingInstruction, tags.meta, tags.contentSeparator], color: 'var(--fg-faint)' },
  { tag: tags.quote, color: 'var(--fg-muted)', fontStyle: 'italic' },
  { tag: [tags.keyword, tags.operator], color: '#c678dd' },
  { tag: [tags.string, tags.special(tags.string)], color: '#98c379' },
  { tag: [tags.number, tags.bool, tags.atom], color: '#d19a66' },
  { tag: tags.comment, color: 'var(--fg-faint)', fontStyle: 'italic' },
  { tag: [tags.function(tags.variableName), tags.propertyName], color: '#61afef' },
])

export default function MarkdownEditor({ tab, active }: EditorProps) {
  const file = useMemo(() => new TextFile(tab.bytes, NEW_NOTE), [tab.bytes])
  const initial = file.text
  const [previewRev, setPreviewRev] = useState(0)
  const [mode, setMode] = useState<ViewMode>(initialView(!!tab.bytes))
  const [rendered, setRendered] = useState(() => renderMarkdown(initial))
  const [activeSlug, setActiveSlug] = useState<string | null>(null)
  const [findOpen, setFindOpen] = useState(false)
  const [findState, setFindState] = useState<{ count: number; current: number } | null>(null)
  const savedText = useRef(initial)
  const viewRef = useRef<EditorView | null>(null)
  const sourceHost = useRef<HTMLDivElement>(null)
  const previewRef = useRef<HTMLDivElement>(null)
  const findQuery = useRef('')
  const markDirty = useWorkspace((s) => s.markDirty)

  // Re-render the preview shortly after typing stops, skipping eager full-string extraction per keystroke.
  useEffect(() => {
    if (previewRev === 0) return
    const src = viewRef.current?.state.doc.toString() ?? initial
    const delay = mode === 'write' ? 600 : src.length > 200_000 ? 300 : 60
    const t = setTimeout(() => setRendered(renderMarkdown(src)), delay)
    return () => clearTimeout(t)
  }, [previewRev, mode, initial])

  const onDocChange = useCallback(
    (doc: EditorView['state']['doc']) => {
      setPreviewRev((r) => r + 1)
      const lenDiffers = doc.length !== savedText.current.length
      markDirty(tab.id, lenDiffers || doc.toString() !== savedText.current)
    },
    [markDirty, tab.id],
  )

  // CodeMirror lives as long as the tab; it's hidden rather than destroyed in preview mode.
  useEffect(() => {
    if (!sourceHost.current) return
    const view = new EditorView({
      parent: sourceHost.current,
      state: EditorState.create({
        doc: initial,
        extensions: [
          history(),
          drawSelection(),
          highlightActiveLine(),
          search({ top: true }),
          keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
          markdown({ base: markdownLanguage, codeLanguages: languages }),
          syntaxHighlighting(highlight),
          EditorView.lineWrapping,
          placeholder('Start writing…'),
          EditorView.contentAttributes.of({ spellcheck: 'true', autocorrect: 'on', autocapitalize: 'sentences' }),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) {
              file.update(u.changes)
              onDocChange(u.state.doc)
            }
          }),
          EditorView.domEventHandlers({
            scroll: () => syncPreviewToSource(),
          }),
        ],
      }),
    })
    viewRef.current = view
    return () => view.destroy()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    if (active && mode !== 'read') viewRef.current?.focus()
  }, [active, mode])

  /** Keep the preview roughly aligned with the editor in split view. */
  const syncPreviewToSource = () => {
    const view = viewRef.current
    const preview = previewRef.current
    if (!view || !preview || preview.offsetParent === null) return
    const top = view.lineBlockAtHeight(view.scrollDOM.scrollTop)
    const line = view.state.doc.lineAt(top.from).number - 1
    const blocks = Array.from(preview.querySelectorAll<HTMLElement>('[data-line]'))
    let target: HTMLElement | undefined
    for (const block of blocks) {
      if (Number(block.dataset.line) <= line) target = block
      else break
    }
    preview.scrollTop = target ? target.offsetTop - 24 : 0
  }

  // Track which heading is on screen to highlight it in the outline.
  useEffect(() => {
    const preview = previewRef.current
    if (!preview || mode === 'write') return
    const onScroll = () => {
      let current: string | null = null
      for (const h of preview.querySelectorAll<HTMLElement>('h1[id],h2[id],h3[id],h4[id],h5[id],h6[id]')) {
        if (h.offsetTop - preview.scrollTop < 80) current = h.id
        else break
      }
      setActiveSlug(current)
    }
    onScroll()
    preview.addEventListener('scroll', onScroll, { passive: true })
    return () => preview.removeEventListener('scroll', onScroll)
  }, [mode, rendered])

  // Relative images resolve against the file's folder.
  useEffect(() => {
    const preview = previewRef.current
    if (!preview || !tab.path || tab.path.startsWith('browser:')) return
    const dir = platform.dirname(tab.path)
    const urls: string[] = []
    let live = true
    for (const img of preview.querySelectorAll('img')) {
      const src = img.getAttribute('src') ?? ''
      if (!src || /^(https?:|data:|blob:)/i.test(src)) continue
      const path = src.startsWith('/') ? src : platform.joinPath(dir, safeDecodeURI(src))
      // Only pictures: a document mustn't make the app read /dev/zero or a huge file.
      if (!IMAGE_EXT.test(path)) {
        img.classList.add('missing')
        continue
      }
      void platform
        .readFile(path)
        .then((bytes) => {
          if (!live) return
          const url = URL.createObjectURL(new Blob([bytes as BlobPart], { type: mimeFor(path) }))
          urls.push(url)
          img.src = url
        })
        .catch(() => img.classList.add('missing'))
    }
    return () => {
      live = false
      urls.forEach(URL.revokeObjectURL)
    }
  }, [rendered, tab.path])

  const onPreviewClick = (e: React.MouseEvent) => {
    const target = e.target as HTMLElement
    const link = target.closest('a')
    if (!link) return
    const href = link.getAttribute('href') ?? ''
    e.preventDefault()
    if (href.startsWith('#')) {
      previewRef.current?.querySelector(`#${CSS.escape(safeDecodeURI(href.slice(1), decodeURIComponent))}`)?.scrollIntoView({ behavior: 'smooth' })
    } else if (/^(https?:|mailto:)/i.test(href)) {
      void platform.openExternal(href)
    } else if (tab.path && kindFromPath(href.split('#')[0])) {
      // Links to other documents open as tabs.
      void useWorkspace.getState().openPaths([platform.joinPath(platform.dirname(tab.path), safeDecodeURI(href.split('#')[0]))])
    }
  }

  const goToHeading = (h: Heading) => {
    if (mode !== 'write') {
      const el = previewRef.current?.querySelector<HTMLElement>(`#${CSS.escape(h.slug)}`)
      if (el && previewRef.current) previewRef.current.scrollTo({ top: el.offsetTop - 20, behavior: 'smooth' })
    }
    const view = viewRef.current
    if (view && mode !== 'read') {
      const line = view.state.doc.line(Math.min(h.line + 1, view.state.doc.lines))
      view.dispatch({ selection: { anchor: line.from }, effects: EditorView.scrollIntoView(line.from, { y: 'start', yMargin: 20 }) })
      view.focus()
    }
  }

  // ── Find in preview (the source editor uses CodeMirror's own search panel).
  const runFind = (query: string, backwards = false) => {
    findQuery.current = query
    const preview = previewRef.current
    if (!preview || !query) {
      setFindState(null)
      return
    }
    const haystack = preview.innerText.toLowerCase()
    const needle = query.toLowerCase()
    let count = 0
    for (let i = haystack.indexOf(needle); i !== -1; i = haystack.indexOf(needle, i + needle.length)) count++
    // window.find is non-standard but supported by WebKit; it selects and scrolls to the match.
    const find = (window as unknown as { find?: (...args: unknown[]) => boolean }).find
    if (count && find) {
      if (!preview.contains(window.getSelection()?.anchorNode ?? null)) {
        const range = document.createRange()
        range.setStart(preview, 0)
        range.collapse(true)
        window.getSelection()?.removeAllRanges()
        window.getSelection()?.addRange(range)
      }
      find(query, false, backwards, true, false, false, false)
    }
    setFindState({ count, current: currentMatchIndex(preview, needle) })
  }

  useEffect(
    () =>
      registerEditor(tab.id, {
        getBytes: async () => file.getBytes(),
        getRevision: () => file.revision,
        markSaved: () => {
          savedText.current = viewRef.current?.state.doc.toString() ?? initial
        },
        viewMode: (next) => setMode((m) => nextView(m, next)),
        find: () => {
          if (mode === 'write' && viewRef.current) openSearchPanel(viewRef.current)
          else setFindOpen(true)
        },
        // CodeMirror's search panel replaces; the preview can't, so show the source beside it.
        replace: () => {
          const view = viewRef.current
          if (!view) return
          if (mode === 'read') setMode('split')
          setFindOpen(false)
          requestAnimationFrame(() => focusReplace(view))
        },
        print: async () => {
          const root = document.getElementById('print-root')!
          // Reuse the live preview so images already resolved from disk come along.
          const html = previewRef.current?.querySelector('article')?.innerHTML ?? renderMarkdown(viewRef.current?.state.doc.toString() ?? initial).html
          root.innerHTML = `<article class="md-body md-print">${html}</article>`
          await waitForImages(root)
          schedulePrintCleanup()
          window.print()
        },
      }),
    [tab.id, mode, file],
  )

  const headings = rendered.headings

  const outline =
    headings.length === 0 ? (
      <div className="empty-hint">Headings will appear here as an outline.</div>
    ) : (
      <nav className="list" aria-label="Outline">
        {headings.map((h, i) => (
          <button
            key={i}
            type="button"
            className={`list-item outline-item ${activeSlug === h.slug ? 'active' : ''}`}
            style={{ paddingLeft: 8 + (h.level - rendered.minHeadingLevel) * 14 }}
            onClick={() => goToHeading(h)}
          >
            <span className="label" style={{ fontWeight: h.level <= 2 ? 550 : 400 }}>
              {h.text || 'Untitled'}
            </span>
          </button>
        ))}
      </nav>
    )

  const inspector = (
    <>
      <InspectorHeader kind="md">Markdown</InspectorHeader>
      <Section title="Export">
        <Button block onClick={() => void useWorkspace.getState().save(tab.id, { saveAs: true })}>
          Save a copy…
        </Button>
        <Button block onClick={() => void runPrint(tab.id)}>
          Print or save as PDF…
        </Button>
      </Section>
    </>
  )

  return (
    <Workbench tab={tab} left={outline} right={inspector}>
      <div className={`md-split mode-${mode}`}>
        <div className="md-source" ref={sourceHost} />
        <div className="md-preview" ref={previewRef} onClick={onPreviewClick}>
          <article className="md-body" dangerouslySetInnerHTML={{ __html: rendered.html }} />
        </div>
      </div>
      <FindBar
        open={findOpen}
        onClose={() => setFindOpen(false)}
        onQuery={(q) => runFind(q)}
        onNext={() => runFind(findQuery.current)}
        onPrev={() => runFind(findQuery.current, true)}
        count={findState?.count ?? null}
        current={findState?.current ?? null}
      />
      <div className="canvas-bar">
        <ViewSwitch mode={mode} setMode={setMode} />
      </div>
    </Workbench>
  )
}

async function runPrint(tabId: string) {
  await getEditor(tabId)?.print?.()
}

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|avif|bmp|ico|svg)$/i

/** decodeURI, but a stray % (“50%.png”) stays as it is instead of throwing. */
function safeDecodeURI(s: string, decode = decodeURI) {
  try {
    return decode(s)
  } catch {
    return s
  }
}

function mimeFor(path: string) {
  const ext = path.split('.').pop()?.toLowerCase()
  return ext === 'svg' ? 'image/svg+xml' : ext === 'jpg' ? 'image/jpeg' : `image/${ext}`
}

function currentMatchIndex(root: HTMLElement, needle: string) {
  const sel = window.getSelection()
  if (!sel || !sel.rangeCount || !root.contains(sel.anchorNode)) return 0
  const range = document.createRange()
  range.setStart(root, 0)
  range.setEnd(sel.getRangeAt(0).startContainer, sel.getRangeAt(0).startOffset)
  const before = range.toString().toLowerCase()
  let n = 0
  for (let i = before.indexOf(needle); i !== -1; i = before.indexOf(needle, i + needle.length)) n++
  return n
}

function waitForImages(root: HTMLElement) {
  return Promise.all(
    Array.from(root.querySelectorAll('img')).map(
      (img) => img.complete || new Promise((r) => ((img.onload = r), (img.onerror = r))),
    ),
  )
}
