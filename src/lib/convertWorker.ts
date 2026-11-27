/// <reference lib="webworker" />

import initWasm, { convertDocument, toMarkdown } from '../engine/wasm/office_wasm'
import { isLegacyPpt, legacyPptToPptx } from '../engine/ppt/load'
import wasmUrl from '../engine/wasm/office_wasm_bg.wasm?url'

export type ConvertWorkerRequest =
  | { id: number; kind: 'toMarkdown'; bytes: Uint8Array; name: string }
  | { id: number; kind: 'convertDocument'; bytes: Uint8Array; name: string; target: string }

export type ConvertWorkerResponse =
  | { id: number; kind: 'text'; text: string }
  | { id: number; kind: 'bytes'; bytes: Uint8Array }
  | { id: number; kind: 'error'; message: string }

let ready: Promise<unknown> | null = null

function ensureReady(): Promise<unknown> {
  ready ??= initWasm({ module_or_path: wasmUrl })
  return ready
}

self.onmessage = async (event: MessageEvent<ConvertWorkerRequest>) => {
  const msg = event.data
  try {
    await ensureReady()
    let bytes = msg.bytes
    let name = msg.name
    if (isLegacyPpt(bytes)) {
      bytes = await legacyPptToPptx(bytes)
      name = name.replace(/\.ppt$/i, '.pptx')
    }
    if (msg.kind === 'toMarkdown') {
      const reply: ConvertWorkerResponse = { id: msg.id, kind: 'text', text: toMarkdown(bytes, name) }
      self.postMessage(reply)
      return
    }
    bytes = convertDocument(bytes, name, msg.target)
    const reply: ConvertWorkerResponse = { id: msg.id, kind: 'bytes', bytes }
    self.postMessage(reply, [bytes.buffer])
  } catch (err) {
    const reply: ConvertWorkerResponse = { id: msg.id, kind: 'error', message: err instanceof Error ? err.message : String(err) }
    self.postMessage(reply)
  }
}
