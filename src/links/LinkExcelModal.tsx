import { BarChart3, FileSpreadsheet, Link2, RefreshCw, Table2, X } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
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

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      role="dialog"
      aria-modal="true"
      aria-label="Link from Excel"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div className="w-full max-w-lg rounded-xl border border-black/10 bg-[var(--bg-surface,#fff)] text-[var(--ink,#18181b)] shadow-2xl dark:border-white/10">
        <div className="flex items-center justify-between border-b border-black/10 px-4 py-3 dark:border-white/10">
          <div className="flex items-center gap-2 font-medium text-sm">
            <Link2 size={16} />
            <span>Link from Excel into {targetLabel}</span>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="rounded p-1 opacity-70 hover:bg-black/5 hover:opacity-100 dark:hover:bg-white/10"
          >
            <X size={16} />
          </button>
        </div>

        <div className="space-y-3 p-4 text-xs">
          {!workbooks.length ? (
            <div className="rounded-lg border border-dashed border-black/15 p-6 text-center opacity-80 dark:border-white/15">
              <FileSpreadsheet size={24} className="mx-auto mb-2 opacity-60" />
              <div className="font-medium">No Excel workbooks open</div>
              <div className="mt-1 opacity-75">
                Open a spreadsheet tab in Writan first, then pick a cell range or chart here to keep it synced live.
              </div>
            </div>
          ) : (
            <>
              <div className="grid grid-cols-2 gap-2">
                <label className="block">
                  <span className="mb-1 block font-medium opacity-75">Workbook</span>
                  <select
                    data-testid="excel-link-workbook"
                    className="w-full rounded-md border border-black/15 bg-transparent px-2 py-1.5 text-xs dark:border-white/15"
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

                <label className="block">
                  <span className="mb-1 block font-medium opacity-75">Sheet</span>
                  <select
                    data-testid="excel-link-sheet"
                    className="w-full rounded-md border border-black/15 bg-transparent px-2 py-1.5 text-xs dark:border-white/15"
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

              <div className="flex gap-1 rounded-lg bg-black/5 p-1 dark:bg-white/5">
                <button
                  type="button"
                  data-testid="excel-link-kind-range"
                  onClick={() => setKind('range')}
                  className={`flex flex-1 items-center justify-center gap-1.5 rounded-md px-2.5 py-1.5 font-medium transition ${
                    kind === 'range'
                      ? 'bg-[var(--bg-surface,#fff)] shadow-sm'
                      : 'opacity-70 hover:opacity-100'
                  }`}
                >
                  <Table2 size={14} />
                  Cell range
                </button>
                <button
                  type="button"
                  data-testid="excel-link-kind-chart"
                  onClick={() => setKind('chart')}
                  className={`flex flex-1 items-center justify-center gap-1.5 rounded-md px-2.5 py-1.5 font-medium transition ${
                    kind === 'chart'
                      ? 'bg-[var(--bg-surface,#fff)] shadow-sm'
                      : 'opacity-70 hover:opacity-100'
                  }`}
                >
                  <BarChart3 size={14} />
                  Chart ({chartsOnSheet.length})
                </button>
              </div>

              {kind === 'range' ? (
                <div className="space-y-2">
                  <label className="block">
                    <span className="mb-1 block font-medium opacity-75">Range (A1 notation)</span>
                    <input
                      data-testid="excel-link-range"
                      type="text"
                      value={range}
                      onChange={(e) => setRange(e.target.value)}
                      placeholder="A1:C5"
                      className="w-full rounded-md border border-black/15 bg-transparent px-2.5 py-1.5 font-mono text-xs dark:border-white/15"
                    />
                  </label>
                  {rangePreview ? (
                    <div className="max-h-40 overflow-auto rounded-md border border-black/10 p-2 dark:border-white/10">
                      <div className="mb-1 text-[11px] opacity-65">
                        Preview: {rangePreview.rows.length} × {rangePreview.rows[0]?.length ?? 0} cells ({rangePreview.sheet}!{rangePreview.range})
                      </div>
                      <table className="w-full border-collapse text-[11px]">
                        <tbody>
                          {rangePreview.rows.slice(0, 6).map((row, ri) => (
                            <tr key={ri}>
                              {row.slice(0, 6).map((cell, ci) => (
                                <td
                                  key={ci}
                                  className="max-w-[96px] truncate border border-black/10 px-1.5 py-0.5 dark:border-white/10"
                                  style={{
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
                    <div className="text-[11px] text-red-500">Enter a valid A1 range like A1:C4.</div>
                  )}
                </div>
              ) : chartsOnSheet.length === 0 ? (
                <div className="rounded-md border border-dashed border-black/15 p-5 text-center opacity-75 dark:border-white/15">
                  No charts on &ldquo;{selectedWb?.engine.sheets[sheetIdx]?.name}&rdquo;. Insert a chart in the spreadsheet first.
                </div>
              ) : (
                <div className="space-y-2">
                  <select
                    data-testid="excel-link-chart-select"
                    className="w-full rounded-md border border-black/15 bg-transparent px-2 py-1.5 text-xs dark:border-white/15"
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
                    <div className="flex justify-center overflow-hidden rounded-md border border-black/10 bg-white p-2 dark:border-white/10">
                      <ChartSvg chart={chartsOnSheet[chartIndex]} width={420} height={220} />
                    </div>
                  )}
                </div>
              )}

              <label className="flex items-center gap-2 pt-1 text-xs">
                <input
                  type="checkbox"
                  checked={autoUpdate}
                  onChange={(e) => setAutoUpdate(e.target.checked)}
                />
                <RefreshCw size={12} className="opacity-70" />
                <span>Update automatically whenever the workbook changes</span>
              </label>

              {error && <div className="text-xs text-red-500">{error}</div>}
            </>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-black/10 px-4 py-2.5 dark:border-white/10">
          <button
            type="button"
            onClick={onClose}
            className="rounded-md px-3 py-1.5 text-xs hover:bg-black/5 dark:hover:bg-white/10"
          >
            Cancel
          </button>
          <button
            type="button"
            data-testid="excel-link-insert-btn"
            disabled={busy || !selectedWb || (kind === 'range' && !rangePreview) || (kind === 'chart' && chartsOnSheet.length === 0)}
            onClick={() => void handleInsert()}
            className="rounded-md bg-blue-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-blue-700 disabled:opacity-40"
          >
            {busy ? 'Inserting…' : kind === 'range' ? 'Insert Linked Range' : 'Insert Linked Chart'}
          </button>
        </div>
      </div>
    </div>
  )
}
