// Crash recovery: a moment after each edit, a copy of every unsaved document goes into the
// webview's IndexedDB, so a crash or a reload of the window doesn't lose it. The next launch puts
// those documents back as unsaved tabs (see recover() in state/workspace.ts). Quitting normally,
// where every unsaved tab was saved or discarded, leaves nothing to recover.

import type { RecoveredDoc } from '../state/workspace'
import { diskDigest, getEditor, recoveryHooks, useWorkspace } from '../state/workspace'

/** Copy this long after the last edit... */
const DELAY = 1000
/** ...and keep typing, at least this often. */
const MAX_WAIT = 10_000

const DB_NAME = 'writan-recovery'
const STORE = 'docs'

let opening: Promise<IDBDatabase> | null = null
function db(): Promise<IDBDatabase> {
  opening ??= new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1)
    req.onupgradeneeded = () => req.result.createObjectStore(STORE, { keyPath: 'id' })
    req.onsuccess = () => resolve(req.result)
    req.onerror = () => reject(req.error)
  })
  return opening
}

async function withStore<T>(mode: IDBTransactionMode, use: (store: IDBObjectStore) => IDBRequest<T> | void): Promise<T | undefined> {
  const tx = (await db()).transaction(STORE, mode, { durability: 'strict' })
  const req = use(tx.objectStore(STORE))
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve(req ? req.result : undefined)
    tx.onerror = tx.onabort = () => reject(tx.error)
  })
}

/** Store operations run one at a time, in the order they were asked for. */
let queue: Promise<unknown> = Promise.resolve()
function enqueue(op: () => Promise<unknown>): Promise<void> {
  const next = queue.then(op).catch((err) => console.warn('Crash recovery:', err))
  queue = next
  return next.then(() => {})
}

/** Tabs this run has copied into the store. Entries of other runs are only touched by recover(). */
const stored = new Set<string>()
const timers = new Map<string, { timer: number; since: number }>()
let started = false
/** Set while quitting: the user already chose what happens to every unsaved tab. */
let paused = false
const stopped = () => !started || paused

function schedule(tabId: string, wait = DELAY) {
  if (stopped()) return
  const now = Date.now()
  const pending = timers.get(tabId)
  if (pending) window.clearTimeout(pending.timer)
  const since = pending?.since ?? now
  timers.set(tabId, { since, timer: window.setTimeout(() => void store(tabId), Math.max(0, Math.min(wait, since + MAX_WAIT - now))) })
}

function cancel(tabId: string) {
  const pending = timers.get(tabId)
  if (pending) window.clearTimeout(pending.timer)
  timers.delete(tabId)
}

function forget(tabId: string) {
  cancel(tabId)
  if (!stored.delete(tabId)) return
  void enqueue(() => withStore('readwrite', (s) => s.delete(tabId)))
}

async function store(tabId: string) {
  timers.delete(tabId)
  if (stopped()) return
  const tab = useWorkspace.getState().tabs.find((t) => t.id === tabId)
  if (!tab?.dirty) return
  const editor = getEditor(tabId)
  // Reading the bytes would cut short what the user is typing (a cell, a PDF form field).
  if (editor?.busy?.()) return schedule(tabId)
  let bytes: Uint8Array | null
  try {
    // A recovered tab nobody has opened yet has no editor: its bytes are the document.
    bytes = editor ? await editor.getBytes() : tab.bytes
  } catch {
    return schedule(tabId, MAX_WAIT)
  }
  const base = await diskDigest(tabId)
  const { tabs } = useWorkspace.getState()
  const now = tabs.find((t) => t.id === tabId)
  if (!bytes || !now?.dirty || stopped()) return
  const doc: RecoveredDoc = { id: tabId, kind: now.kind, title: now.title, path: now.path, bytes, base, index: tabs.indexOf(now), savedAt: Date.now() }
  stored.add(tabId)
  await enqueue(() => withStore('readwrite', (s) => s.put(doc)))
}

/** What load() returned, by tab id, for adopt() to re-key. */
const loaded = new Map<string, RecoveredDoc>()

/** Documents a previous run left unsaved, oldest tab position first. */
async function load(): Promise<RecoveredDoc[]> {
  try {
    const docs = (await withStore<RecoveredDoc[]>('readonly', (s) => s.getAll() as IDBRequest<RecoveredDoc[]>)) ?? []
    const valid = docs.filter((d) => d && typeof d.id === 'string' && d.bytes instanceof Uint8Array).sort((a, b) => a.index - b.index)
    for (const doc of valid) loaded.set(doc.id, doc)
    return valid
  } catch (err) {
    console.warn('Crash recovery:', err)
    return []
  }
}

/**
 * recover() reopened these documents in new tabs: move each copy to its new tab id as stored, not
 * from the editor, which may still show the file as it was on disk. One transaction, so a crash
 * mid-way leaves either the old copies or the new ones.
 */
function adopt(pairs: [string, string][]) {
  const { tabs } = useWorkspace.getState()
  const moves = pairs.flatMap(([oldId, tabId]) => {
    const doc = loaded.get(oldId)
    loaded.delete(oldId)
    if (!doc) return []
    stored.add(tabId)
    const index = tabs.findIndex((t) => t.id === tabId)
    return [{ oldId, doc: { ...doc, id: tabId, index: index < 0 ? doc.index : index } }]
  })
  if (!moves.length) return
  void enqueue(() =>
    withStore('readwrite', (s) => {
      for (const { oldId, doc } of moves) {
        if (oldId !== doc.id) s.delete(oldId)
        s.put(doc)
      }
    }),
  )
}

/** Wire crash recovery into the workspace. Returns a function that stops it. */
export function startRecovery(): () => void {
  started = true
  recoveryHooks.edited = (tabId) => schedule(tabId)
  recoveryHooks.load = load
  recoveryHooks.adopt = adopt
  const stopTabs = useWorkspace.subscribe((now, before) => {
    if (now.tabs === before.tabs) return
    const byId = new Map(now.tabs.map((t) => [t.id, t]))
    for (const id of new Set([...stored, ...timers.keys()])) {
      const tab = byId.get(id)
      if (!tab?.dirty) forget(id)
    }
    // A Save As that left changes unsaved moved the document: keep the copy's name and place current.
    for (const old of before.tabs) {
      const tab = byId.get(old.id)
      if (tab?.dirty && stored.has(tab.id) && (tab.path !== old.path || tab.title !== old.title)) schedule(tab.id)
    }
  })
  return () => {
    stopTabs()
    started = false
    for (const id of [...timers.keys()]) cancel(id)
    recoveryHooks.edited = () => {}
    recoveryHooks.load = async () => []
    recoveryHooks.adopt = () => {}
  }
}

/**
 * The user saved or discarded every unsaved tab and the app is about to quit: drop the copies, and
 * take no more until resumeRecovery() (the quit didn't happen after all).
 */
export async function discardRecovery(): Promise<void> {
  paused = true
  for (const id of [...timers.keys()]) cancel(id)
  const ids = [...stored]
  stored.clear()
  await enqueue(() => withStore('readwrite', (s) => ids.forEach((id) => s.delete(id))))
}

/** Quitting was cancelled or failed: copy the tabs that are still unsaved again. */
export function resumeRecovery() {
  if (!paused) return
  paused = false
  if (!started) return
  for (const tab of useWorkspace.getState().tabs) if (tab.dirty) schedule(tab.id, 0)
}
