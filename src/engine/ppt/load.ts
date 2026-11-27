// Legacy binary PowerPoint conversion lives in its own WebAssembly module: office_oxide is large,
// and nearly every document is already .pptx, so the editor's engine does not carry it.

import init, { convertLegacyPpt } from './wasm/office_ppt_wasm'
import wasmUrl from './wasm/office_ppt_wasm_bg.wasm?url'

let ready: Promise<unknown> | null = null

function load(): Promise<unknown> {
  ready ??= init({ module_or_path: wasmUrl })
  return ready
}

/** A compound document, the container legacy `.ppt` files use. */
export function isLegacyPpt(bytes: Uint8Array): boolean {
  return bytes.length >= 4 && bytes[0] === 0xd0 && bytes[1] === 0xcf && bytes[2] === 0x11 && bytes[3] === 0xe0
}

/** Converts a legacy binary PowerPoint file to `.pptx` bytes. */
export async function legacyPptToPptx(bytes: Uint8Array): Promise<Uint8Array> {
  await load()
  return convertLegacyPpt(bytes)
}
