import { create } from 'zustand'
import { isTauri } from '../platform'
import { getPackMeta, OCR_PACK_CATALOG, TESSDATA_BASE_URL } from './catalog'
import type { OcrPackInventory, OcrPackProgress } from './types'

const LANGS_KEY = 'workspace.ocrLangs'
const PACKS_STORE = 'packs'
const core = () => import('@tauri-apps/api/core')

export function normalizeOcrLangs(langs: readonly string[]): string[] {
  const unique = [...new Set(langs)]
  if (!unique.length) throw new Error('Select at least one OCR language.')
  for (const code of unique) requirePack(code)
  return unique
}

function loadLangs(): string[] {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(LANGS_KEY) ?? 'null')
    if (Array.isArray(saved)) return normalizeOcrLangs(saved)
  } catch {
    // Missing storage and obsolete selections both use English.
  }
  return ['eng']
}

function requirePack(code: string) {
  const pack = getPackMeta(code)
  if (!pack) throw new Error('Unknown OCR language pack.')
  return pack
}

interface OcrPackState extends OcrPackInventory {
  selectedLangs: string[]
  progress: Readonly<Record<string, OcrPackProgress | undefined>>
  setSelectedLangs: (langs: readonly string[]) => void
  ensurePacks: typeof ensurePacks
  downloadPack: typeof downloadPack
  cancelDownload: typeof cancelDownload
  removePack: typeof removePack
  readPackBytes: typeof readPackBytes
  refreshPacks: typeof refreshPacks
}

export const useOcrPacks = create<OcrPackState>((set) => ({
  installed: [],
  totalBytes: 0,
  selectedLangs: loadLangs(),
  progress: {},
  setSelectedLangs: (langs) => {
    const selectedLangs = normalizeOcrLangs(langs)
    try {
      localStorage.setItem(LANGS_KEY, JSON.stringify(selectedLangs))
    } catch {
      // The in-memory selection remains usable when storage is disabled.
    }
    set({ selectedLangs })
  },
  ensurePacks,
  downloadPack,
  cancelDownload,
  removePack,
  readPackBytes,
  refreshPacks,
}))

let database: Promise<IDBDatabase> | undefined

function openDatabase(): Promise<IDBDatabase> {
  database ??= new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open('writan-ocr-packs', 1)
    request.onupgradeneeded = () => request.result.createObjectStore(PACKS_STORE)
    request.onerror = () => reject(request.error)
    request.onsuccess = () => {
      const db = request.result
      db.onversionchange = () => {
        db.close()
        database = undefined
      }
      resolve(db)
    }
  }).catch((error) => {
    database = undefined
    throw error
  })
  return database
}

async function packTransaction<T>(
  mode: IDBTransactionMode,
  request: (store: IDBObjectStore) => IDBRequest<T>,
  signal?: AbortSignal,
): Promise<T> {
  signal?.throwIfAborted()
  const db = await openDatabase()
  signal?.throwIfAborted()
  return new Promise((resolve, reject) => {
    const transaction = db.transaction(PACKS_STORE, mode)
    // Obtain the request before registering cancellation so a synchronous error
    // during request creation cannot leave an abort listener behind.
    const operation = request(transaction.objectStore(PACKS_STORE))
    const abort = () => transaction.abort()
    signal?.addEventListener('abort', abort, { once: true })
    transaction.oncomplete = () => {
      signal?.removeEventListener('abort', abort)
      resolve(operation.result)
    }
    transaction.onabort = () => {
      signal?.removeEventListener('abort', abort)
      reject(signal?.aborted ? signal.reason : transaction.error)
    }
  })
}

async function verified(code: string, data: ArrayBuffer): Promise<boolean> {
  const pack = requirePack(code)
  if (data.byteLength !== pack.size) return false
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', data))
  const hex = Array.from(digest, (byte) => byte.toString(16).padStart(2, '0')).join('')
  return hex === pack.sha256
}

let refreshSequence = 0

export async function refreshPacks(): Promise<OcrPackInventory> {
  const sequence = ++refreshSequence
  const raw = isTauri
    ? await (await core()).invoke<OcrPackInventory | null>('ocr_list_packs')
    : await browserInventory()
  const inventory: OcrPackInventory =
    raw && Array.isArray(raw.installed) && typeof raw.totalBytes === 'number'
      ? { installed: raw.installed, totalBytes: raw.totalBytes }
      : { installed: [], totalBytes: 0 }
  if (sequence === refreshSequence) useOcrPacks.setState(inventory)
  return inventory
}

async function browserInventory(): Promise<OcrPackInventory> {
  const installed = []
  for (const { code } of OCR_PACK_CATALOG) {
    const data = await packTransaction<ArrayBuffer | undefined>('readonly', (store) => store.get(code))
    if (data) installed.push({ code, size: data.byteLength, sha256Verified: await verified(code, data) })
  }
  return { installed, totalBytes: installed.reduce((sum, pack) => sum + pack.size, 0) }
}

export async function readPackBytes(code: string, signal?: AbortSignal): Promise<Uint8Array<ArrayBuffer>> {
  const pack = requirePack(code)
  signal?.throwIfAborted()
  if (isTauri) {
    const { invoke } = await core()
    signal?.throwIfAborted()
    const data = await invoke<ArrayBuffer>('ocr_read_pack', { code })
    signal?.throwIfAborted()
    return new Uint8Array(data)
  }
  const data = await packTransaction<ArrayBuffer | undefined>('readonly', (store) => store.get(code), signal)
  if (!data) throw new Error('Download the ' + pack.name + ' OCR pack first.')
  const intact = await verified(code, data)
  signal?.throwIfAborted()
  if (!intact) throw new Error('The ' + pack.name + ' OCR pack is damaged. Download it again.')
  return new Uint8Array(data)
}

export function ensurePacks(
  langs: readonly string[] = useOcrPacks.getState().selectedLangs,
  signal?: AbortSignal,
): Promise<{ code: string; bytes: Uint8Array<ArrayBuffer> }[]> {
  return Promise.all(normalizeOcrLangs(langs).map(async (code) => ({ code, bytes: await readPackBytes(code, signal) })))
}

function setProgress(code: string, progress?: OcrPackProgress) {
  useOcrPacks.setState((state) => {
    const next = { ...state.progress }
    if (progress) next[code] = progress
    else delete next[code]
    return { progress: next }
  })
}

const downloads = new Map<string, AbortController>()
let packRevision = 0

/** Changes whenever the contents of installed packs can have changed. */
export function ocrPackRevision(): number {
  return packRevision
}

function browserPackBaseUrl(): string {
  if (import.meta.env.VITE_TEST_HOOKS) {
    const override = (window as Window & { __ocrPackBaseUrl?: string }).__ocrPackBaseUrl
    if (override) return override.endsWith('/') ? override : override + '/'
  }
  return TESSDATA_BASE_URL
}

async function browserDownload(code: string, signal: AbortSignal) {
  const pack = requirePack(code)
  const response = await fetch(browserPackBaseUrl() + code + '.traineddata', { signal })
  if (!response.ok || !response.body) throw new Error('OCR pack download failed (HTTP ' + response.status + ').')
  const data = new ArrayBuffer(pack.size)
  const bytes = new Uint8Array(data)
  const reader = response.body.getReader()
  let received = 0
  const integrityError = () => new Error('The ' + pack.name + ' OCR pack download failed verification. Try downloading it again.')
  try {
    while (true) {
      const { done, value } = await reader.read()
      signal.throwIfAborted()
      if (done) break
      if (received + value.length > pack.size) throw integrityError()
      bytes.set(value, received)
      received += value.length
      setProgress(code, { code, received, total: pack.size })
    }
  } catch (error) {
    await reader.cancel().catch(() => {})
    throw error
  } finally {
    reader.releaseLock()
  }
  if (received !== pack.size || !(await verified(code, data))) throw integrityError()
  await packTransaction('readwrite', (store) => store.put(data, code), signal)
}

export async function downloadPack(code: string): Promise<void> {
  const pack = requirePack(code)
  if (downloads.has(code)) throw new Error('The ' + pack.name + ' OCR pack is already downloading.')
  const controller = new AbortController()
  downloads.set(code, controller)
  setProgress(code, { code, received: 0, total: pack.size })
  let unlisten: (() => void) | undefined
  try {
    if (isTauri) {
      const [{ invoke }, { listen }] = await Promise.all([core(), import('@tauri-apps/api/event')])
      unlisten = await listen<OcrPackProgress>('ocr-pack-progress', ({ payload }) => {
        if (payload.code !== code) return
        setProgress(code, payload)
        // Covers cancellation sent before the native command was registered.
        if (controller.signal.aborted) void invoke('ocr_cancel_download', { code }).catch(() => {})
      })
      controller.signal.throwIfAborted()
      await invoke('ocr_download_pack', { code })
    } else {
      await browserDownload(code, controller.signal)
    }
    ++packRevision
    await refreshPacks()
  } catch (error) {
    if (controller.signal.aborted) throw controller.signal.reason
    if (error instanceof DOMException && error.name === 'QuotaExceededError') {
      throw new Error('OCR storage is full. Free up space and try downloading the language pack again.')
    }
    throw error
  } finally {
    unlisten?.()
    downloads.delete(code)
    setProgress(code)
  }
}

export async function cancelDownload(code: string): Promise<void> {
  requirePack(code)
  downloads.get(code)?.abort(new DOMException('OCR pack download cancelled.', 'AbortError'))
  if (isTauri) await (await core()).invoke('ocr_cancel_download', { code })
}

export async function removePack(code: string): Promise<void> {
  requirePack(code)
  if (downloads.has(code)) throw new Error('Wait for the OCR pack download to finish or cancel it before removing the pack.')
  if (isTauri) await (await core()).invoke('ocr_remove_pack', { code })
  else await packTransaction('readwrite', (store) => store.delete(code))
  ++packRevision
  await refreshPacks()
}
