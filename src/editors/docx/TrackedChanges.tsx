import { Check, X } from 'lucide-react'
import { Button, Hint, Section } from '../../components/ui'
import type { Revision } from '../../engine/word/engine'
import { useDocxEditor, useWordSelector } from '../../engine/word/react'

const NONE: Revision[] = []
const SHOWN = 50

const LABEL: Record<Revision['kind'], string> = {
  insertion: 'Inserted',
  deletion: 'Deleted',
  movedFrom: 'Moved from',
  movedTo: 'Moved to',
  formatting: 'Formatted',
  paragraph: 'Paragraph mark',
  row: 'Table row',
}

/** The body's tracked changes, with Accept all and Reject all. Hidden when there are none. */
export function TrackedChangesSection() {
  const editor = useDocxEditor()
  const changes = useWordSelector((e) => e.revisions(), NONE)
  if (!editor || !changes.length) return null
  const resolve = (accept: boolean) => {
    editor.resolveRevisions(accept)
    editor.focus()
  }
  return (
    <Section title={`Tracked changes · ${changes.length}`} collapsible>
      <div className="row">
        <Button style={{ flex: 1 }} onClick={() => resolve(true)} title="Keep every change and remove its mark">
          <Check size={14} /> Accept all
        </Button>
        <Button style={{ flex: 1 }} onClick={() => resolve(false)} title="Put the original text back">
          <X size={14} /> Reject all
        </Button>
      </div>
      <ul className="revision-list" aria-label="Tracked changes">
        {changes.slice(0, SHOWN).map((r, i) => (
          <li key={i} className={`revision ${r.kind}`}>
            <span className="revision-kind">{LABEL[r.kind]}</span>
            {r.author && <span className="revision-author">{r.author}</span>}
            {r.text && <span className="revision-text">{r.text}</span>}
          </li>
        ))}
      </ul>
      {changes.length > SHOWN && <Hint tone="faint">and {changes.length - SHOWN} more</Hint>}
      <Hint tone="faint" as="p">
        Undo reverses either.
      </Hint>
    </Section>
  )
}
