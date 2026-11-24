// Pictures going into a document, scaled down when far bigger than the document could show.

export interface Picture {
  bytes: Uint8Array
  type: string
  /** In pixels. */
  width: number
  height: number
}

/**
 * The picture, measured, and scaled down to fit `maxW` × `maxH` pixels when it's far bigger (a
 * phone photo, a Retina screenshot). That keeps the document, its pages and the saved file quick,
 * and in a shared document it's what has to reach everyone before they see the picture. Smaller
 * ones, GIFs, SVGs and any the scaling wouldn't make smaller are kept byte for byte. Null when the
 * browser can't open it.
 */
export async function shrinkPicture(bytes: Uint8Array, type: string, maxW: number, maxH: number): Promise<Picture | null> {
  const url = URL.createObjectURL(new Blob([bytes as Uint8Array<ArrayBuffer>], { type }))
  try {
    const img = new Image()
    img.src = url
    try {
      await img.decode()
    } catch {
      return null
    }
    const width = img.naturalWidth || 400
    const height = img.naturalHeight || 300
    const scale = Math.min(1, maxW / width, maxH / height)
    if (scale > 0.75 || type === 'image/gif' || type === 'image/svg+xml') return { bytes, type, width, height }
    const out = document.createElement('canvas')
    out.width = Math.max(1, Math.round(width * scale))
    out.height = Math.max(1, Math.round(height * scale))
    out.getContext('2d')!.drawImage(img, 0, 0, out.width, out.height)
    // Photos stay JPEG; anything that may have transparency stays PNG.
    const kind = type === 'image/jpeg' ? 'image/jpeg' : 'image/png'
    const small = await new Promise<Blob | null>((res) => out.toBlob(res, kind, 0.9))
    // A WebP made PNG can come out bigger than it went in: then it goes as it was.
    if (!small || small.size >= bytes.length) return { bytes, type, width, height }
    return { bytes: new Uint8Array(await small.arrayBuffer()), type: kind, width: out.width, height: out.height }
  } finally {
    URL.revokeObjectURL(url)
  }
}
