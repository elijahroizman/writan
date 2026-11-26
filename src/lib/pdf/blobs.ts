const encoder = new TextEncoder()

/**
 * Packs named binary blobs into the compact little-endian container decoded by
 * `office_pdf::unpack_blobs`:
 *   [u32 count] + count * ([u32 keyLen][key utf8][u32 dataLen][data])
 */
export function packBlobs(entries: Iterable<[string, Uint8Array]>): Uint8Array {
  const list: Array<[Uint8Array, Uint8Array]> = []
  let total = 4
  for (const [key, data] of entries) {
    const k = encoder.encode(key)
    list.push([k, data])
    total += 8 + k.byteLength + data.byteLength
  }
  const out = new Uint8Array(total)
  const view = new DataView(out.buffer, out.byteOffset, out.byteLength)
  view.setUint32(0, list.length, true)
  let offset = 4
  for (const [k, data] of list) {
    view.setUint32(offset, k.byteLength, true)
    offset += 4
    out.set(k, offset)
    offset += k.byteLength
    view.setUint32(offset, data.byteLength, true)
    offset += 4
    out.set(data, offset)
    offset += data.byteLength
  }
  return out
}
