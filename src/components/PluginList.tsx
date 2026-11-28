import type { ReactNode } from 'react'
import { confirmDialog } from '../lib/platform'
import type { PluginInfo } from '../plugins/catalog'
import { PLUGINS } from '../plugins/catalog'
import { formatSize, usePlugins } from '../state/plugins'
import { useTex } from '../state/tex'
import { useWorkspace } from '../state/workspace'
import { DocBadge } from './ui'

/** SPDX ids as words: “MIT, (MPL-2.0 OR Apache-2.0)” reads “Licenses: MIT and (MPL-2.0 or Apache-2.0)”. */
function licenseText(spdx: string) {
  const parts = spdx.split(/,\s*/).map((p) => p.replace(/ OR /g, ' or ').replace(/ AND /g, ' and '))
  // A lone group needs no parentheses; next to another license they keep its meaning.
  if (parts.length === 1) parts[0] = parts[0].replace(/^\((.*)\)$/, '$1')
  return `${parts.length > 1 ? 'Licenses' : 'License'}: ${parts.join(' and ')}`
}

/** One plugin: its file types, what it does, and a control (a switch, a checkbox, “Built in”). */
export function PluginRow({ plugin, control, children }: { plugin: PluginInfo; control: ReactNode; children?: ReactNode }) {
  const progress = usePlugins((s) => s.progress[plugin.id])
  const error = usePlugins((s) => s.errors[plugin.id])
  const meta = [plugin.kinds.map((k) => k.extensions.map((e) => `.${e}`).join(' ')).join(' '), licenseText(plugin.license)]
  if (plugin.source.type === 'download') meta.push(`${formatSize(plugin.source.size)} download`)
  return (
    <div className="plugin-row">
      <span className="plugin-badges">
        {plugin.kinds.map((k) => (
          <DocBadge key={k.id} kind={k.id} size={28} />
        ))}
      </span>
      <span className="plugin-text">
        <span className="settings-label">{plugin.name}</span>
        <span className="settings-hint">{plugin.description}</span>
        <span className="settings-hint plugin-meta">{meta.join(' · ')}</span>
        {progress && (
          <span className="plugin-progress" role="progressbar" aria-valuemin={0} aria-valuemax={progress.total} aria-valuenow={progress.received}>
            <span style={{ width: `${Math.min(100, (100 * progress.received) / Math.max(1, progress.total))}%` }} />
          </span>
        )}
        {error && <span className="plugin-error">{error}</span>}
        {children}
      </span>
      <span className="plugin-control">{control}</span>
    </div>
  )
}

/** Turns a plugin off, from Settings or the onboarding. Tabs of its files have to close first. */
export async function disablePlugin(id: string) {
  const plugin = PLUGINS.find((p) => p.id === id)!
  const kinds = new Set(plugin.kinds.map((k) => k.id))
  const open = useWorkspace.getState().tabs.filter((t) => kinds.has(t.kind))
  if (open.length) {
    const tabs = open.length === 1 ? `“${open[0].title}”` : `${open.length} tabs`
    const ok = await confirmDialog(`${plugin.name} can’t turn off while ${tabs} ${open.length === 1 ? 'is' : 'are'} open. Writan asks before closing anything with unsaved changes.`, {
      title: `Close ${tabs} and turn off ${plugin.name}?`,
      okLabel: open.length === 1 ? 'Close Tab' : 'Close Tabs',
    })
    if (!ok) return
    for (const tab of open) if (!(await useWorkspace.getState().closeTab(tab.id))) return
  }
  // LaTeX uses the computer's own TeX; offer to remove that too (unticked unless they choose).
  if (plugin.services?.includes('tex') && !(await useTex.getState().confirmTurnOff())) return
  await usePlugins.getState().disable(id)
}
