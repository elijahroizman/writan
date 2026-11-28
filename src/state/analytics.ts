// Anonymous usage analytics: whether the person allows it, and the events waiting to go.
//
// Nothing leaves this computer unless all of these hold: the build has a PostHog key
// (VITE_POSTHOG_KEY), it's the desktop app and not a dev build, and the person turned the
// setting on. Until they decide (`choice` is null), events wait in memory and go only if they
// turn it on; turning it off drops them. Rust posts the batches (src-tauri/src/analytics.rs).
// What may be sent is in lib/analytics/events.ts; docs/internals/analytics.md explains it.

import { create } from 'zustand'
import { isTauri } from '../lib/platform'
import type { EventName, Events } from '../lib/analytics/events'
import { versionOf } from '../lib/analytics/events'

const CHOICE_KEY = 'workspace.analytics'
const ID_KEY = 'workspace.analyticsId'
/** Owned by the update channel setting. */
const CHANNEL_KEY = 'workspace.updateChannel'

const FLUSH_MS = 30_000
/** Events kept waiting at most; the oldest go first. */
const QUEUE_CAP = 200
/** A post that hasn't answered by then is left to finish in the background. */
const FINAL_FLUSH_MS = 1500

interface Config {
  key: string
  host: string
}

function readConfig(): Config | null {
  // Test builds can stand in a key, to check what would be sent.
  const test = import.meta.env.VITE_TEST_HOOKS ? (globalThis as { __analyticsTest?: { key?: string } }).__analyticsTest : undefined
  const key = (test?.key ?? import.meta.env.VITE_POSTHOG_KEY ?? '').trim()
  if (!key || !isTauri) return null
  if (import.meta.env.DEV && !test) return null
  const host = (import.meta.env.VITE_POSTHOG_HOST ?? '').trim() || 'https://us.i.posthog.com'
  return { key, host }
}

const config = readConfig()

/** Whether this build can send analytics at all. False means every call here does nothing. */
export const analyticsAvailable = config !== null

// ── The choice

function loadChoice(): boolean | null {
  try {
    const saved = localStorage.getItem(CHOICE_KEY)
    return saved === 'true' ? true : saved === 'false' ? false : null
  } catch {
    return null
  }
}

/** The country and territory codes of the EU, the EEA, the UK and Switzerland. */
const EUROPE = new Set([
  'AT', 'BE', 'BG', 'HR', 'CY', 'CZ', 'DK', 'EE', 'FI', 'FR', 'DE', 'GR', 'HU', 'IE', 'IT', 'LV', 'LT', 'LU', 'MT', 'NL',
  'PL', 'PT', 'RO', 'SK', 'SI', 'ES', 'SE', 'IS', 'LI', 'NO', 'GB', 'CH',
  // Their territories in and near Europe, and the French overseas departments, which are in the EU.
  'AX', 'FO', 'GI', 'GG', 'JE', 'IM', 'SJ', 'GF', 'GP', 'MQ', 'RE', 'YT', 'MF',
])

/** Time zones in those countries outside `Europe/`, and the old names some systems still give. */
const EUROPEAN_ZONES = new Set([
  'Atlantic/Reykjavik', 'Atlantic/Canary', 'Atlantic/Madeira', 'Atlantic/Azores', 'Atlantic/Faroe', 'Atlantic/Faeroe', 'Atlantic/Jan_Mayen',
  'Arctic/Longyearbyen', 'Asia/Nicosia', 'Asia/Famagusta',
  'America/Cayenne', 'America/Guadeloupe', 'America/Martinique', 'America/Marigot', 'Indian/Reunion', 'Indian/Mayotte',
  'GB', 'GB-Eire', 'Eire', 'Iceland', 'Portugal', 'Poland', 'WET', 'CET', 'MET', 'EET',
])

function regionOf(tag: string): string | null {
  try {
    const region = new Intl.Locale(tag).region
    if (region) return region.toUpperCase()
  } catch {
    // Not a valid tag: fall back to reading it.
  }
  const match = /^[a-z]{2,3}(?:[-_][a-z]{4})?[-_]([a-z]{2})\b/i.exec(tag)
  return match ? match[1].toUpperCase() : null
}

/** Zones that say nothing about where the computer is. */
const placeless = (zone: string) => !zone || zone === 'UTC' || zone === 'GMT' || zone.startsWith('Etc/') || zone === 'Universal' || zone === 'Zulu'

/**
 * Whether the setting starts on: everywhere but the EU, the EEA, the UK and Switzerland, where
 * the law asks for agreement first. Worked out on this computer, and nothing is looked up. The
 * time zone decides when it names a place. The languages' regions decide only when it doesn't,
 * because many people outside Europe use British English: Chromium on an Australian system
 * reports en-GB.
 */
export function defaultAnalyticsOn(): boolean {
  let zone = ''
  try {
    zone = Intl.DateTimeFormat().resolvedOptions().timeZone ?? ''
  } catch {
    // No time zone: go by the languages.
  }
  if (!placeless(zone)) return !(zone.startsWith('Europe/') || EUROPEAN_ZONES.has(zone))
  const languages = typeof navigator === 'undefined' ? [] : navigator.languages?.length ? navigator.languages : [navigator.language]
  for (const tag of languages) {
    const region = tag ? regionOf(tag) : null
    if (region && EUROPE.has(region)) return false
  }
  return true
}

interface AnalyticsState {
  /** What the person chose; null until they decide, and nothing is sent while it's null. */
  choice: boolean | null
}

export const useAnalytics = create<AnalyticsState>(() => ({ choice: loadChoice() }))

/** Whether the setting shows as on: the person's choice, or the default for where they are. */
export function analyticsShownOn(choice: boolean | null): boolean {
  return choice ?? defaultAnalyticsOn()
}

/**
 * Records the person's choice. Off stops sending at once, drops the waiting events and deletes
 * this install's ID; on again starts with a new ID.
 */
export function setAnalytics(on: boolean): void {
  try {
    localStorage.setItem(CHOICE_KEY, String(on))
  } catch {
    // Storage is full or off: the choice still holds until the app quits.
  }
  if (!on) {
    queue.length = 0
    try {
      localStorage.removeItem(ID_KEY)
    } catch {
      // Nothing to remove.
    }
  }
  useAnalytics.setState({ choice: on })
  if (on) scheduleFlush()
}

// ── Events

interface Queued {
  event: EventName
  properties: Record<string, unknown>
  timestamp: string
}

const queue: Queued[] = []
let timer: ReturnType<typeof setTimeout> | null = null

function installId(): string {
  try {
    const saved = localStorage.getItem(ID_KEY)
    if (saved && /^[0-9a-f-]{36}$/.test(saved)) return saved
    const id = crypto.randomUUID()
    localStorage.setItem(ID_KEY, id)
    return id
  } catch {
    // No storage: a new ID each launch, which counts this computer more than once but stays anonymous.
    return (sessionId ??= crypto.randomUUID())
  }
}
let sessionId: string | undefined

function channel(): 'stable' | 'nightly' {
  try {
    return localStorage.getItem(CHANNEL_KEY) === 'nightly' ? 'nightly' : 'stable'
  } catch {
    return 'stable'
  }
}

type Props<E extends EventName> = keyof Events[E] extends never ? [properties?: Events[E]] : [properties: Events[E]]

/**
 * Records one event. Its properties are checked against `Events`, so only numbers, booleans
 * and the listed strings get in. Does nothing when the build can't send or the person said no.
 */
export function track<E extends EventName>(event: E, ...[properties]: Props<E>): void {
  if (!config || useAnalytics.getState().choice === false) return
  queue.push({
    event,
    properties: {
      ...properties,
      app_version: versionOf(__APP_VERSION__),
      channel: channel(),
      language: typeof navigator === 'undefined' ? 'other' : navigator.language || 'other',
    },
    timestamp: new Date().toISOString(),
  })
  if (queue.length > QUEUE_CAP) queue.splice(0, queue.length - QUEUE_CAP)
  scheduleFlush()
}

function scheduleFlush() {
  if (timer !== null || !config || useAnalytics.getState().choice !== true || !queue.length) return
  timer = setTimeout(() => {
    timer = null
    void flushAnalytics()
  }, FLUSH_MS)
}

async function send(batch: Queued[]): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core')
  const id = installId()
  await invoke('analytics_send', {
    host: config!.host,
    key: config!.key,
    events: batch.map((e) => ({
      event: e.event,
      timestamp: e.timestamp,
      properties: { ...e.properties, distinct_id: id, $process_person_profile: false, $geoip_disable: true, $lib: 'writan' },
    })),
  })
}

/**
 * Sends what's waiting, if the person allowed it. Rust retries a failed batch once and then
 * drops it. With `final` (quitting), waits at most 1.5 seconds.
 */
export async function flushAnalytics(opts?: { final?: boolean }): Promise<void> {
  if (timer !== null) {
    clearTimeout(timer)
    timer = null
  }
  if (!config || useAnalytics.getState().choice !== true || !queue.length) return
  const job = send(queue.splice(0, queue.length)).catch(() => {})
  if (!opts?.final) return job
  await Promise.race([job, new Promise((r) => setTimeout(r, FINAL_FLUSH_MS))])
}
