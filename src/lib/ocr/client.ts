import { ensurePacks, normalizeOcrLangs, ocrPackRevision, useOcrPacks } from './packs'
import type { OcrPageInput, OcrPageResult, OcrRecognizeOptions } from './types'
import type { OcrWorkerCall, OcrWorkerReply } from './worker'

const IDLE_MS = 30_000
type WorkerSuccess = Exclude<OcrWorkerReply, { kind: 'error' }>

type FakeEngine = (page: OcrPageInput, options: OcrRecognizeOptions) => OcrPageResult | Promise<OcrPageResult>

function fakeEngine(): FakeEngine | undefined {
  if (import.meta.env.VITE_TEST_HOOKS) {
    return (window as Window & { __ocrFakeEngine?: FakeEngine }).__ocrFakeEngine
  }
}

export class OcrClient {
  private worker?: Worker
  private initialized?: { key: string; revision: number }
  private idleTimer?: ReturnType<typeof setTimeout>
  private active?: AbortController
  private jobs = new Set<AbortController>()
  private tail: Promise<void> = Promise.resolve()
  private nextId = 0

  recognize(page: OcrPageInput, options: OcrRecognizeOptions = {}): Promise<OcrPageResult> {
    return new Promise((resolve, reject) => {
      const langs = normalizeOcrLangs(options.langs ?? useOcrPacks.getState().selectedLangs)
      const controller = new AbortController()
      const signal = controller.signal
      const source = options.signal
      const cancel = () => controller.abort(source?.reason)
      if (source?.aborted) {
        reject(source.reason)
        return
      }
      source?.addEventListener('abort', cancel, { once: true })
      this.jobs.add(controller)
      const finish = () => {
        source?.removeEventListener('abort', cancel)
        this.jobs.delete(controller)
      }
      signal.addEventListener('abort', () => {
        if (this.active === controller) this.stopWorker()
        finish()
        reject(signal.reason)
      }, { once: true })

      this.tail = this.tail.then(async () => {
        if (signal.aborted) return
        this.active = controller
        clearTimeout(this.idleTimer)
        try {
          const fake = fakeEngine()
          const result = fake
            ? await fake(page, { langs, signal })
            : await this.recognizeWithWorker(page, langs, signal)
          signal.throwIfAborted()
          resolve(result)
        } catch (error) {
          reject(error)
        } finally {
          finish()
          this.active = undefined
          if (this.worker) this.idleTimer = setTimeout(() => this.stopWorker(), IDLE_MS)
        }
      })
    })
  }

  private async recognizeWithWorker(page: OcrPageInput, langs: string[], signal: AbortSignal): Promise<OcrPageResult> {
    const key = langs.join('+')
    const revision = ocrPackRevision()
    if (this.initialized?.key !== key || this.initialized.revision !== revision) {
      this.stopWorker()
      const packs = await ensurePacks(langs, signal)
      signal.throwIfAborted()
      this.worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module', name: 'writan-ocr' })
      try {
        const ready = await this.call({ kind: 'initialize', packs }, packs.map(({ bytes }) => bytes.buffer), signal)
        if (ready.kind !== 'ready') throw new Error('Unexpected OCR worker response.')
        if (revision !== ocrPackRevision()) throw new Error('OCR language packs changed. Try recognizing the page again.')
        this.initialized = { key, revision }
      } catch (error) {
        this.stopWorker()
        throw error
      }
    }
    const canTransfer =
      !!page.transfer &&
      page.data.byteOffset === 0 &&
      page.data.byteLength === page.data.buffer.byteLength &&
      page.data.buffer instanceof ArrayBuffer
    // Copy by default so callers retain their original pixels; transfer straight when explicitly owned.
    const data = canTransfer ? page.data : new Uint8Array(page.data)
    // ImageData exposes width and height on its prototype.
    const { width, height, channels, pageIndex, widthPt, heightPt } = page
    const reply = await this.call({
      kind: 'recognize', page: { data, width, height, channels, pageIndex, widthPt, heightPt },
    }, [data.buffer], signal)
    if (reply.kind !== 'result') throw new Error('Unexpected OCR worker response.')
    return reply.page
  }

  private call(request: OcrWorkerCall, transfers: Transferable[], signal: AbortSignal): Promise<WorkerSuccess> {
    return new Promise((resolve, reject) => {
      signal.throwIfAborted()
      const worker = this.worker
      if (!worker) throw new Error('OCR worker is unavailable.')
      const id = ++this.nextId
      const abort = () => settle(() => reject(signal.reason))
      const failed = (event: Event) => settle(() => {
        this.stopWorker()
        const message = 'message' in event && typeof event.message === 'string' ? event.message : ''
        reject(new Error(message || 'The OCR worker stopped unexpectedly.'))
      })
      const received = ({ data }: MessageEvent<OcrWorkerReply>) => {
        if (data.id !== id) return
        settle(() => {
          if (data.kind === 'error') reject(new Error(data.message))
          else resolve(data)
        })
      }
      function settle(finish: () => void) {
        signal.removeEventListener('abort', abort)
        worker?.removeEventListener('message', received)
        worker?.removeEventListener('error', failed)
        worker?.removeEventListener('messageerror', failed)
        finish()
      }
      signal.addEventListener('abort', abort, { once: true })
      worker.addEventListener('message', received)
      worker.addEventListener('error', failed)
      worker.addEventListener('messageerror', failed)
      try {
        worker.postMessage({ id, ...request }, transfers)
      } catch (error) {
        settle(() => reject(error))
      }
    })
  }

  private stopWorker() {
    clearTimeout(this.idleTimer)
    this.worker?.terminate()
    this.worker = undefined
    this.initialized = undefined
  }

  dispose(): void {
    for (const controller of this.jobs) controller.abort(new DOMException('OCR client was disposed.', 'AbortError'))
    this.stopWorker()
  }
}
