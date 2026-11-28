// Which releases this copy updates to. Stable is the tested releases; nightly is a build of
// main, made whenever something new lands. Both share the app ID and signing, so switching
// happens in place. Moving from nightly to stable never installs an older version: the copy
// stays on its nightly until a stable release newer than it comes out. src/lib/updater.ts
// checks again when the channel changes.

import { create } from 'zustand'
import { isNightlyVersion } from '../lib/versions'

export type UpdateChannel = 'stable' | 'nightly'

/** The stored choice. Other code (analytics) reads this key, so its name and values stay. */
export const UPDATE_CHANNEL_KEY = 'workspace.updateChannel'

/** A nightly build follows the nightlies until someone picks a channel; any other build follows stable. */
const DEFAULT_CHANNEL: UpdateChannel = isNightlyVersion(__APP_VERSION__) ? 'nightly' : 'stable'

function load(): UpdateChannel {
  try {
    const stored = localStorage.getItem(UPDATE_CHANNEL_KEY)
    if (stored === 'stable' || stored === 'nightly') return stored
  } catch {
    // Storage can be unavailable; the default applies.
  }
  return DEFAULT_CHANNEL
}

const store = create<{ channel: UpdateChannel }>(() => ({ channel: load() }))

/** The channel this copy follows. */
export function getUpdateChannel(): UpdateChannel {
  return store.getState().channel
}

/** Follow `channel` from now on. The next update check runs right away. */
export function setUpdateChannel(channel: UpdateChannel): void {
  try {
    localStorage.setItem(UPDATE_CHANNEL_KEY, channel)
  } catch {
    // Kept for this session only.
  }
  if (store.getState().channel !== channel) store.setState({ channel })
}

/** The channel, for a component that shows or changes it. */
export function useUpdateChannel(): UpdateChannel {
  return store((s) => s.channel)
}

/** Calls `listener` with the new channel each time it changes. Returns the unsubscribe. */
export function onUpdateChannelChange(listener: (channel: UpdateChannel) => void): () => void {
  return store.subscribe((state, prev) => {
    if (state.channel !== prev.channel) listener(state.channel)
  })
}
