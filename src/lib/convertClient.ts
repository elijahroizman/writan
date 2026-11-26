import type { ConvertWorkerRequest, ConvertWorkerResponse } from './convertWorker'

const IDLE_MS = 30_000

type Pending = { resolve: (res: ConvertWorkerResponse) => void; reject: (err: unknown) => void }

let worker: Worker | undefined
let idleTimer: ReturnType<typeof setTimeout> | undefined
let nextId = 0
const pending = new Map<number, Pending>()

function failAll(err: unknown) {
  for (const entry of pending.values()) entry.reject(err)
  pending.clear()
}

function stopWorker() {
  clearTimeout(idleTimer)
  worker?.terminate()
  worker = undefined
}

function scheduleIdle() {
  clearTimeout(idleTimer)
  if (!pending.size && worker) idleTimer = setTimeout(stopWorker, IDLE_MS)
}

function ensureWorker(): Worker {
  clearTimeout(idleTimer)
  if (worker) return worker
  const next = new Worker(new URL('./convertWorker.ts', import.meta.url), { type: 'module', name: 'writan-office-convert' })
  next.onmessage = (event: MessageEvent<ConvertWorkerResponse>) => {
    const res = event.data
    const entry = pending.get(res.id)
    if (!entry) return
    pending.delete(res.id)
    scheduleIdle()
    if (res.kind === 'error') entry.reject(new Error(res.message))
    else entry.resolve(res)
  }
  next.onerror = (event) => {
    failAll(new Error(event.message || 'The Office conversion worker stopped unexpectedly.'))
    stopWorker()
  }
  next.onmessageerror = () => {
    failAll(new Error('The Office conversion worker returned unreadable data.'))
    stopWorker()
  }
  worker = next
  return next
}

function call(request: DistributiveOmit<ConvertWorkerRequest, 'id'>, bytes: Uint8Array, signal?: AbortSignal): Promise<ConvertWorkerResponse> {
  if (signal?.aborted) return Promise.reject(signal.reason instanceof Error ? signal.reason : new DOMException('Cancelled', 'AbortError'))
  const w = ensureWorker()
  const id = ++nextId
  return new Promise((resolve, reject) => {
    const cleanup = () => signal?.removeEventListener('abort', onAbort)
    const onAbort = () => {
      pending.delete(id)
      cleanup()
      reject(signal?.reason instanceof Error ? signal.reason : new DOMException('Cancelled', 'AbortError'))
      if (!pending.size) stopWorker()
    }
    signal?.addEventListener('abort', onAbort, { once: true })
    pending.set(id, {
      resolve: (res) => {
        cleanup()
        resolve(res)
      },
      reject: (err) => {
        cleanup()
        reject(err)
      },
    })
    w.postMessage({ ...request, id }, [bytes.buffer])
  })
}

function transferableCopy(bytes: Uint8Array): Uint8Array {
  return bytes.slice()
}

/** Office-format conversion in a worker, so a large document does not freeze the editor shell. */
export async function convertDocumentOffThread(bytes: Uint8Array, name: string, target: string, signal?: AbortSignal): Promise<Uint8Array> {
  const copy = transferableCopy(bytes)
  const res = await call({ kind: 'convertDocument', bytes: copy, name, target }, copy, signal)
  if (res.kind !== 'bytes') throw new Error('The Office conversion worker gave an unexpected answer.')
  return res.bytes
}

/** Office-to-Markdown extraction in a worker, so a large document does not freeze the editor shell. */
export async function markdownOffThread(bytes: Uint8Array, name: string, signal?: AbortSignal): Promise<string> {
  const copy = transferableCopy(bytes)
  const res = await call({ kind: 'toMarkdown', bytes: copy, name }, copy, signal)
  if (res.kind !== 'text') throw new Error('The Office conversion worker gave an unexpected answer.')
  return res.text
}

type DistributiveOmit<T, K extends keyof T> = T extends unknown ? Omit<T, K> : never
