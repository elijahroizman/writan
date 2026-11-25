/// <reference lib="webworker" />

import initWasm, {
  imagesToPdf,
  injectOcrTextLayer,
  inspectPdf,
  renderDocumentToPdf,
} from './wasm/office_pdf_wasm.js'

export type PdfWorkerRequest =
  | { id: number; kind: 'renderDocument'; docJson: string; mediaPack: Uint8Array; fontPack: Uint8Array }
  | { id: number; kind: 'imagesToPdf'; pagesJson: string; mediaPack: Uint8Array; fontPack: Uint8Array }
  | { id: number; kind: 'inspectPdf'; pdfBytes: Uint8Array }
  | { id: number; kind: 'injectOcrTextLayer'; pdfBytes: Uint8Array; layersJson: string; fontPack: Uint8Array; allowSigned?: boolean }

export type PdfWorkerResponse =
  | { id: number; kind: 'bytes'; bytes: Uint8Array }
  | { id: number; kind: 'json'; json: string }
  | { id: number; kind: 'error'; message: string }

let ready: Promise<unknown> | null = null

function ensureReady(): Promise<unknown> {
  ready ??= initWasm()
  return ready
}

self.onmessage = async (event: MessageEvent<PdfWorkerRequest>) => {
  const msg = event.data
  try {
    await ensureReady()
    switch (msg.kind) {
      case 'renderDocument': {
        const bytes = renderDocumentToPdf(msg.docJson, msg.mediaPack, msg.fontPack)
        const reply: PdfWorkerResponse = { id: msg.id, kind: 'bytes', bytes }
        self.postMessage(reply, [bytes.buffer])
        return
      }
      case 'imagesToPdf': {
        const bytes = imagesToPdf(msg.pagesJson, msg.mediaPack, msg.fontPack)
        const reply: PdfWorkerResponse = { id: msg.id, kind: 'bytes', bytes }
        self.postMessage(reply, [bytes.buffer])
        return
      }
      case 'inspectPdf': {
        const json = inspectPdf(msg.pdfBytes)
        const reply: PdfWorkerResponse = { id: msg.id, kind: 'json', json }
        self.postMessage(reply)
        return
      }
      case 'injectOcrTextLayer': {
        const bytes = injectOcrTextLayer(msg.pdfBytes, msg.layersJson, msg.fontPack, msg.allowSigned ?? false)
        const reply: PdfWorkerResponse = { id: msg.id, kind: 'bytes', bytes }
        self.postMessage(reply, [bytes.buffer])
        return
      }
    }
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    const reply: PdfWorkerResponse = { id: msg.id, kind: 'error', message }
    self.postMessage(reply)
  }
}
