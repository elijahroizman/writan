// What every kind of document's tools share: the errors an agent can act on, reading
// arguments, and what a tool answers with.

export class ToolError extends Error {}

export type Args = Record<string, unknown>

/** A picture in an answer, for MCP's image content. */
export interface Picture {
  /** Base64. */
  data: string
  mimeType: string
}

/**
 * What a tool answers. `after`, when there is one, runs once the change has reached the others,
 * so what it says is the shared document's state: an edit says what it changed.
 */
export interface Answer {
  text: string
  pictures?: Picture[]
  after?: () => string | Promise<string>
}

export interface Tool {
  name: string
  description: string
  inputSchema: Record<string, unknown>
}

export const doc = { type: 'string', description: 'The document, as list_documents gives it.' }

export const str = (a: Args, k: string) => (typeof a[k] === 'string' ? (a[k] as string) : undefined)
export const num = (a: Args, k: string) => {
  const v = a[k]
  if (typeof v === 'number' && Number.isFinite(v)) return v
  // Some clients send numbers as strings.
  if (typeof v === 'string' && v.trim() && Number.isFinite(Number(v))) return Number(v)
  return undefined
}
export const int = (a: Args, k: string) => {
  const v = num(a, k)
  return v === undefined ? undefined : Math.trunc(v)
}
export const bool = (a: Args, k: string) => {
  const v = a[k]
  if (typeof v === 'boolean') return v
  if (v === 'true') return true
  if (v === 'false') return false
  return undefined
}

export function need<T>(v: T | undefined, what: string): T {
  if (v === undefined) throw new ToolError(`Give ${what}.`)
  return v
}

/** Runs commands as one change, which undo takes back at once. */
export function grouped<T>(e: { run(c: { cmd: string }): unknown }, f: () => T): T {
  e.run({ cmd: 'beginGroup' })
  try {
    return f()
  } finally {
    e.run({ cmd: 'endGroup' })
  }
}

/** `#rrggbb` from `#RGB`, `rrggbb` or a few color names; `null` for none. */
export function hexColor(v: string, what: string): string {
  const s = v.trim().toLowerCase()
  const named: Record<string, string> = {
    black: '000000',
    white: 'ffffff',
    red: 'ff0000',
    green: '00a000',
    blue: '0000ff',
    yellow: 'ffff00',
    orange: 'ff8c00',
    purple: '800080',
    gray: '808080',
    grey: '808080',
  }
  if (named[s]) return named[s]
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/.exec(s)
  if (!m) throw new ToolError(`${what} “${v}” isn’t a color. Give one like "#1F4E79".`)
  return m[1].length === 3 ? [...m[1]].map((c) => c + c).join('') : m[1]
}

/** A short version of long text, for answers. */
export function clip(text: string, max = 160) {
  const t = text.replace(/\n/g, ' / ')
  return t.length > max ? `${t.slice(0, max - 1)}…` : t
}

export const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`

/** A canvas as a picture for an answer. */
export async function canvasPicture(canvas: HTMLCanvasElement, type: 'image/png' | 'image/jpeg' = 'image/jpeg'): Promise<Picture> {
  const blob = await new Promise<Blob>((resolve, reject) => canvas.toBlob((b) => (b ? resolve(b) : reject(new ToolError('Couldn’t draw the picture.'))), type, 0.88))
  const bytes = new Uint8Array(await blob.arrayBuffer())
  let bin = ''
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000))
  return { data: btoa(bin), mimeType: type }
}
