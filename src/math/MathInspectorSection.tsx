import { Pencil, Sigma } from 'lucide-react'
import { Button, Section } from '../components/ui'
import { parseLatexTag } from './latexTag'

export function MathInspectorSection({ mathTag, onEdit }: { mathTag?: string; onEdit: () => void }) {
  const spec = parseLatexTag(mathTag)
  if (!spec) return null

  return (
    <Section title="LaTeX Equation">
      <div
        style={{
          padding: '6px 8px',
          borderRadius: 6,
          border: '1px solid var(--line)',
          background: 'var(--field)',
          fontFamily: 'var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace)',
          fontSize: 11.5,
          color: 'var(--fg)',
          wordBreak: 'break-word',
          maxHeight: 80,
          overflow: 'auto',
          marginBottom: 8,
        }}
      >
        {spec.display ? `$$${spec.latex}$$` : `$${spec.latex}$`}
      </div>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontSize: 11, color: 'var(--muted)', marginBottom: 8 }}>
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
          <Sigma size={12} />
          {spec.display ? 'Display' : 'Inline'} · {spec.fontSizePt} pt
        </span>
      </div>
      <Button block onMouseDown={(e) => e.preventDefault()} onClick={onEdit}>
        <Pencil size={13} /> Edit equation
      </Button>
    </Section>
  )
}
