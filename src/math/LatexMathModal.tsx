import { AlertCircle, Sigma, X } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { isMac } from '../lib/platform'
import type { LatexMathSpec } from './latexMath'
import { LATEX_SNIPPETS, normalizeHexColor, renderLatexToPng, renderLatexToSvg } from './latexMath'

const COLOR_SWATCHES = ['#000000', '#FFFFFF', '#1E40AF', '#DC2626', '#15803D', '#7C3AED']

function isLightColor(hex: string): boolean {
  const clean = hex.replace(/^#/, '')
  if (clean.length !== 6) return false
  const r = parseInt(clean.slice(0, 2), 16)
  const g = parseInt(clean.slice(2, 4), 16)
  const b = parseInt(clean.slice(4, 6), 16)
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255 > 0.72
}

export interface LatexMathModalProps {
  open: boolean
  mode: 'insert' | 'edit'
  targetLabel: 'Word' | 'Slides'
  initialSpec: LatexMathSpec
  onClose: () => void
  onCommit: (result: { spec: LatexMathSpec; bytes: Uint8Array; width: number; height: number }) => void
}

export function LatexMathModal({ open, mode, targetLabel, initialSpec, onClose, onCommit }: LatexMathModalProps) {
  const [latex, setLatex] = useState(initialSpec.latex)
  const [display, setDisplay] = useState(initialSpec.display)
  const [fontSizePt, setFontSizePt] = useState(initialSpec.fontSizePt)
  const [color, setColor] = useState(normalizeHexColor(initialSpec.color))
  const [busy, setBusy] = useState(false)
  const [submitError, setSubmitError] = useState<string | null>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)

  useEffect(() => {
    if (!open) return
    setLatex(initialSpec.latex)
    setDisplay(initialSpec.display)
    setFontSizePt(initialSpec.fontSizePt)
    setColor(normalizeHexColor(initialSpec.color))
    setBusy(false)
    setSubmitError(null)
    requestAnimationFrame(() => {
      const el = inputRef.current
      if (!el) return
      el.focus()
      el.setSelectionRange(el.value.length, el.value.length)
    })
  }, [open, initialSpec])

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onClose()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, onClose])

  const spec: LatexMathSpec = useMemo(
    () => ({
      latex,
      display,
      fontSizePt: Math.max(6, Math.min(144, Number.isFinite(fontSizePt) ? fontSizePt : 12)),
      color: normalizeHexColor(color),
    }),
    [latex, display, fontSizePt, color],
  )

  const preview = useMemo(() => {
    if (!open) return null
    if (!spec.latex.trim()) return { svg: '', width: 0, height: 0 }
    return renderLatexToSvg(spec)
  }, [open, spec])

  if (!open) return null

  const canSubmit = !busy && !!spec.latex.trim() && !!preview?.svg && !preview.error

  const insertSnippet = (snippet: string) => {
    const el = inputRef.current
    if (!el) {
      setLatex((prev) => prev + snippet)
      return
    }
    const start = el.selectionStart ?? latex.length
    const end = el.selectionEnd ?? latex.length
    const next = latex.slice(0, start) + snippet + latex.slice(end)
    setLatex(next)
    requestAnimationFrame(() => {
      el.focus()
      const pos = start + snippet.length
      el.setSelectionRange(pos, pos)
    })
  }

  const handleSubmit = async () => {
    if (!canSubmit) return
    setBusy(true)
    setSubmitError(null)
    try {
      const rendered = await renderLatexToPng(spec)
      onCommit({ spec, ...rendered })
      onClose()
    } catch (err) {
      setSubmitError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  const darkPreviewBg = isLightColor(spec.color)

  const fieldStyle: React.CSSProperties = {
    width: '100%',
    padding: '8px 10px',
    borderRadius: 7,
    border: '1px solid var(--line)',
    background: 'var(--field)',
    color: 'var(--fg)',
    fontSize: 12.5,
    fontFamily: 'var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace)',
    outline: 'none',
    boxSizing: 'border-box',
    resize: 'vertical',
    minHeight: 74,
    lineHeight: 1.45,
  }

  const dialog = (
    <div
      className="modal-backdrop modal-layer-dialog"
      role="dialog"
      aria-modal="true"
      aria-label={mode === 'edit' ? 'Edit LaTeX math' : 'Insert LaTeX math'}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div
        style={{
          width: '100%',
          maxWidth: 520,
          borderRadius: 12,
          border: '1px solid var(--line)',
          background: 'var(--panel)',
          color: 'var(--fg)',
          boxShadow: 'var(--shadow-pop)',
          overflow: 'hidden',
          fontFamily: 'var(--font-ui)',
        }}
      >
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            padding: '12px 16px',
            borderBottom: '1px solid var(--line)',
          }}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13.5, fontWeight: 600 }}>
            <Sigma size={15} />
            <span>{mode === 'edit' ? 'Edit LaTeX equation' : `Insert LaTeX math into ${targetLabel}`}</span>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              justifyContent: 'center',
              width: 26,
              height: 26,
              borderRadius: 6,
              border: 'none',
              background: 'transparent',
              color: 'var(--muted)',
              cursor: 'pointer',
            }}
          >
            <X size={15} />
          </button>
        </div>

        <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 6 }}>
              <label htmlFor="writan-latex-input" style={{ fontSize: 12, fontWeight: 500, color: 'var(--muted)' }}>
                LaTeX expression
              </label>
              <span style={{ fontSize: 11, color: 'var(--faint)' }}>{isMac ? '⌘↩' : 'Ctrl+Enter'} to {mode === 'edit' ? 'update' : 'insert'}</span>
            </div>
            <textarea
              id="writan-latex-input"
              ref={inputRef}
              value={latex}
              placeholder="e.g. \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}"
              spellCheck={false}
              style={fieldStyle}
              onChange={(e) => setLatex(e.target.value)}
              onKeyDown={(e) => {
                if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
                  e.preventDefault()
                  void handleSubmit()
                }
              }}
            />
          </div>

          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 5 }}>
            {LATEX_SNIPPETS.map((s) => (
              <button
                key={s.label}
                type="button"
                title={s.title}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => insertSnippet(s.insert)}
                style={{
                  padding: '3px 8px',
                  borderRadius: 6,
                  border: '1px solid var(--line)',
                  background: 'var(--field)',
                  color: 'var(--fg)',
                  fontSize: 11.5,
                  cursor: 'pointer',
                }}
              >
                {s.label}
              </button>
            ))}
          </div>

          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 12, alignItems: 'center' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              <span style={{ fontSize: 12, color: 'var(--muted)' }}>Style</span>
              <div
                style={{
                  display: 'inline-flex',
                  borderRadius: 7,
                  border: '1px solid var(--line)',
                  background: 'var(--field)',
                  padding: 2,
                }}
              >
                <button
                  type="button"
                  onClick={() => setDisplay(false)}
                  style={{
                    padding: '3px 9px',
                    borderRadius: 5,
                    border: 'none',
                    background: !display ? 'var(--accent)' : 'transparent',
                    color: !display ? '#ffffff' : 'var(--fg)',
                    fontSize: 11.5,
                    fontWeight: 500,
                    cursor: 'pointer',
                  }}
                >
                  Inline ($…$)
                </button>
                <button
                  type="button"
                  onClick={() => setDisplay(true)}
                  style={{
                    padding: '3px 9px',
                    borderRadius: 5,
                    border: 'none',
                    background: display ? 'var(--accent)' : 'transparent',
                    color: display ? '#ffffff' : 'var(--fg)',
                    fontSize: 11.5,
                    fontWeight: 500,
                    cursor: 'pointer',
                  }}
                >
                  Display ($$…$$)
                </button>
              </div>
            </div>

            <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 12, color: 'var(--muted)' }}>
              <span>Size</span>
              <input
                type="number"
                min={6}
                max={144}
                step={1}
                value={fontSizePt}
                onChange={(e) => setFontSizePt(Number(e.target.value) || 12)}
                style={{
                  width: 56,
                  padding: '4px 6px',
                  borderRadius: 6,
                  border: '1px solid var(--line)',
                  background: 'var(--field)',
                  color: 'var(--fg)',
                  fontSize: 12,
                }}
              />
              <span>pt</span>
            </label>

            <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
              <span style={{ fontSize: 12, color: 'var(--muted)' }}>Color</span>
              {COLOR_SWATCHES.map((sw) => (
                <button
                  key={sw}
                  type="button"
                  aria-label={`Color ${sw}`}
                  onClick={() => setColor(sw)}
                  style={{
                    width: 18,
                    height: 18,
                    borderRadius: '50%',
                    border: spec.color === sw ? '2px solid var(--accent)' : '1px solid var(--line)',
                    background: sw,
                    cursor: 'pointer',
                    padding: 0,
                  }}
                />
              ))}
              <input
                type="color"
                aria-label="Custom equation color"
                value={spec.color}
                onChange={(e) => setColor(e.target.value)}
                style={{
                  width: 22,
                  height: 22,
                  border: 'none',
                  background: 'transparent',
                  padding: 0,
                  cursor: 'pointer',
                }}
              />
            </div>
          </div>

          <div
            data-testid="latex-math-preview"
            style={{
              minHeight: 82,
              maxHeight: 200,
              overflow: 'auto',
              borderRadius: 8,
              border: '1px solid var(--line)',
              background: darkPreviewBg ? '#18181b' : '#ffffff',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              padding: 14,
            }}
          >
            {!spec.latex.trim() ? (
              <span style={{ fontSize: 12, color: darkPreviewBg ? '#a1a1aa' : '#71717a' }}>Live equation preview</span>
            ) : preview?.error ? (
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, color: '#dc2626', fontSize: 12 }}>
                <AlertCircle size={14} />
                <span>{preview.error}</span>
              </div>
            ) : preview?.svg ? (
              <div dangerouslySetInnerHTML={{ __html: preview.svg }} />
            ) : null}
          </div>

          {submitError && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 6, color: '#dc2626', fontSize: 12 }}>
              <AlertCircle size={14} />
              <span>{submitError}</span>
            </div>
          )}
        </div>

        <div
          style={{
            display: 'flex',
            justifyContent: 'flex-end',
            gap: 8,
            padding: '10px 16px',
            borderTop: '1px solid var(--line)',
          }}
        >
          <button
            type="button"
            onClick={onClose}
            style={{
              padding: '6px 12px',
              borderRadius: 7,
              border: '1px solid var(--line)',
              background: 'transparent',
              color: 'var(--fg)',
              fontSize: 12.5,
              cursor: 'pointer',
            }}
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={!canSubmit}
            onClick={() => void handleSubmit()}
            style={{
              padding: '6px 14px',
              borderRadius: 7,
              border: 'none',
              background: canSubmit ? 'var(--accent)' : 'var(--line)',
              color: canSubmit ? '#ffffff' : 'var(--muted)',
              fontSize: 12.5,
              fontWeight: 500,
              cursor: canSubmit ? 'pointer' : 'not-allowed',
            }}
          >
            {busy ? 'Rendering…' : mode === 'edit' ? 'Update equation' : 'Insert equation'}
          </button>
        </div>
      </div>
    </div>
  )

  return createPortal(dialog, document.body)
}
