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

const smallBtnStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 4,
  padding: '4px 8px',
  borderRadius: 6,
  border: '1px solid var(--line)',
  background: 'var(--field)',
  color: 'var(--fg)',
  fontSize: 11,
  cursor: 'pointer',
}

const iconBtnStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  justifyContent: 'center',
  width: 22,
  height: 22,
  borderRadius: 5,
  border: 'none',
  background: 'transparent',
  color: 'var(--fg-muted)',
  cursor: 'pointer',
}

export function LinkedExcelSection({ controller, activeTag, onOpenInsert, onOpenSource }: LinkedExcelSectionProps) {
  const active = controller.getByTag(activeTag)
  const { links, updating } = controller

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, fontSize: 12 }} data-testid="linked-excel-section">
      {active && (
        <div
          data-testid="active-excel-link"
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 6,
            padding: 8,
            borderRadius: 8,
            border: '1px solid color-mix(in srgb, var(--accent) 40%, transparent)',
            background: 'var(--accent-soft)',
          }}
        >
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 6 }}>
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5, fontWeight: 600, color: 'var(--accent)' }}>
              <FileSpreadsheet size={13} />
              Selected Linked {active.kind === 'range' ? 'Range' : 'Chart'}
            </span>
            <span style={{ fontSize: 10.5, color: 'var(--fg-muted)' }}>
              {findOpenWorkbook(active) ? 'Live' : active.workbookPath ? 'On disk' : 'Offline'}
            </span>
          </div>
          <div
            style={{
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              fontFamily: 'var(--font-mono)',
              fontSize: 11,
              color: 'var(--fg)',
            }}
            title={`${active.workbookName} · ${linkLabel(active)}`}
          >
            {active.workbookName} · {linkLabel(active)}
          </div>
          <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, color: 'var(--fg-muted)', cursor: 'pointer' }}>
            <input
              type="checkbox"
              checked={active.autoUpdate}
              onChange={(e) => controller.setAutoUpdate(active.id, e.target.checked)}
            />
            Auto-update
          </label>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, paddingTop: 2 }}>
            <button
              type="button"
              data-testid="active-excel-link-update"
              disabled={updating}
              onClick={() => void controller.updateLink(active.id)}
              style={{ ...smallBtnStyle, opacity: updating ? 0.5 : 1 }}
            >
              <RefreshCw size={11} />
              Update
            </button>
            {onOpenSource && (findOpenWorkbook(active) || active.workbookPath) && (
              <button
                type="button"
                onClick={() => onOpenSource(active)}
                style={smallBtnStyle}
              >
                <ExternalLink size={11} />
                Source
              </button>
            )}
            <button
              type="button"
              data-testid="active-excel-link-unlink"
              onClick={() => controller.unlink(active.id)}
              style={{ ...smallBtnStyle, color: 'var(--danger)' }}
            >
              <Unlink size={11} />
              Unlink
            </button>
          </div>
        </div>
      )}

      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        <button
          type="button"
          data-testid="open-link-excel-modal"
          onClick={onOpenInsert}
          style={{
            ...smallBtnStyle,
            flex: 1,
            justifyContent: 'center',
            padding: '6px 8px',
            fontWeight: 500,
          }}
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
            style={{
              ...smallBtnStyle,
              padding: '6px 8px',
              opacity: updating ? 0.5 : 1,
            }}
          >
            <RefreshCw size={12} />
            Update all
          </button>
        )}
      </div>

      {links.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          {links.map((link) => {
            const live = !!findOpenWorkbook(link)
            return (
              <div
                key={link.id}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: 6,
                  padding: '6px 8px',
                  borderRadius: 6,
                  border: '1px solid var(--line)',
                  background: 'var(--raised)',
                  fontSize: 11,
                }}
              >
                <div style={{ minWidth: 0, flex: 1 }}>
                  <div
                    style={{
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                      fontWeight: 500,
                      color: 'var(--fg)',
                    }}
                  >
                    {link.workbookName} · {linkLabel(link)}
                  </div>
                  <div style={{ fontSize: 10, color: 'var(--fg-muted)' }}>
                    {link.kind === 'range' ? 'Range' : 'Chart'} · {live ? 'Live' : link.workbookPath ? 'On disk' : 'Offline'}
                  </div>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 2, flexShrink: 0 }}>
                  <button
                    type="button"
                    title="Update now"
                    disabled={updating}
                    onClick={() => void controller.updateLink(link.id)}
                    style={iconBtnStyle}
                  >
                    <RefreshCw size={11} />
                  </button>
                  {onOpenSource && (live || link.workbookPath) && (
                    <button
                      type="button"
                      title="Open source workbook"
                      onClick={() => onOpenSource(link)}
                      style={iconBtnStyle}
                    >
                      <ExternalLink size={11} />
                    </button>
                  )}
                  <button
                    type="button"
                    title="Unlink"
                    onClick={() => controller.unlink(link.id)}
                    style={{ ...iconBtnStyle, color: 'var(--danger)' }}
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
