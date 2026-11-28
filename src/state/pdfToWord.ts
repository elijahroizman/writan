// Opening a PDF as an editable Word document (the PDF inspector's button, the menu and the
// palette): one conversion per PDF tab at a time, with its progress for the inspector to show.

import { create } from 'zustand'
import * as platform from '../lib/platform'
import { useWorkspace } from './workspace'
import { track } from './analytics'

interface Job {
  status: string
  abort: AbortController
}

interface PdfToWordState {
  jobs: Record<string, Job>
  /** Converts the PDF in `tabId` and opens the Word copy in a new tab. */
  start(tabId: string): Promise<void>
  cancel(tabId: string): void
}

export const usePdfToWord = create<PdfToWordState>((set, get) => ({
  jobs: {},
  async start(tabId) {
    const tab = useWorkspace.getState().tabs.find((t) => t.id === tabId)
    if (!tab || tab.kind !== 'pdf' || get().jobs[tabId]) return
    track('feature_used', { feature: 'pdf_to_word', kind: 'pdf' })
    const abort = new AbortController()
    const update = (status: string) => {
      if (get().jobs[tabId]?.abort === abort) set((s) => ({ jobs: { ...s.jobs, [tabId]: { status, abort } } }))
    }
    set((s) => ({ jobs: { ...s.jobs, [tabId]: { status: 'Converting…', abort } } }))
    try {
      const { convertAndDeliver } = await import('../lib/convert')
      await convertAndDeliver(tabId, 'docx', 'open', { signal: abort.signal, onProgress: update })
    } catch (err) {
      if (!abort.signal.aborted) await platform.showError(err instanceof Error ? err.message : String(err), 'Couldn’t open the PDF in Word')
    } finally {
      set((s) => {
        const { [tabId]: _, ...jobs } = s.jobs
        return { jobs }
      })
    }
  },
  cancel(tabId) {
    get().jobs[tabId]?.abort.abort()
  },
}))
