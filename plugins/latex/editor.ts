// The LaTeX editor: source on the left, the compiled PDF on the right (drawn by Writan's own
// PDF viewer), and the outline, problems and PDF actions in the sidebar. Typing `\` lists the
// commands with what each does (complete.ts). It compiles with the
// system's TeX (context.tex) a few seconds after typing stops (Settings › Plugins sets how many).

import { HighlightStyle, StreamLanguage } from '@codemirror/language'
import { stex } from '@codemirror/legacy-modes/mode/stex'
import { tags } from '@lezer/highlight'
import type { PdfView, PluginEditor, PluginEditorContext, TexStatus } from '../../src/plugins/api'
import { encode, mountSourcePreview, sidebarHint, sidebarItem, sidebarSection } from '../shared/source-preview'
import { latexAutocomplete } from './complete'
import css from './latex.css?inline'
import type { Problem } from './log'
import { parseLog } from './log'
import { guessEngine, magicEngine, outline } from './outline'

const BLANK = `\\documentclass{article}
\\usepackage{amsmath}

\\title{Untitled}
\\author{}
\\date{\\today}

\\begin{document}
\\maketitle

\\section{Introduction}

Write here. The PDF on the right updates a few seconds after you stop typing.

\\begin{equation}
  e^{i\\pi} + 1 = 0
\\end{equation}

\\end{document}
`

const ENGINES: { id: string; label: string }[] = [
  { id: 'pdflatex', label: 'pdfLaTeX' },
  { id: 'xelatex', label: 'XeLaTeX' },
  { id: 'lualatex', label: 'LuaLaTeX' },
]

const highlight = HighlightStyle.define([
  { tag: tags.tagName, color: '#61afef' },
  { tag: tags.keyword, color: '#c678dd' },
  { tag: [tags.bracket, tags.brace], color: 'var(--fg-muted)' },
  { tag: tags.comment, color: 'var(--fg-faint)', fontStyle: 'italic' },
  { tag: [tags.string, tags.special(tags.string)], color: '#98c379' },
  { tag: [tags.atom, tags.number], color: '#d19a66' },
  { tag: [tags.variableName, tags.special(tags.variableName)], color: '#e5c07b' },
  { tag: tags.invalid, color: '#e5484d' },
])

let styled = false

const el = <K extends keyof HTMLElementTagNameMap>(tag: K, className?: string, text?: string) => {
  const node = document.createElement(tag)
  if (className) node.className = className
  if (text) node.textContent = text
  return node
}

export function mountEditor(element: HTMLElement, context: PluginEditorContext): PluginEditor {
  if (!styled) {
    styled = true
    const style = el('style')
    style.dataset.workspacePlugin = 'latex'
    style.textContent = css
    document.head.appendChild(style)
  }
  const tex = context.tex
  let destroyed = false
  let pdf: Uint8Array | null = null
  let compiling = false
  /** Text waiting to compile once the running compile finishes. */
  let queued: string | null = null
  /** Typing happened in source-only view, where nothing compiles; compile when the PDF shows. */
  let stale = false
  let lastCompiled: string | null = null
  let compiledAt = 0
  let problems: Problem[] = []
  /** Failures that aren't in TeX's log (no engine, no PDF made), listed with the problems. */
  let failures: Problem[] = []
  let texStatus: TexStatus | null = null
  /** The PDF has been opened in a tab of its own, which each compile then updates. */
  let pdfTab = false

  const kit = mountSourcePreview({
    element,
    context,
    blank: BLANK,
    language: StreamLanguage.define(stex),
    highlight,
    lineNumbers: true,
    extensions: latexAutocomplete(),
    placeholder: '\\documentclass{article} …',
    delay: () => (context.previewDelay?.() ?? 3) * 1000,
    onIdle: (text) => {
      refreshOutline(text)
      // Source-only view makes no PDF to look at, unless there's one open in its own tab.
      if (kit.view === 'write' && !pdfTab) stale = true
      else void compile(text)
    },
  })

  // ── Preview: Writan's PDF viewer, and a line over it when there's no PDF to show.

  const pdfHost = el('div', 'tex-pdf')
  kit.preview.appendChild(pdfHost)
  const pdfView: PdfView | null = context.createPdfView?.(pdfHost) ?? null
  const empty = el('div', 'tex-empty')
  kit.preview.appendChild(empty)

  function setEmpty(text: string | null) {
    empty.hidden = !text
    empty.textContent = text ?? ''
  }

  // ── Compiling

  const engineFor = (text: string) => {
    const magic = magicEngine(text)
    return magic && ENGINES.some((e) => e.id === magic) ? magic : guessEngine(text)
  }

  async function compile(text: string, force = false) {
    if (destroyed) return
    if (!force && text === lastCompiled) return
    if (compiling) {
      queued = text
      return
    }
    if (!tex) {
      setEmpty('Writan compiles LaTeX in the app on your Mac or PC. This browser version can edit the source but not make the PDF.')
      return
    }
    compiling = true
    stale = false
    failures = []
    updateStatus()
    try {
      // TeX may have been installed since (from Settings, or outside Writan).
      if (force || !texStatus?.installed) texStatus = await tex.status()
      if (!texStatus.installed) {
        setEmpty(pdf ? null : 'LaTeX needs TeX to make the PDF. Install it from the sidebar.')
        return
      }
      const engine = engineFor(text)
      if (!texStatus.engines.includes(engine)) {
        failures = [{ severity: 'error', message: `This document asks for ${engineLabel(engine)}, which your TeX doesn’t have.` }]
        return
      }
      const result = await tex.compile(text, { engine })
      if (destroyed) return
      lastCompiled = text
      compiledAt = Date.now()
      problems = parseLog(result.log, result.file)
      if (result.pdf) {
        pdf = result.pdf
        setEmpty(null)
        // A copy of the PDF in its own tab follows along; it stops once that tab is closed or edited.
        if (pdfTab) pdfTab = context.updateDocument?.({ bytes: result.pdf, name: `${context.title}.pdf` }) ?? false
        await pdfView?.show(result.pdf).catch((err) => failures.push({ severity: 'error', message: `Couldn’t show the PDF: ${err instanceof Error ? err.message : String(err)}` }))
      } else if (!pdf) {
        setEmpty('No PDF yet.')
      }
      if (!result.pdf && !problems.some((p) => p.severity === 'error')) failures.push({ severity: 'error', message: 'TeX didn’t make a PDF, and its log doesn’t say why.' })
    } catch (err) {
      failures = [{ severity: 'error', message: err instanceof Error ? err.message : String(err) }]
    } finally {
      compiling = false
      renderProblems()
      updateStatus()
      if (queued !== null && !destroyed) {
        const next = queued
        queued = null
        void compile(next)
      }
    }
  }

  async function installTex() {
    if (!tex) return
    texStatus = await tex.install()
    updateStatus()
    if (texStatus.installed) void compile(kit.text(), true)
  }

  const engineLabel = (id: string) => ENGINES.find((e) => e.id === id)?.label ?? id

  // ── Sidebar: the PDF's state and actions, problems, and the outline.

  const sidebar = context.sidebar
  const status = el('div', 'tex-status')
  const engineSelect = el('select', 'tex-engine')
  const openButton = el('button', 'btn block', 'Open PDF in a new tab')
  const saveButton = el('button', 'btn block', 'Save PDF as…')
  const compileButton = el('button', 'btn block', 'Compile now')
  const installButton = el('button', 'btn block primary', 'Install TeX…')
  let problemsBody: HTMLElement | null = null
  let outlineBody: HTMLElement | null = null

  if (sidebar) {
    sidebar.classList.add('tex-sidebar')
    const doc = sidebarSection(sidebar, 'PDF')
    const actions = el('div', 'sp-side-actions')
    const engineRow = el('label', 'tex-engine-row')
    engineRow.append(el('span', undefined, 'Engine'), engineSelect)
    for (const button of [openButton, saveButton, compileButton, installButton]) button.type = 'button'
    installButton.hidden = true
    actions.append(status, engineRow, installButton, openButton, saveButton, compileButton)
    doc.appendChild(actions)
    problemsBody = sidebarSection(sidebar, 'Problems', { list: true })
    outlineBody = sidebarSection(sidebar, 'Outline', { list: true })

    engineSelect.addEventListener('change', () => setEngine(engineSelect.value))
    openButton.addEventListener('click', () => {
      if (!pdf || !context.openDocument) return
      context.openDocument({ bytes: pdf, name: `${context.title}.pdf` })
      pdfTab = !!context.updateDocument
    })
    installButton.addEventListener('click', () => void installTex())
    saveButton.addEventListener('click', () => pdf && void context.saveFile?.({ bytes: pdf, name: `${context.title}.pdf` }))
    compileButton.addEventListener('click', () => void compile(kit.text(), true))
    if (!context.openDocument) openButton.hidden = true
    if (!context.saveFile) saveButton.hidden = true
  }

  /** The engine menu: Automatic, or one written into the document as `% !TEX program`. */
  function refreshEngineMenu(text: string) {
    const magic = magicEngine(text)
    const auto = guessEngine(text)
    const options = [{ id: '', label: `Automatic (${engineLabel(auto)})` }, ...ENGINES]
    engineSelect.replaceChildren(
      ...options.map((o) => {
        const option = el('option', undefined, o.label)
        option.value = o.id
        return option
      }),
    )
    engineSelect.value = magic && ENGINES.some((e) => e.id === magic) ? magic : ''
  }

  function setEngine(id: string) {
    const doc = kit.source.state.doc
    const lines = Math.min(doc.lines, 30)
    for (let n = 1; n <= lines; n++) {
      const line = doc.line(n)
      if (/^\s*%\s*!\s*TEX\s+(?:TS-)?program\s*=/i.test(line.text)) {
        // Remove the line with its newline, or rewrite it.
        const change = id ? { from: line.from, to: line.to, insert: `% !TEX program = ${id}` } : { from: line.from, to: Math.min(line.to + 1, doc.length), insert: '' }
        kit.source.dispatch({ changes: change })
        void compile(kit.text())
        return
      }
    }
    if (id) kit.source.dispatch({ changes: { from: 0, insert: `% !TEX program = ${id}\n` } })
    void compile(kit.text())
  }

  function updateStatus() {
    if (!sidebar) return
    openButton.disabled = saveButton.disabled = !pdf
    compileButton.disabled = compiling
    installButton.hidden = !tex || !texStatus || texStatus.installed
    const errors = [...failures, ...problems].filter((p) => p.severity === 'error').length
    status.textContent = compiling
      ? 'Compiling…'
      : !tex
        ? 'Can’t compile in the browser.'
        : texStatus && !texStatus.installed
          ? 'TeX isn’t installed.'
          : compiledAt
            ? `Compiled at ${new Date(compiledAt).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}${errors ? ` · ${errors === 1 ? '1 error' : `${errors} errors`}` : ''}`
            : 'Not compiled yet.'
    status.classList.toggle('busy', compiling)
  }

  function renderProblems() {
    if (!problemsBody) return
    const all = [...failures, ...problems]
    if (!all.length) {
      problemsBody.replaceChildren(sidebarHint(compiledAt ? 'No errors or warnings.' : 'Errors and warnings show here after a compile.'))
      return
    }
    problemsBody.replaceChildren(
      ...all.slice(0, 200).map((p) =>
        sidebarItem(p.message, {
          tone: p.severity,
          detail: p.line ? `Line ${p.line}` : p.file,
          onClick: () => p.line && kit.goToLine(p.line),
        }),
      ),
    )
  }

  let outlineTimer = 0
  function refreshOutline(text: string) {
    window.clearTimeout(outlineTimer)
    refreshEngineMenu(text)
    if (!outlineBody) return
    const headings = outline(text)
    if (!headings.length) {
      outlineBody.replaceChildren(sidebarHint('Sections will appear here as an outline.'))
      return
    }
    const top = Math.min(...headings.map((h) => h.level))
    outlineBody.replaceChildren(...headings.map((h) => sidebarItem(h.title, { indent: h.level - top, onClick: () => kit.goToLine(h.line) })))
  }

  // The outline follows typing sooner than the PDF does.
  kit.source.dom.addEventListener('keyup', () => {
    window.clearTimeout(outlineTimer)
    outlineTimer = window.setTimeout(() => refreshOutline(kit.text()), 400)
  })

  refreshOutline(kit.text())
  renderProblems()
  updateStatus()
  setEmpty(tex ? 'Compiling…' : null)

  return {
    getBytes: async () => encode(kit.text()),
    views: { read: 'PDF', split: 'Source and PDF', write: 'Source' },
    setView(view) {
      kit.setView(view)
      if (view !== 'write' && stale) void compile(kit.text())
    },
    find: () => kit.find(),
    zoom: (direction) => pdfView?.zoom(direction),
    destroy() {
      destroyed = true
      window.clearTimeout(outlineTimer)
      pdfView?.destroy()
      kit.destroy()
      sidebar?.replaceChildren()
    },
  }
}
