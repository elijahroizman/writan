import { useEffect } from 'react'
import { attach, type CollabEngine } from './session'

/** Connects an editor's engine to its tab's sharing, while both last. */
export function useShared(tabId: string, engine: CollabEngine | null) {
  useEffect(() => (engine ? attach(tabId, engine) : undefined), [tabId, engine])
}
