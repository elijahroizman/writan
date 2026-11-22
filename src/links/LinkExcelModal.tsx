import { BarChart3, FileSpreadsheet, Link2, RefreshCw, Table2, X } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { createPortal } from 'react-dom'
import { ChartSvg } from '../engine/sheets/Chart'
import type { ExcelLinksController, OpenWorkbook } from './excelLinks'
import { selectionToA1 } from './excelLinks'

interface LinkExcelModalProps {
  open: boolean
  onClose: () => void
  controller: ExcelLinksController
  targetLabel: 'Word' | 'Slides'
}

export function LinkExcelModal({ open, onClose, controller, targetLabel }: LinkExcelModalProps) {
  const workbooks = controller.openWorkbooks
  const [tabId, setTabId] = useState<string>('')
  const [kind, setKind] = useState<'range' | 'chart'>('range')
  const [sheet, setSheet] = useState<string>('')
  const [range, setRange] = useState<string>('A1:C5')
  const [chartIndex, setChartIndex] = useState<number>(0)
  const [autoUpdate, setAutoUpdate] = useState<boolean>(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const selectedWb: OpenWorkbook | null = useMemo(
    () => workbooks.find((w) => w.tabId === tabId) ?? workbooks[0] ?? null,
    [workbooks, tabId],
  )

  useEffect(() => {
    if (!open || !selectedWb) return
    setTabId(selectedWb.tabId)
    const activeIdx = selectedWb.engine.state.sheet
    const activeSheet = selectedWb.engine.sheets[activeIdx]?.name ?? selectedWb.engine.sheets[0]?.name ?? ''
    setSheet(activeSheet)
    setRange(selectionToA1(selectedWb.engine.state.anchor, selectedWb.engine.state.focus))
    setChartIndex(0)
    setError(null)
  }, [open, selectedWb?.tabId])

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, onClose])

  const sheetIdx = useMemo(() => {
    if (!selectedWb) return 0
    const idx = selectedWb.engine.sheets.findIndex((s) => s.name === sheet)
    return idx >= 0 ? idx : 0
  }, [selectedWb, sheet])

  const chartsOnSheet = useMemo(() => {
    if (!open || !selectedWb) return []
    return selectedWb.engine.chartsOf(sheetIdx)
  }, [open, selectedWb, sheetIdx])

  const rangePreview = useMemo(() => {
    if (!open || !selectedWb || kind !== 'range' || !range.trim()) return null
    return selectedWb.engine.rangeSnapshot(sheetIdx, range.trim())
  }, [open, selectedWb, kind, sheetIdx, range])

  if (!open) return null

  const handleInsert = async () => {
    if (!selectedWb) return
    setBusy(true)
    setError(null)
    try {
      if (kind === 'range') {
        const res = await controller.insertRange({
          workbook: selectedWb,
          sheet: selectedWb.engine.sheets[sheetIdx]?.name ?? sheet,
          range: range.trim() || 'A1',
          autoUpdate,
        })
        if (!res) {
          setError('Could not read that range. Use A1 notation like A1:C5.')
          return
        }
      } else {
        const res = await controller.insertChart({
          workbook: selectedWb,
          sheet: selectedWb.engine.sheets[sheetIdx]?.name ?? sheet,
          chartIndex,
          autoUpdate,
        })
        if (!res) {
          setError('Select a chart from the sheet first.')
          return
        }
      }
      onClose()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  const inputStyle: React.CSSProperties = {
    width: '100%',
    height: 32,
    padding: '0 10px',
    borderRadius: 6,
    border: '1px solid var(--line)',
    background: 'var(--field)',
    color: 'var(--fg)',
    fontSize: 12,
    outline: 'none',
    boxSizing: 'border-box',
  }

  const canInsert = !busy && !!selectedWb && (kind === 'range' ? !!rangePreview : chartsOnSheet.length > 0)

  const dialog = (
    <div
      className="modal-backdrop modal-layer-dialog"
      role="dialog"
      aria-modal="true"
      aria-label="Link from Excel"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div
        style={{
          width: '100%',
          maxWidth: 500,
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
            <Link2 size={15} />
            <span>Link from Excel into {targetLabel}</span>
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
              color: 'var(--fg-muted)',
              cursor: 'pointer',
            }}
          >
            <X size={15} />
          </button>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 12, padding: 16, fontSize: 12 }}>
          {!workbooks.length ? (
            <div
              style={{
                borderRadius: 8,
                border: '1px dashed var(--line)',
                padding: '24px 16px',
                textAlign: 'center',
                color: 'var(--fg-muted)',
              }}
            >
              <FileSpreadsheet size={24} style={{ margin: '0 auto 8px', opacity: 0.7 }} />
              <div style={{ fontWeight: 600, color: 'var(--fg)', marginBottom: 4 }}>No Excel workbooks open</div>
              <div>
                Open a spreadsheet tab in Writan first, then pick a cell range or chart here to keep it synced live.
              </div>
            </div>
          ) : (
            <>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
                <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                  <span style={{ fontWeight: 500, color: 'var(--fg-muted)' }}>Workbook</span>
                  <select
                    data-testid="excel-link-workbook"
                    style={inputStyle}
                    value={selectedWb?.tabId ?? ''}
                    onChange={(e) => {
                      const next = workbooks.find((w) => w.tabId === e.target.value)
                      if (!next) return
                      setTabId(next.tabId)
                      setSheet(next.engine.sheets[next.engine.state.sheet]?.name ?? next.engine.sheets[0]?.name ?? '')
                      setRange(selectionToA1(next.engine.state.anchor, next.engine.state.focus))
                      setChartIndex(0)
                    }}
                  >
                    {workbooks.map((w) => (
                      <option key={w.tabId} value={w.tabId}>
                        {w.name}
                      </option>
                    ))}
                  </select>
                </label>

                <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                  <span style={{ fontWeight: 500, color: 'var(--fg-muted)' }}>Sheet</span>
                  <select
                    data-testid="excel-link-sheet"
                    style={inputStyle}
                    value={selectedWb?.engine.sheets[sheetIdx]?.name ?? ''}
                    onChange={(e) => {
                      setSheet(e.target.value)
                      setChartIndex(0)
                    }}
                  >
                    {(selectedWb?.engine.sheets ?? []).map((s) => (
                      <option key={s.name} value={s.name}>
                        {s.name}
                      </option>
                    ))}
                  </select>
                </label>
              </div>

              <div
                style={{
                  display: 'flex',
                  gap: 4,
                  padding: 3,
                  borderRadius: 8,
                  background: 'var(--field)',
                  border: '1px solid var(--line)',
                }}
              >
                <button
                  type="button"
                  data-testid="excel-link-kind-range"
                  onClick={() => setKind('range')}
                  style={{
                    flex: 1,
                    display: 'inline-flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    gap: 6,
                    padding: '6px 10px',
                    borderRadius: 6,
                    border: 'none',
                    fontSize: 12,
                    fontWeight: 500,
                    cursor: 'pointer',
                    background: kind === 'range' ? 'var(--panel)' : 'transparent',
                    color: kind === 'range' ? 'var(--fg)' : 'var(--fg-muted)',
                    boxShadow: kind === 'range' ? '0 1px 2px rgba(0,0,0,0.12)' : 'none',
                  }}
                >
                  <Table2 size={14} />
                  Cell range
                </button>
                <button
                  type="button"
                  data-testid="excel-link-kind-chart"
                  onClick={() => setKind('chart')}
                  style={{
                    flex: 1,
                    display: 'inline-flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    gap: 6,
                    padding: '6px 10px',
                    borderRadius: 6,
                    border: 'none',
                    fontSize: 12,
                    fontWeight: 500,
                    cursor: 'pointer',
                    background: kind === 'chart' ? 'var(--panel)' : 'transparent',
                    color: kind === 'chart' ? 'var(--fg)' : 'var(--fg-muted)',
                    boxShadow: kind === 'chart' ? '0 1px 2px rgba(0,0,0,0.12)' : 'none',
                  }}
                >
                  <BarChart3 size={14} />
                  Chart ({chartsOnSheet.length})
                </button>
              </div>

              {kind === 'range' ? (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                  <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                    <span style={{ fontWeight: 500, color: 'var(--fg-muted)' }}>Range (A1 notation)</span>
                    <input
                      data-testid="excel-link-range"
                      type="text"
                      value={range}
                      onChange={(e) => setRange(e.target.value)}
                      placeholder="A1:C5"
                      style={{ ...inputStyle, fontFamily: 'var(--font-mono)' }}
                    />
                  </label>
                  {rangePreview ? (
                    <div
                      style={{
                        maxHeight: 160,
                        overflow: 'auto',
                        borderRadius: 6,
                        border: '1px solid var(--line)',
                        padding: 8,
                        background: 'var(--raised)',
                      }}
                    >
                      <div style={{ marginBottom: 6, fontSize: 11, color: 'var(--fg-muted)' }}>
                        Preview: {rangePreview.rows.length} × {rangePreview.rows[0]?.length ?? 0} cells ({rangePreview.sheet}!{rangePreview.range})
                      </div>
                      <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 11 }}>
                        <tbody>
                          {rangePreview.rows.slice(0, 6).map((row, ri) => (
                            <tr key={ri}>
                              {row.slice(0, 6).map((cell, ci) => (
                                <td
                                  key={ci}
                                  style={{
                                    maxWidth: 96,
                                    overflow: 'hidden',
                                    textOverflow: 'ellipsis',
                                    whiteSpace: 'nowrap',
                                    border: '1px solid var(--line)',
                                    padding: '3px 6px',
                                    fontWeight: cell.bold ? 600 : undefined,
                                    fontStyle: cell.italic ? 'italic' : undefined,
                                  }}
                                >
                                  {cell.text || '\u00a0'}
                                </td>
                              ))}
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  ) : (
                    <div style={{ fontSize: 11, color: 'var(--danger)' }}>Enter a valid A1 range like A1:C4.</div>
                  )}
                </div>
              ) : chartsOnSheet.length === 0 ? (
                <div
                  style={{
                    borderRadius: 6,
                    border: '1px dashed var(--line)',
                    padding: '20px 14px',
                    textAlign: 'center',
                    color: 'var(--fg-muted)',
                  }}
                >
                  No charts on &ldquo;{selectedWb?.engine.sheets[sheetIdx]?.name}&rdquo;. Insert a chart in the spreadsheet first.
                </div>
              ) : (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                  <select
                    data-testid="excel-link-chart-select"
                    style={inputStyle}
                    value={chartIndex}
                    onChange={(e) => setChartIndex(Number(e.target.value))}
                  >
                    {chartsOnSheet.map((c, idx) => (
                      <option key={idx} value={idx}>
                        {c.title || `Chart ${idx + 1}`} ({c.kind})
                      </option>
                    ))}
                  </select>
                  {chartsOnSheet[chartIndex] && (
                    <div
                      style={{
                        display: 'flex',
                        justifyContent: 'center',
                        overflow: 'hidden',
                        borderRadius: 6,
                        border: '1px solid var(--line)',
                        background: '#ffffff',
                        padding: 8,
                      }}
                    >
                      <ChartSvg chart={chartsOnSheet[chartIndex]} width={420} height={210} />
                    </div>
                  )}
                </div>
              )}

              <label style={{ display: 'flex', alignItems: 'center', gap: 8, paddingTop: 2, fontSize: 12, cursor: 'pointer' }}>
                <input
                  type="checkbox"
                  checked={autoUpdate}
                  onChange={(e) => setAutoUpdate(e.target.checked)}
                />
                <RefreshCw size={12} style={{ color: 'var(--fg-muted)' }} />
                <span>Update automatically whenever the workbook changes</span>
              </label>

              {error && <div style={{ fontSize: 12, color: 'var(--danger)' }}>{error}</div>}
            </>
          )}
        </div>

        <div
          style={{
            display: 'flex',
            alignItems: 'center',
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
              borderRadius: 6,
              border: '1px solid var(--line)',
              background: 'transparent',
              color: 'var(--fg)',
              fontSize: 12,
              cursor: 'pointer',
            }}
          >
            Cancel
          </button>
          <button
            type="button"
            data-testid="excel-link-insert-btn"
            disabled={!canInsert}
            onClick={() => void handleInsert()}
            style={{
              padding: '6px 14px',
              borderRadius: 6,
              border: 'none',
              background: 'var(--accent)',
              color: 'var(--accent-fg)',
              fontSize: 12,
              fontWeight: 500,
              cursor: canInsert ? 'pointer' : 'not-allowed',
              opacity: canInsert ? 1 : 0.45,
            }}
          >
            {busy ? 'Inserting…' : kind === 'range' ? 'Insert Linked Range' : 'Insert Linked Chart'}
          </button>
        </div>
      </div>
    </div>
  )

  return createPortal(dialog, document.body)
}
