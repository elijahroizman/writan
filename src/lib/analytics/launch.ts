// What this computer looked like before this launch wrote anything: main.tsx imports this
// first, so these are read before the rest of the app saves its state. Reads only.

const VERSION_KEY = 'workspace.analyticsVersion'
/** Things any earlier Writan saves on its first launch. */
const EARLIER_STATE = ['workspace.setupDone', 'workspace.session', 'workspace.recents', 'workspace.settings', 'workspace.theme', VERSION_KEY]

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

/** The version that ran last time, if a version with analytics ran before. */
export const previousVersion = read(VERSION_KEY)

/** Nothing of Writan's was saved here before this launch. */
export const firstLaunch = EARLIER_STATE.every((key) => read(key) === null)

/** Records this launch's version, for the next launch to compare with. */
export function rememberVersion(version: string) {
  try {
    localStorage.setItem(VERSION_KEY, version)
  } catch {
    // Without storage every launch looks like the first after an update would; it's skipped then.
  }
}
