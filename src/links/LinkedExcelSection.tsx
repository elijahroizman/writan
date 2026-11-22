import { ExternalLink, FileSpreadsheet, Link2, RefreshCw, Unlink } from 'lucide-react'
import type { ExcelLink, ExcelLinksController } from './excelLinks'
import { findOpenWorkbook } from './excelLinks'

interface LinkedExcelSectionProps {
  controller: ExcelLinksController
  activeTag?: string | null
  onOpenInsert: () => void
  onOpenSource?: (link: ExcelLink) => void
}

function linkLabel(link: ExcelLink): string {
  if (link.kind === 'range') return `${link.sheet}!${link.range ?? 'A1'}`
  return link.chartTitle || `Chart ${(link.chartIndex ?? 0) + 1}`
}

export function LinkedExcelSection({ controller, activeTag, onOpenInsert, onOpenSource }: LinkedExcelSectionProps) {
  const active = controller.getByTag(activeTag)
  const { links, updating } = controller

  return (
    <div className="space-y-2 text-xs" data-testid="linked-excel-section">
      {active && (
        <div className="rounded-md border border-blue-500/30 bg-blue-500/5 p-2 space-y-1.5" data-testid="active-excel-link">
          <div className="flex items-center justify-between gap-1">
            <span className="flex items-center gap-1 font-medium text-blue-600 dark:text-blue-400">
              <FileSpreadsheet size={13} />
              Selected Linked {active.kind === 'range' ? 'Range' : 'Chart'}
            </span>
            <span className="text-[10px] opacity-65">
              {findOpenWorkbook(active) ? 'Live' : active.workbookPath ? 'On disk' : 'Offline'}
            </span>
          </div>
          <div className="truncate font-mono text-[11px] opacity-85" title={`${active.workbookName} · ${linkLabel(active)}`}>
            {active.workbookName} · {linkLabel(active)}
          </div>
          <label className="flex items-center gap-1.5 text-[11px] opacity-85">
            <input
              type="checkbox"
              checked={active.autoUpdate}
              onChange={(e) => controller.setAutoUpdate(active.id, e.target.checked)}
            />
            Auto-update
          </label>
          <div className="flex flex-wrap gap-1 pt-0.5">
            <button
              type="button"
              data-testid="active-excel-link-update"
              disabled={updating}
              onClick={() => void controller.updateLink(active.id)}
              className="inline-flex items-center gap-1 rounded border border-black/15 px-2 py-1 text-[11px] hover:bg-black/5 disabled:opacity-50 dark:border-white/15 dark:hover:bg-white/10"
            >
              <RefreshCw size={11} />
              Update
            </button>
            {onOpenSource && (findOpenWorkbook(active) || active.workbookPath) && (
              <button
                type="button"
                onClick={() => onOpenSource(active)}
                className="inline-flex items-center gap-1 rounded border border-black/15 px-2 py-1 text-[11px] hover:bg-black/5 dark:border-white/15 dark:hover:bg-white/10"
              >
                <ExternalLink size={11} />
                Source
              </button>
            )}
            <button
              type="button"
              data-testid="active-excel-link-unlink"
              onClick={() => controller.unlink(active.id)}
              className="inline-flex items-center gap-1 rounded border border-black/15 px-2 py-1 text-[11px] text-red-600 hover:bg-red-500/10 dark:border-white/15 dark:text-red-400"
            >
              <Unlink size={11} />
              Unlink
            </button>
          </div>
        </div>
      )}

      <div className="flex items-center gap-1.5">
        <button
          type="button"
          data-testid="open-link-excel-modal"
          onClick={onOpenInsert}
          className="inline-flex flex-1 items-center justify-center gap-1 rounded-md border border-black/15 px-2 py-1.5 text-[11px] font-medium hover:bg-black/5 dark:border-white/15 dark:hover:bg-white/10"
        >
          <Link2 size={12} />
          Link from Excel
        </button>
        {links.length > 0 && (
          <button
            type="button"
            data-testid="update-all-excel-links"
            disabled={updating}
            onClick={() => void controller.updateAll()}
            title="Update all linked Excel ranges and charts"
            className="inline-flex items-center gap-1 rounded-md border border-black/15 px-2 py-1.5 text-[11px] hover:bg-black/5 disabled:opacity-50 dark:border-white/15 dark:hover:bg-white/10"
          >
            <RefreshCw size={12} className={updating ? 'animate-spin' : ''} />
            Update all
          </button>
        )}
      </div>

      {links.length > 0 && (
        <div className="space-y-1">
          {links.map((link) => {
            const live = !!findOpenWorkbook(link)
            return (
              <div
                key={link.id}
                className="flex items-center justify-between gap-1 rounded border border-black/10 px-2 py-1 text-[11px] dark:border-white/10"
              >
                <div className="min-w-0 flex-1">
                  <div className="truncate font-medium">
                    {link.workbookName} · {linkLabel(link)}
                  </div>
                  <div className="text-[10px] opacity-60">
                    {link.kind === 'range' ? 'Range' : 'Chart'} · {live ? 'Live' : link.workbookPath ? 'On disk' : 'Offline'}
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-0.5">
                  <button
                    type="button"
                    title="Update now"
                    disabled={updating}
                    onClick={() => void controller.updateLink(link.id)}
                    className="rounded p-1 opacity-75 hover:bg-black/5 hover:opacity-100 dark:hover:bg-white/10"
                  >
                    <RefreshCw size={11} />
                  </button>
                  {onOpenSource && (live || link.workbookPath) && (
                    <button
                      type="button"
                      title="Open source workbook"
                      onClick={() => onOpenSource(link)}
                      className="rounded p-1 opacity-75 hover:bg-black/5 hover:opacity-100 dark:hover:bg-white/10"
                    >
                      <ExternalLink size={11} />
                    </button>
                  )}
                  <button
                    type="button"
                    title="Unlink"
                    onClick={() => controller.unlink(link.id)}
                    className="rounded p-1 opacity-75 hover:bg-red-500/10 hover:text-red-500 hover:opacity-100"
                  >
                    <Unlink size={11} />
                  </button>
                </div>
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}
