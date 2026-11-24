// Pictures an agent puts in a document: from a file, an address or base64, checked to be a
// picture, and scaled down when far bigger than the document could show.

import { shrinkPicture } from '../../lib/pictures'
import { fetchBytes, readFile } from '../../lib/platform'
import { type Args, str, ToolError } from './common'

/**
 * A picture's bytes from what the agent gave. A file or an address is read by this computer, so
 * only an agent on it can name one; an agent elsewhere sends the bytes.
 */
export async function pictureBytes(args: Args, local: boolean): Promise<{ bytes: Uint8Array; name: string }> {
  const path = str(args, 'path')
  const url = str(args, 'url')
  const data = str(args, 'data')
  if ((path || url) && !local) throw new ToolError('Agents on another computer can only send a picture as data (base64). path and url are for agents on the computer Writan runs on.')
  if (path) {
    let bytes: Uint8Array
    try {
      bytes = await readFile(path)
    } catch (err) {
      throw new ToolError(`Couldn’t read ${path}: ${String((err as Error)?.message ?? err)}`)
    }
    return { bytes, name: path.split(/[\\/]/).pop() || 'Picture' }
  }
  if (url) {
    if (!/^https?:\/\//i.test(url)) throw new ToolError('Give an http or https address.')
    let bytes: Uint8Array
    try {
      bytes = await fetchBytes(url)
    } catch (err) {
      throw new ToolError(`Couldn’t download ${url}: ${String((err as Error)?.message ?? err)}`)
    }
    let name = 'Picture'
    try {
      name = decodeURIComponent(new URL(url).pathname.split('/').pop() || 'Picture')
    } catch {
      // A name that doesn't decode stays "Picture".
    }
    return { bytes, name }
  }
  if (data) {
    let bin: string
    try {
      bin = atob(data.replace(/^data:[^,]*,/, '').replace(/\s+/g, ''))
    } catch {
      throw new ToolError('data isn’t base64.')
    }
    const bytes = new Uint8Array(bin.length)
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
    return { bytes, name: 'Picture' }
  }
  throw new ToolError('Give the picture as path, url or data.')
}

/** The picture's type from its first bytes, for the kinds a document can show; `null` for anything else. */
export function pictureType(b: Uint8Array): string | null {
  const is = (sig: number[], at = 0) => sig.every((v, i) => b[at + i] === v)
  if (is([0x89, 0x50, 0x4e, 0x47])) return 'image/png'
  if (is([0xff, 0xd8, 0xff])) return 'image/jpeg'
  if (is([0x47, 0x49, 0x46, 0x38])) return 'image/gif'
  if (is([0x52, 0x49, 0x46, 0x46]) && is([0x57, 0x45, 0x42, 0x50], 8)) return 'image/webp'
  const text = new TextDecoder().decode(b.subarray(0, 1024)).trimStart()
  if (/^(<\?xml[^>]*>\s*)?(<!--[\s\S]*?-->\s*)*(<!DOCTYPE svg[^>]*>\s*)?<svg[\s>]/i.test(text)) return 'image/svg+xml'
  return null
}

/**
 * The picture, opened to check it and measure it. One far bigger than `maxW` × `maxH` is scaled
 * down to fit, as the editor does with a phone photo; smaller ones are kept byte for byte.
 */
export async function scaledPicture(bytes: Uint8Array, type: string, maxW: number, maxH: number): Promise<{ bytes: Uint8Array; type: string; width: number; height: number }> {
  const pic = await shrinkPicture(bytes, type, maxW, maxH)
  if (!pic) throw new ToolError('That picture can’t be opened. Give a PNG, JPEG, GIF, WebP or SVG.')
  return pic
}
