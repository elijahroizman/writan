import { FileText } from 'lucide-react'
import { KINDS } from '../lib/kinds'
import { basename, dirname, tildify } from '../lib/platform'
import type { PaletteItem } from '../state/modals'
import { useWorkspace } from '../state/workspace'
import { KindIcon } from './ui'

export function recentItems(): PaletteItem[] {
  const ws = useWorkspace.getState()
  const open = new Set(ws.tabs.map((t) => t.path))
  return ws.recents
    .filter((r) => !open.has(r.path))
    .map((r) => ({
      id: `recent:${r.path}`,
      label: basename(r.path),
      detail: tildify(dirname(r.path)),
      group: 'Recent' as const,
      path: r.path,
      icon: KINDS[r.kind] ? <KindIcon kind={r.kind} /> : <FileText size={14} />,
      run: () => ws.openPaths([r.path]),
    }))
}
