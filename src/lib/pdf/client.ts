import type { PdfWorkerRequest, PdfWorkerResponse } from './worker'

export interface PdfInspection {
  pageCount: number
  encrypted: boolean
  hasDigitalSignature: boolean
  signatureFields: string[]
  hasForms: boolean
  hasOutlines: boolean
  annotationCount: number
}

const IDLE_MS = 30_000

function toTransferCopy(bytes: Uint8Array): Uint8Array {
  return bytes.slice()
}

export class PdfEngineClient {
  private worker?: Worker
  private idleTimer?: ReturnType<typeof setTimeout>
  private nextId = 0
  private pending = new Map<number, { resolve: (res: PdfWorkerResponse) => void; reject: (err: unknown) => void }>()

  private ensureWorker(): Worker {
    clearTimeout(this.idleTimer)
    if (this.worker) return this.worker
    const worker = new Worker(new URL('./worker.ts', import.meta.url), {
      type: 'module',
      name: 'writan-pdf-export',
    })
    worker.onmessage = (event: MessageEvent<PdfWorkerResponse>) => {
      const res = event.data
      const entry = this.pending.get(res.id)
      if (!entry) return
      this.pending.delete(res.id)
      this.scheduleIdle()
      if (res.kind === 'error') {
        entry.reject(new Error(res.message))
      } else {
        entry.resolve(res)
      }
    }
    worker.onerror = (event) => {
      const err = new Error(event.message || 'The PDF export worker stopped unexpectedly.')
      this.failAll(err)
      this.stopWorker()
    }
    worker.onmessageerror = () => {
      const err = new Error('The PDF export worker returned unreadable data.')
      this.failAll(err)
      this.stopWorker()
    }
    this.worker = worker
    return worker
  }

  private scheduleIdle() {
    clearTimeout(this.idleTimer)
    if (this.pending.size === 0 && this.worker) {
      this.idleTimer = setTimeout(() => this.stopWorker(), IDLE_MS)
    }
  }

  private failAll(err: unknown) {
    for (const entry of this.pending.values()) entry.reject(err)
    this.pending.clear()
  }

  private stopWorker() {
    clearTimeout(this.idleTimer)
    this.worker?.terminate()
    this.worker = undefined
  }

  private call(request: DistributiveOmit<PdfWorkerRequest, 'id'>, transfers: Transferable[], signal?: AbortSignal): Promise<PdfWorkerResponse> {
    if (signal?.aborted) {
      return Promise.reject(signal.reason instanceof Error ? signal.reason : new DOMException('Cancelled', 'AbortError'))
    }
    const worker = this.ensureWorker()
    const id = ++this.nextId
    return new Promise((resolve, reject) => {
      const cleanup = () => signal?.removeEventListener('abort', onAbort)
      const onAbort = () => {
        this.pending.delete(id)
        this.stopWorker()
        cleanup()
        reject(signal?.reason instanceof Error ? signal.reason : new DOMException('Cancelled', 'AbortError'))
      }
      signal?.addEventListener('abort', onAbort, { once: true })
      this.pending.set(id, {
        resolve: (res) => {
          cleanup()
          resolve(res)
        },
        reject: (err) => {
          cleanup()
          reject(err)
        },
      })
      try {
        worker.postMessage({ id, ...request } as PdfWorkerRequest, transfers)
      } catch (err) {
        this.pending.delete(id)
        cleanup()
        reject(err)
      }
    })
  }

  async renderDocument(docJson: string, mediaPack: Uint8Array, fontPack: Uint8Array, signal?: AbortSignal): Promise<Uint8Array> {
    const m = toTransferCopy(mediaPack)
    const f = toTransferCopy(fontPack)
    const res = await this.call({ kind: 'renderDocument', docJson, mediaPack: m, fontPack: f }, [m.buffer, f.buffer], signal)
    if (res.kind !== 'bytes') throw new Error('Unexpected response from PDF worker.')
    return res.bytes
  }

  async imagesToPdf(pagesJson: string, mediaPack: Uint8Array, fontPack: Uint8Array, signal?: AbortSignal): Promise<Uint8Array> {
    const m = toTransferCopy(mediaPack)
    const f = toTransferCopy(fontPack)
    const res = await this.call({ kind: 'imagesToPdf', pagesJson, mediaPack: m, fontPack: f }, [m.buffer, f.buffer], signal)
    if (res.kind !== 'bytes') throw new Error('Unexpected response from PDF worker.')
    return res.bytes
  }

  async inspectPdf(pdfBytes: Uint8Array, signal?: AbortSignal): Promise<PdfInspection> {
    const copy = toTransferCopy(pdfBytes)
    const res = await this.call({ kind: 'inspectPdf', pdfBytes: copy }, [copy.buffer], signal)
    if (res.kind !== 'json') throw new Error('Unexpected response from PDF worker.')
    return JSON.parse(res.json) as PdfInspection
  }

  async injectOcrTextLayer(
    pdfBytes: Uint8Array,
    layersJson: string,
    fontPack: Uint8Array,
    opts: { signal?: AbortSignal; allowSigned?: boolean } = {},
  ): Promise<Uint8Array> {
    const copy = toTransferCopy(pdfBytes)
    const f = toTransferCopy(fontPack)
    const res = await this.call(
      { kind: 'injectOcrTextLayer', pdfBytes: copy, layersJson, fontPack: f, allowSigned: opts.allowSigned },
      [copy.buffer, f.buffer],
      opts.signal,
    )
    if (res.kind !== 'bytes') throw new Error('Unexpected response from PDF worker.')
    return res.bytes
  }

  dispose(): void {
    this.failAll(new DOMException('PDF engine client disposed', 'AbortError'))
    this.stopWorker()
  }
}

type DistributiveOmit<T, K extends keyof T> = T extends unknown ? Omit<T, K> : never

export const pdfEngineClient = new PdfEngineClient()
