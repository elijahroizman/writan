// The usage events that come from the app as a whole rather than from one place in it: the
// launch, each day it runs, unhandled errors, and shared documents. App.tsx starts these.

import { useCollab } from '../../collab/session'
import type { Share } from '../../collab/session'
import { analyticsAvailable, flushAnalytics, track, useAnalytics } from '../../state/analytics'
import { HOME_TAB_ID, useWorkspace } from '../../state/workspace'
import type { ErrorArea, ErrorSource, Kind } from './events'
import { errorNameOf, kindOf, minutesBucket, roundMs, versionOf } from './events'
import { firstLaunch, previousVersion, rememberVersion } from './launch'

const DAY_KEY = 'workspace.analyticsDay'
/** At most this many error events per launch, and one per kind of error per ten minutes. */
const ERROR_CAP = 20
const ERROR_GAP_MS = 10 * 60_000

/** Once the last session is on screen: the launch, an update since last time, and the day. */
export function appLaunched() {
  if (!analyticsAvailable) return
  const version = __APP_VERSION__
  if (previousVersion && previousVersion !== version) {
    track('app_updated', { from_version: versionOf(previousVersion), to_version: versionOf(version) })
  }
  rememberVersion(version)
  track('app_launched', { first_launch: firstLaunch, launch_ms: roundMs(performance.now()) })
  checkDay()
}

function today() {
  const d = new Date()
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`
}

/** One `app_active` per calendar day, so a copy left open for days still counts each day. */
function checkDay() {
  if (useAnalytics.getState().choice === false) return
  try {
    if (localStorage.getItem(DAY_KEY) === today()) return
    localStorage.setItem(DAY_KEY, today())
  } catch {
    return
  }
  track('app_active')
}

/** The editor in front, for an error's area. */
function area(): ErrorArea {
  const { activeId, tabs } = useWorkspace.getState()
  if (activeId === HOME_TAB_ID) return 'home'
  const tab = tabs.find((t) => t.id === activeId)
  return tab ? kindOf(tab.kind) : 'shell'
}

let errors = 0
const lastError = new Map<string, number>()

/** Records an error's name and where it happened; never its message or stack. */
export function trackError(error: unknown, source: ErrorSource, at: ErrorArea = area()) {
  if (errors >= ERROR_CAP) return
  const name = errorNameOf(error)
  const key = `${at}:${name}:${source}`
  const now = Date.now()
  if (now - (lastError.get(key) ?? -Infinity) < ERROR_GAP_MS) return
  lastError.set(key, now)
  errors++
  track('error', { area: at, name, source })
}

interface Session {
  kind: Kind
  role: Share['role']
  agentsOnly: boolean
  started: number
  peak: number
}

/** Shared documents: started or joined once live, ended when they stop, with how many took part. */
function followCollab(): () => void {
  const sessions = new Map<string, Session>()
  const end = (id: string) => {
    const s = sessions.get(id)
    if (!s) return
    sessions.delete(id)
    track('collab_ended', {
      kind: s.kind,
      role: s.role,
      agents_only: s.agentsOnly,
      peak_participants: s.peak,
      minutes_bucket: minutesBucket((Date.now() - s.started) / 60_000),
    })
  }
  return useCollab.subscribe((shares) => {
    for (const [id, share] of Object.entries(shares)) {
      // This app's own agents' copies, which have no tab.
      if (id.startsWith('agent:')) continue
      const s = sessions.get(id)
      if (share.status === 'live') {
        const participants = share.peers.length + 1
        if (!s) {
          const kind = kindOf(useWorkspace.getState().tabs.find((t) => t.id === id)?.kind)
          const agentsOnly = !!share.private
          sessions.set(id, { kind, role: share.role, agentsOnly, started: Date.now(), peak: participants })
          track('collab_started', { kind, role: share.role, agents_only: agentsOnly })
        } else {
          s.peak = Math.max(s.peak, participants)
          if (!share.private) s.agentsOnly = false
        }
      } else if (s && (share.status === 'ended' || share.status === 'disconnected')) {
        end(id)
      }
    }
    for (const id of [...sessions.keys()]) if (!shares[id]) end(id)
  })
}

/** Starts the app-wide events. Returns the undo. */
export function startAnalytics(): () => void {
  if (!analyticsAvailable) return () => {}
  const onError = (e: ErrorEvent) => trackError(e.error, 'uncaught')
  const onRejection = (e: PromiseRejectionEvent) => trackError(e.reason, 'rejection')
  const onHidden = () => document.visibilityState === 'hidden' && void flushAnalytics()
  window.addEventListener('error', onError)
  window.addEventListener('unhandledrejection', onRejection)
  document.addEventListener('visibilitychange', onHidden)
  const dayTimer = window.setInterval(checkDay, 30 * 60_000)
  const stopCollab = followCollab()
  return () => {
    window.removeEventListener('error', onError)
    window.removeEventListener('unhandledrejection', onRejection)
    document.removeEventListener('visibilitychange', onHidden)
    window.clearInterval(dayTimer)
    stopCollab()
  }
}
