import { create } from 'zustand'
import * as platform from '../lib/platform'
import { useSettings } from '../state/settings'
import { useWorkspace } from '../state/workspace'

/** Which tab's history is showing, if any. */
export const useVersionHistory = create<{ tabId: string | null; show(tabId: string | null): void }>((set) => ({
  tabId: null,
  show: (tabId) => set({ tabId }),
}))

/** Open the history of the tab in front, or explain why there isn't one. */
export function showVersionHistory(tabId = useWorkspace.getState().activeId) {
  const tab = useWorkspace.getState().tabs.find((t) => t.id === tabId)
  if (!tab) return
  if (!tab.path) {
    void platform.showError('Save this document first. Writan keeps a version each time you save a file.', 'No versions yet')
    return
  }
  if (!useSettings.getState().history) {
    void platform
      .confirmDialog('Version history is off, so Writan isn’t keeping copies when you save. Turn it on?', { okLabel: 'Turn On', kind: 'info' })
      .then((ok) => {
        if (!ok) return
        useSettings.getState().setHistory(true)
        useVersionHistory.getState().show(tabId)
      })
    return
  }
  useVersionHistory.getState().show(tabId)
}
