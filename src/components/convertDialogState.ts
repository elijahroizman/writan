import { create } from 'zustand'
import type { ConvertFormat } from '../lib/convert'

export type ConvertModalMode = 'convert' | 'bundle'

interface ConvertModalState {
  open: boolean
  mode: ConvertModalMode
  tabId: string | null
  /** The format selected when the dialog opens. */
  format: ConvertFormat | null
  openConvert(tabId?: string | null, format?: ConvertFormat): void
  openBundle(): void
  close(): void
}

export const useConvertDialog = create<ConvertModalState>((set) => ({
  open: false,
  mode: 'convert',
  tabId: null,
  format: null,
  openConvert: (tabId = null, format) => set({ open: true, mode: 'convert', tabId, format: format ?? null }),
  openBundle: () => set({ open: true, mode: 'bundle', tabId: null, format: null }),
  close: () => set({ open: false, tabId: null, format: null }),
}))
