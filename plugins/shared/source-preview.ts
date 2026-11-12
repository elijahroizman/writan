// The source-and-preview layout Markdown has, for plugins: CodeMirror on the left, a preview on
// the right, and Writan's view switch (⌘E) choosing one, the other or both. LaTeX uses
// it; so can any plugin whose files are text that renders into something.
//
// It doesn't know what the preview is. The plugin renders into `preview` and gets told, through
// `onIdle`, when typing has paused long enough to render again.

import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import type { HighlightStyle } from '@codemirror/language'
import { syntaxHighlighting } from '@codemirror/language'
import { openSearchPanel, search, searchKeymap } from '@codemirror/search'
import type { Extension } from '@codemirror/state'
import { EditorState } from '@codemirror/state'
import { drawSelection, EditorView, highlightActiveLine, keymap, lineNumbers, placeholder } from '@codemirror/view'
import type { PluginEditorContext, ViewMode } from '../../src/plugins/api'
import css from './source-preview.css?inline'

export interface SourcePreviewOptions {
  element: HTMLElement
  context: PluginEditorContext
  /** The text a new document starts with. */
  blank: string
  /** CodeMirror language support, if any. */
  language?: Extension
  highlight?: HighlightStyle
  placeholder?: string
  /** Line numbers in the source (code-like files want them; prose doesn't). */
  lineNumbers?: boolean
  /** More CodeMirror extensions for the source, like suggestions while typing. */
  extensions?: Extension
  /** Milliseconds to wait after the last keystroke before onIdle. Read each time. */
  delay(): number
  /** Typing paused (and once at the start): render the preview from `text`. */
  onIdle(text: string): void
}

export interface SourcePreview {
  source: EditorView
  /** The preview pane: the plugin's to fill. */
  preview: HTMLElement
  root: HTMLElement
  view: ViewMode
  text(): string
  setView(view: ViewMode): void
  /** CodeMirror's search panel (find in the source). */
  find(): void
  /** Put the cursor on a line (1-based) and show it. */
  goToLine(line: number, column?: number): void
  /** Render now rather than after the pause. */
  flush(): void
  destroy(): void
}

let styled = false

export function mountSourcePreview(options: SourcePreviewOptions): SourcePreview {
  if (!styled) {
    styled = true
    const style = document.createElement('style')
    style.dataset.workspacePlugin = 'source-preview'
    style.textContent = css
    document.head.appendChild(style)
  }
  const { element, context } = options
  // null is a new document; an existing empty file stays empty.
  const initial = context.bytes ? decode(context.bytes) : options.blank
  let timer = 0
  let rendered: string | null = null

  const root = document.createElement('div')
  root.className = 'sp-split mode-split'
  const sourceHost = document.createElement('div')
  sourceHost.className = 'sp-source'
  const preview = document.createElement('div')
  preview.className = 'sp-preview'
  root.append(sourceHost, preview)
  element.appendChild(root)

  const render = () => {
    window.clearTimeout(timer)
    const text = source.state.doc.toString()
    if (text === rendered) return
    rendered = text
    options.onIdle(text)
  }

  const source = new EditorView({
    parent: sourceHost,
    state: EditorState.create({
      doc: initial,
      extensions: [
        history(),
        drawSelection(),
        highlightActiveLine(),
        options.lineNumbers ? lineNumbers() : [],
        search({ top: true }),
        keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
        options.language ?? [],
        options.highlight ? syntaxHighlighting(options.highlight) : [],
        EditorView.lineWrapping,
        options.placeholder ? placeholder(options.placeholder) : [],
        options.extensions ?? [],
        EditorView.contentAttributes.of({ spellcheck: 'false', autocorrect: 'off', autocapitalize: 'off' }),
        EditorView.updateListener.of((update) => {
          if (!update.docChanged) return
          context.onChange()
          window.clearTimeout(timer)
          timer = window.setTimeout(render, options.delay())
        }),
      ],
    }),
  })

  const api: SourcePreview = {
    source,
    preview,
    root,
    view: 'split',
    text: () => source.state.doc.toString(),
    setView(view) {
      api.view = view
      root.className = `sp-split mode-${view}`
      if (view !== 'read') source.focus()
      // Something typed in source-only view shows when the preview comes back.
      if (view !== 'write') render()
    },
    find: () => {
      if (api.view === 'read') api.setView('split')
      openSearchPanel(source)
    },
    goToLine(line, column = 0) {
      if (api.view === 'read') api.setView('split')
      const target = source.state.doc.line(Math.max(1, Math.min(line, source.state.doc.lines)))
      const at = Math.min(target.from + column, target.to)
      source.dispatch({ selection: { anchor: at }, effects: EditorView.scrollIntoView(at, { y: 'center' }) })
      source.focus()
    },
    flush: render,
    destroy() {
      window.clearTimeout(timer)
      source.destroy()
      root.remove()
    },
  }
  queueMicrotask(render)
  return api
}

export function decode(bytes: Uint8Array) {
  return new TextDecoder('utf-8').decode(bytes).replace(/^﻿/, '')
}

export const encode = (text: string) => new TextEncoder().encode(text)

/**
 * A section in the plugin's sidebar, built from the same classes as Writan's own (a bold
 * title, a divider between sections). Buttons and text go in as they are; pass `list` for rows
 * made with sidebarItem, which sit closer to the edge.
 */
export function sidebarSection(sidebar: HTMLElement, title: string, options: { list?: boolean } = {}): HTMLElement {
  const section = document.createElement('section')
  section.className = 'section'
  const heading = document.createElement('header')
  heading.className = 'section-header'
  const name = document.createElement('span')
  name.className = 'section-title'
  name.textContent = title
  heading.appendChild(name)
  const body = document.createElement('div')
  body.className = options.list ? 'section-body flush sp-list' : 'section-body'
  section.append(heading, body)
  sidebar.appendChild(section)
  return body
}

/** A clickable row in a sidebar list (an outline entry, a problem). */
export function sidebarItem(label: string, options: { indent?: number; detail?: string; tone?: 'error' | 'warning'; onClick(): void }): HTMLButtonElement {
  const item = document.createElement('button')
  item.type = 'button'
  item.className = `list-item sp-side-item${options.tone ? ` tone-${options.tone}` : ''}`
  item.style.paddingLeft = `${8 + (options.indent ?? 0) * 14}px`
  const text = document.createElement('span')
  text.className = 'label'
  text.textContent = label
  // The whole message is the label; a long one wraps instead of being cut off.
  item.title = label
  item.appendChild(text)
  if (options.detail) {
    const detail = document.createElement('span')
    detail.className = 'sp-side-detail'
    detail.textContent = options.detail
    item.appendChild(detail)
  }
  item.addEventListener('click', options.onClick)
  return item
}

/** A line of muted text, for an empty list. */
export function sidebarHint(text: string): HTMLElement {
  const hint = document.createElement('div')
  hint.className = 'empty-hint'
  hint.textContent = text
  return hint
}
