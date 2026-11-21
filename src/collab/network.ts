// macOS's permission to reach the local network, which sharing needs. macOS asks the person the
// first time an app does something there that it checks; until they allow it, connections to
// other machines fail with "no route to host". Sharing and joining make macOS ask straight away
// (collab_local_network in src-tauri/src/collab/mod.rs), and joining waits while it's asking.

import { isMac, isTauri } from '../lib/platform'

/** How long to wait for an answer to macOS's question before giving up. */
const WAIT = 60_000

/** Makes macOS ask now, if nobody has answered yet. Does nothing elsewhere. */
export async function askForNetwork(): Promise<void> {
  if (!isTauri || !isMac) return
  const { invoke } = await import('@tauri-apps/api/core')
  await invoke('collab_local_network').catch(() => {})
}

/** Whether a failed connection is macOS keeping Writan off the local network. */
export function isNetworkRefusal(err: unknown): boolean {
  return isMac && /no route to host|os error 65\b/i.test(String((err as Error)?.message ?? err))
}

/**
 * Runs `attempt`. While it fails because macOS hasn't let Writan reach the network (it's still
 * asking, or was told no), `waiting(true)` is called and it's tried again every second, until it
 * works, fails some other way, `cancelled()` says to stop, or a minute passes. Resolves to
 * `null` when it gave up or was cancelled.
 */
export async function whileNetworkAllowed<T>(attempt: () => Promise<T>, waiting: (on: boolean) => void, cancelled: () => boolean): Promise<T | null> {
  const until = Date.now() + WAIT
  for (;;) {
    try {
      return await attempt()
    } catch (err) {
      if (!isNetworkRefusal(err)) throw err
      if (cancelled() || Date.now() > until) return null
      waiting(true)
      await new Promise((r) => setTimeout(r, 1000))
      if (cancelled()) return null
    }
  }
}

/** Opens Privacy & Security › Local Network in System Settings. */
export function openNetworkSettings() {
  void import('@tauri-apps/api/core').then(({ invoke }) => invoke('collab_network_settings')).catch(() => {})
}

/** While macOS asks. */
export const NETWORK_ASK = 'Writan needs to reach your local network to join. When macOS asks, choose Allow.'
/** After it was turned down, or nobody answered. */
export const NETWORK_DENIED = 'Writan can’t reach your local network. Turn on Writan in System Settings › Privacy & Security › Local Network, then try again.'
