// Main-thread side of the PDFium worker. Each open PDF gets its own worker, so closing a tab
// hands all of its memory back; the compiled WebAssembly module is shared between them.

import wasmUrl from '@embedpdf/pdfium/pdfium.wasm?url'
import type { Api, Bitmap, Boot, Method, OpenResult, Request, Response } from './protocol'

/** Lower runs first. */
export const Priority = {
  /** Something the user just did (rotate, save, add a highlight). */
  action: 0,
  /** Pages on screen; add the distance from the middle of the window so the centre draws first. */
  visible: 10,
  tiles: 20,
  text: 30,
  nearby: 40,
  thumbnails: 50,
  search: 60,
  /** Recognizing text on scanned pages in the background. */
  ocr: 70,
} as const

export class Cancelled extends Error {
  constructor() {
    super('cancelled')
  }
}

/** Open engines, for automated checks in test builds (window.__pdfEngines). */
const testEngines = new Set<PdfEngine>()
if (import.meta.env.DEV || import.meta.env.VITE_TEST_HOOKS) Object.assign(window, { __pdfEngines: testEngines })

let compiled: Promise<WebAssembly.Module | undefined> | null = null
let runningWorkers = 0

/** The compiled module is big; let it go when no PDF has a worker running. */
function workerStopped() {
  if (--runningWorkers === 0) compiled = null
}

function compileOnce() {
  compiled ??= (async () => {
    try {
      return await WebAssembly.compileStreaming(fetch(wasmUrl))
    } catch {
      try {
        return await WebAssembly.compile(await (await fetch(wasmUrl)).arrayBuffer())
      } catch {
        return undefined // the worker fetches it itself
      }
    }
  })()
  return compiled
}

type Pending = { resolve: (v: unknown) => void; reject: (e: unknown) => void; done?: () => void }

function startWorker() {
  runningWorkers++
  // The worker script loads while the WebAssembly compiles.
  const worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module', name: 'pdfium' })
  void compileOnce().then((module) => {
    const boot: Boot = { boot: true, module, wasmUrl: new URL(wasmUrl, location.href).href }
    worker.postMessage(boot)
  })
  return worker
}

export class PdfEngine {
  private worker: Worker | null
  private pending = new Map<number, Pending>()
  private seq = 0
  private destroyed = false
  /** While suspended there's no worker; the next call starts one and reopens these bytes. */
  private suspended: { bytes: Uint8Array; password: string; onResume: (r: OpenResult) => void } | null = null
  private resuming: Promise<void> | null = null

  private constructor(worker: Worker) {
    this.worker = worker
    this.attach(worker)
  }

  private attach(worker: Worker) {
    worker.onmessage = (e: MessageEvent<Response>) => this.onResponse(e.data)
    worker.onerror = (e) => {
      const err = new Error(e.message || 'The PDF engine stopped unexpectedly.')
      for (const p of this.pending.values()) p.reject(err)
      this.pending.clear()
    }
  }

  static async create(): Promise<PdfEngine> {
    const engine = new PdfEngine(startWorker())
    if (import.meta.env.DEV || import.meta.env.VITE_TEST_HOOKS) testEngines.add(engine)
    return engine
  }

  /**
   * Stop the worker and hand all its memory back (for a background tab with nothing unsaved).
   * It comes back by itself, reopening `bytes`, the next time anything is asked of it.
   */
  suspend(bytes: Uint8Array, password: string, onResume: (r: OpenResult) => void) {
    if (this.destroyed || this.suspended || !this.worker) return
    this.worker.terminate()
    this.worker = null
    workerStopped()
    for (const p of this.pending.values()) p.reject(new Cancelled())
    this.pending.clear()
    this.suspended = { bytes, password, onResume }
  }

  private resume() {
    this.resuming ??= (async () => {
      const s = this.suspended!
      const worker = startWorker()
      if (this.destroyed) {
        worker.terminate()
        return workerStopped()
      }
      this.worker = worker
      this.attach(worker)
      this.suspended = null
      const result = await this.call('open', [s.bytes.slice().buffer, s.password])
      s.onResume(result)
    })().finally(() => {
      this.resuming = null
    })
    return this.resuming
  }

  async call<M extends Method>(method: M, args: Parameters<Api[M]>, opts: { priority?: number; signal?: AbortSignal } = {}): Promise<Awaited<ReturnType<Api[M]>>> {
    if (this.destroyed) throw new Cancelled()
    if (this.suspended || (this.resuming && method !== 'open')) await this.resume()
    return this.send(method, args, opts)
  }

  private send<M extends Method>(method: M, args: Parameters<Api[M]>, opts: { priority?: number; signal?: AbortSignal }): Promise<Awaited<ReturnType<Api[M]>>> {
    const worker = this.worker
    if (this.destroyed || !worker) return Promise.reject(new Cancelled())
    const { priority = Priority.action, signal } = opts
    if (signal?.aborted) return Promise.reject(new Cancelled())
    const id = ++this.seq
    return new Promise((resolve, reject) => {
      const onAbort = () => {
        if (!this.pending.delete(id)) return
        // Drop it from the worker's queue. If it's already running, its result is thrown away.
        worker.postMessage({ cancel: id } satisfies Request)
        reject(new Cancelled())
      }
      signal?.addEventListener('abort', onAbort, { once: true })
      this.pending.set(id, {
        resolve: resolve as (v: unknown) => void,
        reject,
        done: () => signal?.removeEventListener('abort', onAbort),
      })
      const transfer = args.filter((a): a is ArrayBuffer => a instanceof ArrayBuffer)
      worker.postMessage({ id, method, args, priority } satisfies Request, transfer)
    })
  }

  private onResponse(res: Response) {
    const p = this.pending.get(res.id)
    this.pending.delete(res.id)
    if (!p) {
      // Cancelled after the worker had started: free any bitmap it sent.
      discardBitmap(res.result as Bitmap | undefined)
      return
    }
    p.done?.()
    if (res.error) {
      if (res.error.code === -1) p.reject(new Cancelled())
      else p.reject(Object.assign(new Error(res.error.message), { code: res.error.code }))
    } else p.resolve(res.result)
  }

  destroy() {
    if (this.destroyed) return
    this.destroyed = true
    testEngines.delete(this)
    if (this.worker) {
      this.worker.terminate()
      workerStopped()
    }
    this.worker = null
    for (const p of this.pending.values()) p.reject(new Cancelled())
    this.pending.clear()
  }
}

/** Free a bitmap that arrived too late to be used. */
export function discardBitmap(bitmap: Bitmap | undefined) {
  for (const band of bitmap?.bands ?? []) band.image?.close()
}

export const isCancelled = (err: unknown) => err instanceof Cancelled
