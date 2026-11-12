// Suggestions while typing LaTeX, like VS Code's: `\` lists the commands, `\begin{` and `\end{`
// the environments, and `\ref{` (and the other reference commands) the document's labels.
// Picking a command with arguments puts the cursor in the first; Tab moves to the next.

import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete'
import { acceptCompletion, autocompletion, snippet, startCompletion } from '@codemirror/autocomplete'
import { Prec } from '@codemirror/state'
import type { Extension } from '@codemirror/state'
import type { EditorView } from '@codemirror/view'
import { keymap } from '@codemirror/view'
import type { LatexCommand, LatexEnvironment } from './commands'
import { LATEX_COMMANDS, LATEX_ENVIRONMENTS } from './commands'

/** The arguments as CodeMirror snippet text: each `{}` and `<>` becomes a field. */
export function snippetArgs(args: string) {
  return args.replace(/\{\}/g, '{$${}}').replace(/<>/g, '$${}')
}

/** Commands people reach for most come first when nothing narrows the list. */
const COMMON = new Set(['\\begin', '\\section', '\\subsection', '\\textbf', '\\textit', '\\emph', '\\item', '\\frac', '\\label', '\\ref', '\\cite', '\\usepackage', '\\includegraphics', '\\caption', '\\documentclass'])

function commandOption(c: LatexCommand): Completion {
  // Listed as written, `\frac{}{}`, so what's typed so far shows matched in it.
  const base: Completion = { label: c.name + c.args.replace(/<>/g, ''), boost: COMMON.has(c.name) ? 2 : 0 }
  // \begin opens the environment list in its braces.
  if (c.name === '\\begin') {
    return {
      ...base,
      apply: (view: EditorView, _completion: Completion, from: number, to: number) => {
        view.dispatch({ changes: { from, to, insert: '\\begin{}' }, selection: { anchor: from + 7 }, userEvent: 'input.complete' })
        startCompletion(view)
      },
    }
  }
  if (!c.args) return { ...base, apply: c.name }
  return { ...base, apply: snippet(c.name + snippetArgs(c.args)) }
}

function environmentOption(e: LatexEnvironment, closing: boolean, indent: string): Completion {
  return {
    label: e.name,
    apply: closing
      ? (view, _c, from, to) => {
          const close = view.state.sliceDoc(to, to + 1) === '}' ? 1 : 0
          view.dispatch({ changes: { from, to: to + close, insert: `${e.name}}` }, selection: { anchor: from + e.name.length + 1 }, userEvent: 'input.complete' })
        }
      : (view, _c, from, to) => {
          // The closing brace already there (from \begin{}) is replaced, not doubled.
          const close = view.state.sliceDoc(to, to + 1) === '}' ? 1 : 0
          const inner = indent + '  '
          const lines = (e.body || '').split('\n').map((l) => inner + l)
          let body = lines.join('\n')
          let cursor = body.indexOf('@')
          if (cursor >= 0) body = body.slice(0, cursor) + body.slice(cursor + 1)
          else cursor = inner.length
          const head = `${e.name}}${e.after}\n`
          const insert = `${head}${body}\n${indent}\\end{${e.name}}`
          view.dispatch({ changes: { from, to: to + close, insert }, selection: { anchor: from + head.length + cursor }, userEvent: 'input.complete' })
        },
  }
}

const COMMAND_OPTIONS = LATEX_COMMANDS.map(commandOption)
const KNOWN = new Set(LATEX_COMMANDS.map((c) => c.name))

/** Commands the document defines itself (\newcommand, \def, \DeclareMathOperator). */
function ownCommands(text: string): Completion[] {
  const out = new Map<string, Completion>()
  const re = /\\(?:(?:re)?newcommand\*?|providecommand\*?|DeclareMathOperator\*?|def|let)\s*\{?\s*(\\[a-zA-Z@]+)/g
  for (let m = re.exec(text); m; m = re.exec(text)) {
    const name = m[1]
    if (KNOWN.has(name) || out.has(name)) continue
    out.set(name, { label: name })
  }
  return [...out.values()]
}

const LABEL_COMMANDS = /\\(?:ref|eqref|pageref|autoref|nameref|cref|Cref|vref)\{([^}\s]*)$/
const CITE_COMMANDS = /\\(?:cite|citep|citet|parencite|textcite|autocite|nocite)(?:\[[^\]]*\])*\{(?:[^}]*,)?\s*([^,}\s]*)$/

function latexCompletions(context: CompletionContext): CompletionResult | null {
  const line = context.state.doc.lineAt(context.pos)
  const before = line.text.slice(0, context.pos - line.from)
  // Nothing inside a comment.
  if (/(^|[^\\])(\\\\)*%/.test(before)) return null

  const env = /\\(begin|end)\{([a-zA-Z*]*)$/.exec(before)
  if (env) {
    const indent = /^\s*/.exec(line.text)![0]
    return {
      from: context.pos - env[2].length,
      options: LATEX_ENVIRONMENTS.map((e) => environmentOption(e, env[1] === 'end', indent)),
      validFor: /^[a-zA-Z*]*$/,
    }
  }

  const text = context.state.doc.toString()
  const label = LABEL_COMMANDS.exec(before)
  if (label) {
    const names = new Set<string>()
    for (const m of text.matchAll(/\\label\{([^}]+)\}/g)) names.add(m[1])
    if (!names.size) return null
    return { from: context.pos - label[1].length, options: [...names].map((n) => ({ label: n })), validFor: /^[^}\s]*$/ }
  }
  const cite = CITE_COMMANDS.exec(before)
  if (cite) {
    const keys = new Set<string>()
    for (const m of text.matchAll(/\\bibitem(?:\[[^\]]*\])?\{([^}]+)\}/g)) keys.add(m[1])
    if (!keys.size) return null
    return { from: context.pos - cite[1].length, options: [...keys].map((k) => ({ label: k })), validFor: /^[^,}\s]*$/ }
  }

  const word = /\\[a-zA-Z@]*\*?$/.exec(before)
  if (!word) return null
  const from = context.pos - word[0].length
  // `\\` is a line break, not the start of a command.
  if (/(^|[^\\])(\\\\)+$/.test(before.slice(0, word.index + 1))) return null
  return { from, options: [...COMMAND_OPTIONS, ...ownCommands(text)], validFor: /^\\[a-zA-Z@]*\*?$/ }
}

/** Suggestions as you type, Tab or Enter to pick one, styled like Writan's other lists. */
export function latexAutocomplete(): Extension {
  return [
    autocompletion({
      override: [latexCompletions],
      icons: false,
      tooltipClass: () => 'tex-complete',
      maxRenderedOptions: 400,
    }),
    // Tab picks the highlighted suggestion; with no list open it indents as before.
    Prec.highest(keymap.of([{ key: 'Tab', run: acceptCompletion }])),
  ]
}
