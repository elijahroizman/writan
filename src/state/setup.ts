// The onboarding (components/Setup.tsx) shows once per version of it. A new install sees it on
// its first launch, and so does a copy updating from a version that showed an older one, or none.
// Quitting halfway shows it again on the next launch.

import { create } from 'zustand'

const KEY = 'workspace.setupDone'

/**
 * The onboarding someone last finished. Writan 0.x saved `1`, or nothing if it updated from a
 * version without setup. Raise this to show a new onboarding to everyone once.
 */
export const SETUP_VERSION = '2'

function firstRun(): boolean {
  try {
    if (localStorage.getItem(KEY) === SETUP_VERSION) return false
    localStorage.setItem(KEY, 'pending')
    return true
  } catch {
    return false
  }
}

interface SetupState {
  open: boolean
  /** Shown because this copy ran an older version before: it had finished an older onboarding,
   * or had state saved before setup existed. */
  upgraded: boolean
  /** Show it again (Settings › General). */
  show(): void
  finish(): void
}

function hadEarlierVersion(): boolean {
  try {
    const done = localStorage.getItem(KEY)
    return (done !== null && done !== 'pending' && done !== SETUP_VERSION) || localStorage.getItem('workspace.session') !== null
  } catch {
    return false
  }
}

const upgraded = hadEarlierVersion()

export const useSetup = create<SetupState>((set) => ({
  upgraded,
  open: firstRun(),
  show: () => set({ open: true }),
  finish: () => {
    try {
      localStorage.setItem(KEY, SETUP_VERSION)
    } catch {
      // Not kept: it shows again next launch.
    }
    set({ open: false })
  },
}))
