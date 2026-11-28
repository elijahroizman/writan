// How long documents take to open, and where opening fails. The clock starts when someone asks
// to open a file (or, for a tab opened in the background, when it's first shown) and stops when
// its editor has drawn the document. Only the kind, the extension from a fixed list and the
// size in steps are kept; the path is read for its extension here and goes no further.

import { track } from '../../state/analytics'
import type { OpenSource, OpenStage } from './events'
import { extensionOf, kindOf, roundMs, sizeBucket } from './events'

const OFFICE = new Set(['docx', 'pptx', 'xlsx', 'csv'])
/** An open that hasn't finished by then (a tab closed while loading) is forgotten. */
const STALE_MS = 10 * 60_000

interface Pending {
  kind: string
  extension: ReturnType<typeof extensionOf>
  bytes: number
  source: OpenSource
  started: number
  /** Opened behind another tab: the clock starts when it's first shown. */
  background: boolean
  engineCold?: boolean
  /** A password was asked for, so the time says more about the person than the app. */
  waited: boolean
}

const pending = new Map<string, Pending>()

let engineStarted = false
/** Called by engine/load.ts the first time the document engine starts loading. */
export function noteEngineStarted() {
  engineStarted = true
}

export const openClock = () => performance.now()

/** A file was read into a new tab. `started` is when it was asked for (`openClock`). */
export function openRead(tabId: string, path: string, kind: string, bytes: number, started: number, opts: { source: OpenSource; background: boolean }) {
  const now = performance.now()
  for (const [id, p] of pending) if (now - p.started > STALE_MS) pending.delete(id)
  pending.set(tabId, {
    kind,
    extension: extensionOf(path),
    bytes,
    source: opts.source,
    started,
    background: opts.background,
    engineCold: OFFICE.has(kind) ? !engineStarted : undefined,
    waited: false,
  })
}

/** The tab's editor is about to load. A background tab's clock starts now. */
export function editorShown(tabId: string) {
  const p = pending.get(tabId)
  if (p?.background) {
    p.background = false
    p.started = performance.now()
    if (OFFICE.has(p.kind)) p.engineCold = !engineStarted
  }
}

/** The editor is waiting on the person (a PDF's password). */
export function documentWaiting(tabId: string) {
  const p = pending.get(tabId)
  if (p) p.waited = true
}

/** The editor has drawn the document. Only the first time after it was opened counts. */
export function documentReady(tabId: string) {
  const p = pending.get(tabId)
  if (!p || p.background) return
  pending.delete(tabId)
  track('document_opened', {
    kind: kindOf(p.kind),
    extension: p.extension,
    size_bucket: sizeBucket(p.bytes),
    source: p.source,
    ...(p.waited ? {} : { open_ms: roundMs(performance.now() - p.started) }),
    ...(p.engineCold === undefined ? {} : { engine_cold: p.engineCold }),
  })
}

/** The editor couldn't open what it was given. */
export function documentFailed(tabId: string, stage: OpenStage) {
  const p = pending.get(tabId)
  if (!p) return
  pending.delete(tabId)
  track('document_open_failed', { kind: kindOf(p.kind), extension: p.extension, stage })
}

/** Opening failed before there was a tab: the file couldn't be read, or Writan doesn't open its type. */
export function openFailed(path: string, kind: string | null, stage: OpenStage) {
  track('document_open_failed', { kind: kindOf(kind), extension: extensionOf(path), stage })
}
