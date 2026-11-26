import type { CommentEditor as Editor, HighlightRange } from '../../engine/word/comments'
import { Check, MessageSquarePlus, RotateCcw, Trash2 } from 'lucide-react'
import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { Button, Hint, IconButton, Section, TextField } from '../../components/ui'
import { withKeys } from '../../state/shortcuts'
import type { Anchor, Note, Thread } from './commentXml'
import { initialsOf, nearest, newParaId, readComments, writeComments } from './commentXml'

type Paragraphs = readonly { paraId?: string; blockId?: string; text: string }[]

const NAME_KEY = 'workspace.authorName'
export const authorName = () => localStorage.getItem(NAME_KEY) ?? ''

/** Find an anchor's text again after edits: where it was, or nearby when the paragraph changed. */
function locate(a: Anchor, paragraphs: Paragraphs, indexOf: Map<string | undefined, number>): (Anchor & { found: boolean }) | null {
  const i = indexOf.get(a.paraId)
  if (i !== undefined) {
    const at = match(paragraphs[i].text, a)
    if (at) return { ...a, ...at, index: i, found: true }
    if (!a.text) return { ...a, index: i, found: true }
  }
  // Enter split the paragraph, or the text was cut and pasted: look outward from where it was.
  if (a.text.length >= 3) {
    for (let d = 0; d < paragraphs.length; d++) {
      for (const j of d ? [a.index - d, a.index + d] : [a.index]) {
        const p = paragraphs[j]
        if (!p || j === i) continue
        const start = p.text.indexOf(a.text)
        if (start !== -1) return { paraId: p.paraId ?? '', index: j, start, text: a.text, found: true }
      }
    }
  }
  return i === undefined ? null : { ...a, index: i, start: Math.min(a.start, paragraphs[i].text.length), found: false }
}

/** The anchor's text in a paragraph: exact, or between its first and last few characters when words inside it changed. */
function match(text: string, a: Anchor): { start: number; text: string } | null {
  const exact = nearest(text, a.text, a.start)
  if (exact !== -1) return { start: exact, text: a.text }
  if (a.text.length < 6) return null
  const k = Math.min(8, Math.floor(a.text.length / 2))
  const head = nearest(text, a.text.slice(0, k), a.start)
  if (head === -1) return null
  const tail = text.indexOf(a.text.slice(-k), head + 1)
  if (tail === -1 || tail + k - head > a.text.length * 2 + 40) return null
  return { start: head, text: text.slice(head, tail + k) }
}

/**
 * The document's comment threads, kept beside the engine (whose comment commands need its paid
 * review module). Threads follow their text as it moves, paint as highlights, and are written
 * into the file on save.
 */
export class CommentStore {
  threads: Thread[] = []
  /** Ids of comments from the file that were deleted. */
  deleted = new Set<string>()
  /** Text picked for a new comment, waiting for its words. */
  draft: Anchor | null = null
  active: string | null = null
  /** Threads whose text can no longer be found; they keep their place but don't paint. */
  lost = new Set<string>()
  /**
   * Whether comments changed since the file was opened. The engine never learns about new
   * comments, so from then on every save writes them.
   */
  touched = false
  private version = 0
  private editVersion = 0
  private listeners = new Set<() => void>()
  private loading: Promise<void> | null = null

  constructor(private onEdit: () => void) {}

  subscribe = (fn: () => void) => {
    this.listeners.add(fn)
    return () => void this.listeners.delete(fn)
  }
  getVersion = () => this.version
  /** Content edits only. Highlight relocation and selection do not change the saved comments. */
  getEditVersion = () => this.editVersion
  private emit() {
    this.version++
    this.listeners.forEach((fn) => fn())
  }
  private edited(editor: Editor | null) {
    this.touched = true
    this.editVersion++
    this.onEdit()
    if (editor) this.paint(editor)
    this.emit()
  }

  /** Read the threads of a document that was just opened. */
  load(editor: Editor, original: Uint8Array | null) {
    this.threads = []
    this.deleted.clear()
    this.draft = null
    this.touched = false
    this.loading = (async () => {
      if (!original || !editor.hasComments()) return
      // Read from original when paragraph ids already exist; fall back to the engine's save so
      // older files without w14:paraId get matching synthetic ids.
      const paragraphs = editor.query({ type: 'paragraphs' })
      const missingIds = paragraphs.some((p) => !p.paraId || p.paraId.startsWith('#'))
      const bytes = missingIds ? new Uint8Array(await editor.save()) : original
      const loaded = await readComments(bytes)
      // Keep anything posted while the file's comments were still being read.
      this.threads = [...loaded, ...this.threads].sort((a, b) => a.anchor.index - b.anchor.index || a.anchor.start - b.anchor.start)
      this.relocate(editor)
    })().catch((error) => console.error('Could not read comments', error))
    this.emit()
  }

  /** Follow every thread's text after an edit, and repaint. */
  relocate(editor: Editor) {
    if (!this.threads.length && !this.draft) return
    let paragraphs: Paragraphs
    try {
      paragraphs = editor.query({ type: 'paragraphs' })
    } catch {
      return
    }
    const indexOf = new Map(paragraphs.map((p, i) => [p.paraId, i]))
    this.lost.clear()
    for (const t of this.threads) {
      const next = locate(t.anchor, paragraphs, indexOf)
      if (!next?.found) this.lost.add(t.id)
      if (next) t.anchor = { paraId: next.paraId, index: next.index, start: next.start, text: next.text }
    }
    if (this.draft) {
      const next = locate(this.draft, paragraphs, indexOf)
      this.draft = next?.found ? { paraId: next.paraId, index: next.index, start: next.start, text: next.text } : null
    }
    this.paint(editor, paragraphs)
    this.emit()
  }

  /** The engine's address for an anchor's text, for highlighting and selecting it. */
  private matchFor(editor: Editor, a: Anchor) {
    if (!a.text.trim()) return null
    try {
      return editor.findMatches(a.text.slice(0, 200), { matchCase: true }).find((m) => m.paragraphIndex === a.index && m.start === a.start) ?? null
    } catch {
      return null
    }
  }

  paint(editor: Editor, cachedParagraphs?: Paragraphs) {
    const ranges: HighlightRange[] = []
    this.ids = []
    let paragraphs = cachedParagraphs
    const add = (id: string, a: Anchor) => {
      if (!a.text.trim()) return
      if (!paragraphs) {
        try {
          paragraphs = editor.query({ type: 'paragraphs' })
        } catch {
          paragraphs = []
        }
      }
      const p = paragraphs[a.index]
      const blockId = p?.blockId ?? this.matchFor(editor, a)?.blockId
      if (!blockId) return
      this.ids.push(id)
      ranges.push({ blockId, start: a.start, length: a.text.length, expectedText: a.text })
    }
    for (const t of this.threads) if (!t.resolved && !this.lost.has(t.id)) add(t.id, t.anchor)
    if (this.draft) add('draft', this.draft)
    const activeIndex = this.ids.indexOf(this.draft ? 'draft' : (this.active ?? ''))
    try {
      editor.setHighlights('comments', ranges, {
        color: 'rgba(255, 196, 0, 0.3)',
        activeColor: 'rgba(255, 166, 0, 0.55)',
        activeIndex,
        className: 'ws-comment-mark',
        priority: -10,
      })
    } catch (error) {
      console.error('Could not highlight comments', error)
    }
  }
  /** Thread ids in the order of the painted ranges ('draft' for a comment being written). */
  private ids: string[] = []

  /** The thread whose highlighted text is under a point in the document. */
  threadAt(editor: Editor, x: number, y: number) {
    return this.hitAt(editor, x, y)?.id ?? null
  }

  /** The thread under a point, and the highlighted line box there. */
  hitAt(editor: Editor, x: number, y: number) {
    const hit = editor.getHighlightsAt(x, y).find((h) => h.name === 'comments')
    const id = hit ? this.ids[hit.index] : null
    return hit && id && id !== 'draft' ? { id, rect: hit.rect } : null
  }

  /** Start a comment on the selected text. Answers false when nothing is selected. */
  begin(editor: Editor) {
    const selection = editor.query({ type: 'selection' })
    const selected = editor.query({ type: 'selectedText' })
    if (!selection || !selected.trim() || !('paraId' in selection.from)) return false
    const paragraphs = editor.query({ type: 'paragraphs' })
    const index = paragraphs.findIndex((p) => p.paraId === (selection.from as { paraId: string }).paraId)
    if (index === -1) return false
    const paragraph = paragraphs[index].text
    // Across paragraphs, the comment covers the part of the first one.
    const multi = 'paraId' in selection.to && selection.to.paraId !== selection.from.paraId
    const text = multi ? selected.split(/\r?\n/)[0] : selected
    let starts: number[] = []
    for (let i = paragraph.indexOf(text); i !== -1; i = paragraph.indexOf(text, i + 1)) starts.push(i)
    if (multi) starts = starts.filter((s) => s + text.length === paragraph.length)
    if (!starts.length) return false
    const start = starts.length === 1 ? starts[0] : this.pickOccurrence(editor, { paraId: '', index, start: 0, text }, starts)
    this.draft = { paraId: paragraphs[index].paraId ?? '', index, start, text }
    this.active = null
    this.paint(editor)
    this.emit()
    return true
  }

  /** Which of several copies of the selected words is the selected one: the one under the selection's first line. */
  private pickOccurrence(editor: Editor, base: Anchor, starts: number[]) {
    const rect = editor.selectionRect()
    const ranges = starts.flatMap((start) => {
      const m = this.matchFor(editor, { ...base, start })
      return m ? [{ blockId: m.blockId, start, length: base.text.length }] : []
    })
    if (!rect || ranges.length !== starts.length) return starts[0]
    try {
      editor.setHighlights('comment-probe', ranges, { color: 'transparent' })
      const hit = editor.getHighlightsAt(rect.left + Math.min(4, rect.width / 2), rect.top + rect.height / 2).find((h) => h.name === 'comment-probe')
      return hit ? starts[hit.index] : starts[0]
    } finally {
      editor.clearHighlights('comment-probe')
    }
  }

  cancel(editor: Editor | null) {
    this.draft = null
    if (editor) this.paint(editor)
    this.emit()
  }

  private nextId() {
    let max = -1
    for (const t of this.threads) for (const n of [t, ...t.replies]) max = Math.max(max, Number(n.id) || 0)
    for (const id of this.deleted) max = Math.max(max, Number(id) || 0)
    return String(max + 1)
  }

  private note(text: string, author: string): Note {
    const taken = new Set(this.threads.flatMap((t) => [t.paraId, ...t.replies.map((r) => r.paraId)]))
    return { id: this.nextId(), author, date: new Date().toISOString().replace(/\.\d+Z$/, 'Z'), text, paraId: newParaId(taken), inFile: false }
  }

  post(editor: Editor | null, text: string, author: string) {
    if (!this.draft || !text.trim()) return
    const thread: Thread = { ...this.note(text.trim(), author), replies: [], resolved: false, anchor: this.draft }
    this.threads = [...this.threads, thread].sort((a, b) => a.anchor.index - b.anchor.index || a.anchor.start - b.anchor.start)
    this.draft = null
    this.active = thread.id
    this.edited(editor)
  }

  reply(editor: Editor | null, id: string, text: string, author: string) {
    const t = this.threads.find((x) => x.id === id)
    if (!t || !text.trim()) return
    t.replies = [...t.replies, this.note(text.trim(), author)]
    this.threads = [...this.threads]
    this.edited(editor)
  }

  resolve(editor: Editor | null, id: string, resolved: boolean) {
    this.threads = this.threads.map((t) => (t.id === id ? { ...t, resolved } : t))
    this.edited(editor)
  }

  remove(editor: Editor | null, id: string, replyId?: string) {
    const t = this.threads.find((x) => x.id === id)
    if (!t) return
    const gone = replyId ? t.replies.filter((r) => r.id === replyId) : [t, ...t.replies]
    for (const n of gone) if (n.inFile) this.deleted.add(n.id)
    this.threads = replyId ? this.threads.map((x) => (x === t ? { ...t, replies: t.replies.filter((r) => r.id !== replyId) } : x)) : this.threads.filter((x) => x !== t)
    if (this.active === id && !replyId) this.active = null
    this.edited(editor)
  }

  /** Make a thread the current one, and show and select its text. */
  focus(editor: Editor | null, id: string | null, reveal = false) {
    this.active = id
    if (editor) {
      this.paint(editor)
      const t = this.threads.find((x) => x.id === id)
      if (reveal && t) {
        const m = this.matchFor(editor, t.anchor)
        if (m && !t.resolved) editor.selectMatch(m)
        else if (t.anchor.paraId) editor.scrollToAnchor({ paraId: t.anchor.paraId })
      }
    }
    this.emit()
  }

  /** The saved document with the threads written in. */
  async write(editor: Editor, bytes: Uint8Array) {
    await this.loading
    if (!this.touched) return bytes
    this.relocate(editor)
    return writeComments(bytes, this.threads, this.deleted)
  }
}

export function useComments(store: CommentStore) {
  useSyncExternalStore(store.subscribe, store.getVersion)
  return store
}

function when(date: string) {
  const d = new Date(date)
  if (!date || Number.isNaN(d.getTime())) return ''
  const minutes = (Date.now() - d.getTime()) / 60000
  if (minutes < 1) return 'Just now'
  if (minutes < 60) return `${Math.floor(minutes)} min ago`
  if (minutes < 60 * 24 && d.getDate() === new Date().getDate()) return d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })
  return d.toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: d.getFullYear() === new Date().getFullYear() ? undefined : 'numeric' })
}

/** A box to write a comment or reply in. ⌘↩ posts it, Escape cancels. */
function Composer({ placeholder, submitLabel, onSubmit, onCancel, autoFocus }: { placeholder: string; submitLabel: string; onSubmit: (text: string, author: string) => void; onCancel?: () => void; autoFocus?: boolean }) {
  const [text, setText] = useState('')
  const [name, setName] = useState(authorName)
  const needsName = !authorName()
  const submit = () => {
    if (!text.trim() || !name.trim()) return
    localStorage.setItem(NAME_KEY, name.trim())
    onSubmit(text, name.trim())
    setText('')
  }
  return (
    <div className="comment-composer" onClick={(e) => e.stopPropagation()}>
      {needsName && <TextField value={name} placeholder="Your name" onChange={(e) => setName(e.currentTarget.value)} />}
      <textarea
        className="comment-input"
        value={text}
        rows={2}
        placeholder={placeholder}
        autoFocus={autoFocus}
        onChange={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault()
            submit()
          } else if (e.key === 'Escape' && onCancel) {
            e.preventDefault()
            onCancel()
          }
        }}
      />
      <div className="row">
        <Button variant="primary" disabled={!text.trim() || !name.trim()} onClick={submit}>
          {submitLabel}
        </Button>
        {onCancel && <Button onClick={onCancel}>Cancel</Button>}
      </div>
    </div>
  )
}

function NoteHead({ note, children }: { note: Note; children?: React.ReactNode }) {
  return (
    <div className="comment-head">
      <span className="comment-avatar">{initialsOf(note.author)}</span>
      <span className="comment-author" title={note.author}>
        {note.author}
      </span>
      <span className="comment-when">{when(note.date)}</span>
      {children}
    </div>
  )
}

function ThreadCard({ store, editor, thread }: { store: CommentStore; editor: Editor | null; thread: Thread }) {
  const ref = useRef<HTMLDivElement>(null)
  const active = store.active === thread.id
  useEffect(() => {
    if (active) ref.current?.scrollIntoView({ block: 'nearest' })
  }, [active])
  return (
    <div
      ref={ref}
      className={`comment ${active ? 'active' : ''} ${thread.resolved ? 'resolved' : ''}`}
      onClick={() => store.focus(editor, thread.id, true)}
    >
      <NoteHead note={thread}>
        <span className="comment-actions" onClick={(e) => e.stopPropagation()}>
          {thread.resolved ? (
            <IconButton label="Reopen" size="sm" onClick={() => store.resolve(editor, thread.id, false)}>
              <RotateCcw size={13} />
            </IconButton>
          ) : (
            <IconButton label="Resolve" size="sm" onClick={() => store.resolve(editor, thread.id, true)}>
              <Check size={13} />
            </IconButton>
          )}
          <IconButton label="Delete thread" size="sm" onClick={() => store.remove(editor, thread.id)}>
            <Trash2 size={13} />
          </IconButton>
        </span>
      </NoteHead>
      {thread.anchor.text && <div className={`comment-quote ${store.lost.has(thread.id) ? 'lost' : ''}`}>{thread.anchor.text}</div>}
      <div className="comment-text">{thread.text}</div>
      {thread.replies.map((r) => (
        <div key={r.id} className="comment-reply">
          <NoteHead note={r}>
            {active && (
              <span className="comment-actions" onClick={(e) => e.stopPropagation()}>
                <IconButton label="Delete reply" size="sm" onClick={() => store.remove(editor, thread.id, r.id)}>
                  <Trash2 size={13} />
                </IconButton>
              </span>
            )}
          </NoteHead>
          <div className="comment-text">{r.text}</div>
        </div>
      ))}
      {active && !thread.resolved && <Composer placeholder="Reply" submitLabel="Reply" onSubmit={(text, author) => store.reply(editor, thread.id, text, author)} />}
    </div>
  )
}

/** A thread shown over its text while the pointer rests on it; `at` is the highlighted line box. */
export function CommentPeek({ store, id, at }: { store: CommentStore; id: string; at: { left: number; top: number; bottom: number } }) {
  useComments(store)
  const thread = store.threads.find((t) => t.id === id)
  if (!thread) return null
  const replies = thread.replies.length
  // Under the line, or above it near the bottom of the window.
  const below = at.bottom + 180 < window.innerHeight
  return (
    <div
      className="comment-peek"
      role="tooltip"
      style={{ left: Math.max(8, Math.min(at.left, window.innerWidth - 300)), ...(below ? { top: at.bottom + 6 } : { bottom: window.innerHeight - at.top + 6 }) }}
    >
      <NoteHead note={thread} />
      <div className="comment-text">{thread.text}</div>
      {replies > 0 && <div className="comment-peek-more">{replies === 1 ? '1 reply' : `${replies} replies`}</div>}
    </div>
  )
}

/** The inspector's Comments section: add one on the selected text, and every thread in the document. */
export function CommentsSection({ store, editor, canAdd }: { store: CommentStore; editor: Editor | null; canAdd: boolean }) {
  useComments(store)
  const open = store.threads.filter((t) => !t.resolved).length
  return (
    <Section
      title={open ? `Comments · ${open}` : 'Comments'}
      actions={
        <IconButton
          label={withKeys('Add comment', 'docx.comment')}
          size="sm"
          disabled={!canAdd || !editor}
          onMouseDown={(e) => e.preventDefault() /* keep the selection */}
          onClick={() => editor && store.begin(editor)}
        >
          <MessageSquarePlus size={14} />
        </IconButton>
      }
    >
      {store.draft && (
        <div className="comment active">
          <div className="comment-quote">{store.draft.text}</div>
          <Composer
            autoFocus
            placeholder="Add a comment"
            submitLabel="Comment"
            onSubmit={(text, author) => {
              store.post(editor, text, author)
              editor?.focus()
            }}
            onCancel={() => {
              store.cancel(editor)
              editor?.focus()
            }}
          />
        </div>
      )}
      {store.threads.map((t) => (
        <ThreadCard key={t.id} store={store} editor={editor} thread={t} />
      ))}
      {!store.draft && !store.threads.length && <Hint tone="faint" as="p">Select text, then add a comment.</Hint>}
    </Section>
  )
}
