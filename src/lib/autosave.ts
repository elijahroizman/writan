// Autosave: a few seconds after you stop editing, a file that already has a location is saved
// there. A document that has never been saved is left alone until you save it yourself.

import { create } from 'zustand'
import { KINDS } from './kinds'
import type { AutosaveSettings } from '../state/settings'
import { useSettings } from '../state/settings'
import type { DocTab } from '../state/workspace'
import { autosaveHooks, getEditor, useWorkspace } from '../state/workspace'

/** Keep typing and it still saves at least this often. */
const MAX_WAIT = 30_000

export interface AutosaveStatus {
  state: 'saving' | 'saved' | 'error'
  message?: string
}

/** The last autosave result for each tab, shown in the title bar. */
export const useAutosaveStatus = create<Record<string, AutosaveStatus>>(() => ({}))

const setStatus = (tabId: string, status: AutosaveStatus | null) =>
  useAutosaveStatus.setState((all) => {
    const next = { ...all }
    if (status) next[tabId] = status
    else delete next[tabId]
    return next
  })

/** Why a tab isn't autosaved, or null if it is. */
export function autosaveBlocker(tab: DocTab, s: AutosaveSettings = useSettings.getState().autosave): string | null {
  if (!s.enabled) return 'Autosave is off.'
  if (s.kinds[tab.kind] === false) return `Autosave is off for ${KINDS[tab.kind].short.toLowerCase()} files.`
  if (!tab.path) return 'Save this file once. After that, changes autosave.'
  if (tab.path.startsWith('browser:')) return 'Only files saved on this computer can autosave.'
  if (tab.kind === 'pptx' && tab.path.toLowerCase().endsWith('.ppt')) return 'Save this PowerPoint file as .pptx first.'
  // Two tabs writing the same file would overwrite each other's changes.
  if (useWorkspace.getState().tabs.some((t) => t.id !== tab.id && t.path === tab.path)) return 'This file is open in another tab too.'
  return null
}

const timers = new Map<string, { timer: number; since: number }>()
const running = new Map<string, Promise<boolean>>()

function schedule(tabId: string) {
  const tab = useWorkspace.getState().tabs.find((t) => t.id === tabId)
  if (!tab?.dirty || autosaveBlocker(tab)) return
  const now = Date.now()
  const pending = timers.get(tabId)
  if (pending) window.clearTimeout(pending.timer)
  const since = pending?.since ?? now
  const wait = Math.max(0, Math.min(useSettings.getState().autosave.delay * 1000, since + MAX_WAIT - now))
  timers.set(tabId, { since, timer: window.setTimeout(() => void run(tabId, true), wait) })
}

function cancel(tabId: string) {
  const pending = timers.get(tabId)
  if (pending) window.clearTimeout(pending.timer)
  timers.delete(tabId)
}

/** If the editor is mid-entry, try the timed save again after the delay instead. Returns whether it waits. */
function retryWhenIdle(tabId: string): boolean {
  if (!getEditor(tabId)?.busy?.()) return false
  const pending = timers.get(tabId)
  if (pending) window.clearTimeout(pending.timer)
  timers.set(tabId, { since: pending?.since ?? Date.now(), timer: window.setTimeout(() => void run(tabId, true), useSettings.getState().autosave.delay * 1000) })
  return true
}

/**
 * Save one tab if it has changes autosave covers. Resolves true when it has none left. `timed`
 * (the timer, not quitting or closing): if the editor is mid-entry, wait and try again.
 */
async function run(tabId: string, timed = false): Promise<boolean> {
  if (timed && retryWhenIdle(tabId)) return false
  cancel(tabId)
  // One save per tab at a time; a change made meanwhile is picked up after it.
  while (running.has(tabId)) await running.get(tabId)
  // The user may have started an entry while that save ran.
  if (timed && retryWhenIdle(tabId)) return false
  const tab = useWorkspace.getState().tabs.find((t) => t.id === tabId)
  if (!tab) return true
  if (!tab.dirty) return true
  if (autosaveBlocker(tab) || !tab.path || !getEditor(tabId)) return false
  const path = tab.path
  const job = (async () => {
    // Let running.set below happen before anything else.
    await Promise.resolve()
    try {
      setStatus(tabId, { state: 'saving' })
      if (!(await useWorkspace.getState().save(tabId, { to: path, quiet: true }))) throw new Error('The document is still loading.')
      setStatus(tabId, { state: 'saved' })
      return true
    } catch (err) {
      setStatus(tabId, { state: 'error', message: String(err) })
      return false
    } finally {
      running.delete(tabId)
    }
  })()
  running.set(tabId, job)
  if (!(await job)) return false
  const after = useWorkspace.getState().tabs.find((t) => t.id === tabId)
  if (after?.dirty) schedule(tabId)
  return !after?.dirty
}

/** Wire autosave into the workspace. Returns a function that stops it. */
export function startAutosave(): () => void {
  autosaveHooks.edited = schedule
  autosaveHooks.flush = run
  // Turning autosave on (or widening it) picks up changes that are already waiting.
  const stopSettings = useSettings.subscribe((now, before) => {
    if (now.autosave === before.autosave) return
    for (const tab of useWorkspace.getState().tabs) {
      if (autosaveBlocker(tab, now.autosave)) cancel(tab.id)
      else schedule(tab.id)
    }
  })
  const stopTabs = useWorkspace.subscribe((now, before) => {
    if (now.tabs === before.tabs) return
    const open = new Set(now.tabs.map((t) => t.id))
    for (const id of timers.keys()) if (!open.has(id)) cancel(id)
    for (const id of Object.keys(useAutosaveStatus.getState())) if (!open.has(id)) setStatus(id, null)
  })
  return () => {
    stopSettings()
    stopTabs()
    for (const id of [...timers.keys()]) cancel(id)
    autosaveHooks.edited = () => {}
    autosaveHooks.flush = async () => false
  }
}

/** Save every tab autosave covers right away (before quitting). */
export async function flushAutosave(): Promise<void> {
  await Promise.all(useWorkspace.getState().tabs.filter((t) => t.dirty).map((t) => run(t.id)))
}
