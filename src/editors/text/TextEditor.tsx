import { defaultKeymap, history, historyKeymap, indentWithTab, redo, undo } from '@codemirror/commands'
import { openSearchPanel, search, searchKeymap } from '@codemirror/search'
import { EditorState } from '@codemirror/state'
import { drawSelection, EditorView, highlightActiveLine, keymap, placeholder } from '@codemirror/view'
import { Redo2, Undo2 } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import type { EditorProps } from '../../App'
import { Workbench } from '../../components/Workbench'
import { IconButton } from '../../components/ui'
import { focusReplace } from '../../lib/codemirror'
import { TextFile } from '../../lib/textcodec'
import { registerEditor, useWorkspace } from '../../state/workspace'
import { documentReady } from '../../lib/analytics/documents'
import { schedulePrintCleanup } from '../../lib/print'
import './text.css'

/** Plain .txt files: a wrapping text editor with nothing else around it. */
export default function TextEditor({ tab, active }: EditorProps) {
  const file = useMemo(() => new TextFile(tab.bytes), [tab.bytes])
  const initial = file.text
  const savedText = useRef(initial)
  const viewRef = useRef<EditorView | null>(null)
  const host = useRef<HTMLDivElement>(null)
  const markDirty = useWorkspace((s) => s.markDirty)
  const [cursor, setCursor] = useState({ line: 1, col: 1 })

  useEffect(() => {
    if (!host.current) return
    const view = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc: initial,
        extensions: [
          history(),
          drawSelection(),
          highlightActiveLine(),
          search({ top: true }),
          keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
          EditorView.lineWrapping,
          placeholder('Start typing…'),
          EditorView.contentAttributes.of({ spellcheck: 'true', autocorrect: 'on', autocapitalize: 'sentences' }),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) {
              file.update(u.changes)
              const lenDiffers = u.state.doc.length !== savedText.current.length
              markDirty(tab.id, lenDiffers || u.state.doc.toString() !== savedText.current)
            }
            if (u.docChanged || u.selectionSet) {
              const head = u.state.selection.main.head
              const line = u.state.doc.lineAt(head)
              setCursor({ line: line.number, col: head - line.from + 1 })
            }
          }),
        ],
      }),
    })
    viewRef.current = view
    documentReady(tab.id)
    return () => view.destroy()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    if (active) viewRef.current?.focus()
  }, [active])

  useEffect(
    () =>
      registerEditor(tab.id, {
        getBytes: async () => file.getBytes(),
        getRevision: () => file.revision,
        markSaved: () => {
          savedText.current = currentText(viewRef.current)
        },
        find: () => viewRef.current && openSearchPanel(viewRef.current),
        replace: () => viewRef.current && focusReplace(viewRef.current),
        print: async () => {
          const root = document.getElementById('print-root')!
          root.innerHTML = ''
          const pre = document.createElement('pre')
          pre.className = 'txt-print'
          pre.textContent = currentText(viewRef.current)
          root.appendChild(pre)
          schedulePrintCleanup()
          window.print()
        },
      }),
    [tab.id, file],
  )

  const run = (command: (view: EditorView) => boolean) => {
    const view = viewRef.current
    if (!view) return
    command(view)
    view.focus()
  }

  return (
    <Workbench tab={tab}>
      <div className="txt-editor" ref={host} />
      <div className="canvas-bar">
        <IconButton label="Undo (⌘Z)" size="sm" onMouseDown={(e) => e.preventDefault()} onClick={() => run(undo)}>
          <Undo2 size={14} />
        </IconButton>
        <IconButton label="Redo (⇧⌘Z)" size="sm" onMouseDown={(e) => e.preventDefault()} onClick={() => run(redo)}>
          <Redo2 size={14} />
        </IconButton>
        <span className="sep" />
        <span className="readout">
          Line {cursor.line}, column {cursor.col}
        </span>
      </div>
    </Workbench>
  )
}

function currentText(view: EditorView | null) {
  return view?.state.doc.toString() ?? ''
}
