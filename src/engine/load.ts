// Loads the document engine (WebAssembly, built from crates/office-wasm by scripts/engine.mjs)
// once, the first time a Word, PowerPoint or Excel document opens.

import init, * as wasm from './wasm/office_wasm'
import wasmUrl from './wasm/office_wasm_bg.wasm?url'
import './fonts/fonts.css'
import { noteEngineStarted } from '../lib/analytics/documents'

export type Engine = typeof wasm

let ready: Promise<Engine> | null = null

export function loadEngine(): Promise<Engine> {
  if (!ready) noteEngineStarted()
  ready ??= init({ module_or_path: wasmUrl }).then(() => wasm)
  return ready
}

const KINDS = { word: ['word/', 'a Word document', '.docx'], slides: ['ppt/', 'a PowerPoint presentation', '.pptx'], sheets: ['xl/', 'an Excel workbook', '.xlsx'] } as const
type OfficeKind = keyof typeof KINDS

/**
 * Refuse an Office file whose contents are another kind than its editor expects (a Word document
 * renamed .pptx), instead of showing it as an empty deck that a save would write over. Reads only
 * the zip's list of names; damaged files are left for the editor to report.
 */
export function expectKind(bytes: Uint8Array | null, expected: OfficeKind) {
  if (bytes && bytes.length === 0) throw new Error('This file is empty. It does not contain an Office document.')
  if (bytes && isEncrypted(bytes)) throw new Error('This file is protected with a password. Writan can’t open password-protected Office files yet.')
  const kind = bytes && officeKind(bytes)
  if (!kind || kind === expected) return
  throw new Error(`This file is ${KINDS[kind][1]}, not ${KINDS[expected][1]}. Rename it to end in ${KINDS[kind][2]} to open it.`)
}

/** A password-protected Office file: a compound file holding an EncryptedPackage stream. */
function isEncrypted(bytes: Uint8Array) {
  if (bytes.length < 512 || bytes[0] !== 0xd0 || bytes[1] !== 0xcf || bytes[2] !== 0x11 || bytes[3] !== 0xe0) return false
  const name = [...'EncryptedPackage'].flatMap((c) => [c.charCodeAt(0), 0])
  for (let i = bytes.indexOf(name[0]); i >= 0; i = bytes.indexOf(name[0], i + 1)) {
    if (name.every((b, j) => bytes[i + j] === b)) return true
  }
  return false
}

/** Which Office kind a zip holds, from its top folders (word/, ppt/ or xl/); null if unsure. */
function officeKind(bytes: Uint8Array): OfficeKind | null {
  if (bytes.length < 22 || bytes[0] !== 0x50 || bytes[1] !== 0x4b) return null
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  // The end-of-central-directory record: the last 22 bytes, before a comment of up to 64 KB.
  let end = -1
  for (let i = bytes.length - 22; i >= Math.max(0, bytes.length - 22 - 0xffff); i--) {
    if (view.getUint32(i, true) === 0x06054b50) {
      end = i
      break
    }
  }
  if (end < 0) return null
  const found = new Set<OfficeKind>()
  let at = view.getUint32(end + 16, true)
  const decoder = new TextDecoder()
  for (let n = view.getUint16(end + 10, true); n > 0 && at + 46 <= bytes.length; n--) {
    if (view.getUint32(at, true) !== 0x02014b50) return null
    const nameLength = view.getUint16(at + 28, true)
    const name = decoder.decode(bytes.subarray(at + 46, at + 46 + nameLength)).replace(/^\//, '')
    for (const kind of Object.keys(KINDS) as OfficeKind[]) if (name.startsWith(KINDS[kind][0])) found.add(kind)
    at += 46 + nameLength + view.getUint16(at + 30, true) + view.getUint16(at + 32, true)
  }
  return found.size === 1 ? [...found][0] : null
}
