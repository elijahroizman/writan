// macOS's permission to reach the local network, which sharing needs. macOS asks the person the
// first time an app tries; until they allow it, every connection to another machine fails.
// Sharing and joining try first (collab_local_network in src-tauri/src/collab/mod.rs), so macOS
// asks then, and wait while the question is up.

import { isMac, isTauri } from '../lib/platform'

export type NetworkAccess = 'granted' | 'denied' | 'unknown'

/** How long to wait for an answer to macOS's question before giving up. */
const WAIT = 60_000

async function check(): Promise<NetworkAccess> {
  if (!isTauri || !isMac) return 'granted'
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<NetworkAccess>('collab_local_network').catch(() => 'unknown' as const)
}

/**
 * Resolves true once Writan may reach the local network (or there's no telling). While macOS
 * hasn't allowed it, `waiting(true)` is called, and the check runs again every second until
 * it's allowed, `cancelled()` says to stop, or a minute passes.
 */
export async function localNetworkAllowed(waiting: (on: boolean) => void, cancelled: () => boolean = () => false): Promise<boolean> {
  if ((await check()) !== 'denied') return true
  waiting(true)
  try {
    const until = Date.now() + WAIT
    while (Date.now() < until && !cancelled()) {
      await new Promise((r) => setTimeout(r, 1000))
      if ((await check()) !== 'denied') return true
    }
    return false
  } finally {
    waiting(false)
  }
}

/** Opens Privacy & Security › Local Network in System Settings. */
export function openNetworkSettings() {
  void import('@tauri-apps/api/core').then(({ invoke }) => invoke('collab_network_settings')).catch(() => {})
}

/** While macOS asks. */
export const NETWORK_ASK = 'Writan needs to reach your local network to share. When macOS asks, choose Allow.'
/** After it was turned down, or nobody answered. */
export const NETWORK_DENIED = 'Writan can’t reach your local network. Turn on Writan in System Settings › Privacy & Security › Local Network, then try again.'
