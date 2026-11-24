// Community color themes from Open VSX (open-vsx.org), the open registry of VS Code extensions:
// searching it, and turning a theme extension into Writan themes. Its limits and checks follow
// T3 Code's Open VSX import (MIT, © 2026 T3 Tools Inc.): only openly licensed extensions, the
// package's checksum checked, and its ZIP looked over before it's opened.
//
// Requests go through `fetchBytes`, which in the app is Rust's (the page's own requests may only
// reach Writan).

import type JSZip from 'jszip'
import { fetchBytes } from './platform'
import type { ThemeDef } from './themes'
import { otherTone, parseThemeFile, stripJsonComments } from './themes'

const API = 'https://open-vsx.org/api'
const MAX_JSON_BYTES = 512 * 1024
const MAX_THEME_BYTES = 256 * 1024
const MAX_ZIP_ENTRIES = 5000
const MAX_UNCOMPRESSED_BYTES = 100 * 1024 * 1024
const MAX_COMPRESSION_RATIO = 200
const MAX_THEMES = 40
const MAX_INCLUDE_DEPTH = 8

/** Licenses that let anyone use the colors. Other extensions aren't shown. */
const LICENSES = new Set(['0BSD', 'Apache-2.0', 'BSD-2-Clause', 'BSD-3-Clause', 'CC0-1.0', 'ISC', 'MIT', 'MPL-2.0', 'Unlicense'])

export type OpenVsxSort = 'downloadCount' | 'rating' | 'timestamp' | 'relevance'

export interface OpenVsxTheme {
  /** namespace.name */
  id: string
  name: string
  publisher: string
  description: string
  downloads: number
  iconUrl: string | null
  sourceUrl: string | null
  manifestUrl: string
  sha256Url: string
  vsixUrl: string
  version: string
  license: string
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v)

/** Only open-vsx.org's own https addresses are fetched from what it answers. */
function openVsxUrl(v: unknown): string | null {
  if (typeof v !== 'string') return null
  try {
    const url = new URL(v)
    return url.protocol === 'https:' && url.hostname.toLowerCase() === 'open-vsx.org' ? url.toString() : null
  } catch {
    return null
  }
}

function publicUrl(v: unknown): string | null {
  const raw = typeof v === 'string' ? v : isRecord(v) && typeof v.url === 'string' ? v.url : null
  if (!raw) return null
  try {
    const url = new URL(raw.replace(/^git\+/, '').replace(/\.git$/, ''))
    return url.protocol === 'https:' && !url.username && !url.password ? url.toString() : null
  } catch {
    return null
  }
}

async function getBytes(url: string, limit: number, what: string): Promise<Uint8Array> {
  let bytes: Uint8Array
  try {
    bytes = await fetchBytes(url)
  } catch {
    throw new Error(`Open VSX didn’t answer. Check your connection and try again.`)
  }
  if (bytes.byteLength > limit) throw new Error(`${what} is too big to import safely.`)
  return bytes
}

async function getJson(url: string, what: string): Promise<unknown> {
  const bytes = await getBytes(url, MAX_JSON_BYTES, what)
  try {
    return JSON.parse(new TextDecoder().decode(bytes))
  } catch {
    throw new Error(`Open VSX sent ${what.toLowerCase()} Writan can’t read.`)
  }
}

function themeContributions(manifest: Record<string, unknown>): Record<string, unknown>[] {
  const c = isRecord(manifest.contributes) ? manifest.contributes : null
  return Array.isArray(c?.themes) ? c.themes.filter(isRecord) : []
}

const licenseMatches = (manifest: Record<string, unknown>, license: string) => typeof manifest.license === 'string' && manifest.license.trim().toLowerCase() === license.toLowerCase()

function fromDetail(v: unknown): OpenVsxTheme | null {
  if (!isRecord(v) || !isRecord(v.files)) return null
  const namespace = typeof v.namespace === 'string' ? v.namespace.trim() : ''
  const name = typeof v.name === 'string' ? v.name.trim() : ''
  const version = typeof v.version === 'string' ? v.version.trim() : ''
  const license = typeof v.license === 'string' ? v.license.trim() : ''
  const manifestUrl = openVsxUrl(v.files.manifest)
  const sha256Url = openVsxUrl(v.files.sha256)
  const vsixUrl = openVsxUrl(v.files.download)
  if (!namespace || !name || !version || !manifestUrl || !sha256Url || !vsixUrl || !LICENSES.has(license)) return null
  return {
    id: `${namespace}.${name}`,
    name: (typeof v.displayName === 'string' && v.displayName.trim()) || name,
    publisher: namespace,
    description: typeof v.description === 'string' ? v.description : '',
    downloads: typeof v.downloadCount === 'number' && Number.isFinite(v.downloadCount) ? v.downloadCount : 0,
    iconUrl: openVsxUrl(v.files.icon),
    sourceUrl: publicUrl(v.repository) ?? publicUrl(v.homepage),
    manifestUrl,
    sha256Url,
    vsixUrl,
    version,
    license,
  }
}

/** Theme extensions matching `query`, up to eight, with open licenses only. */
export async function searchThemes(query: string, sortBy: OpenVsxSort = 'downloadCount', signal?: AbortSignal): Promise<OpenVsxTheme[]> {
  const q = query.trim()
  if (!q) return []
  const url = new URL(`${API}/-/search`)
  url.searchParams.set('query', q)
  url.searchParams.set('category', 'Themes')
  url.searchParams.set('sortBy', sortBy)
  url.searchParams.set('sortOrder', 'desc')
  url.searchParams.set('size', '16')
  const found = await getJson(url.toString(), 'The search')
  signal?.throwIfAborted()
  if (!isRecord(found) || !Array.isArray(found.extensions)) throw new Error('Open VSX sent a search Writan can’t read.')
  const names = found.extensions.flatMap((e): [string, string][] => (isRecord(e) && typeof e.namespace === 'string' && typeof e.name === 'string' ? [[e.namespace, e.name]] : []))
  const details = await Promise.allSettled(
    names.slice(0, 16).map(async ([namespace, name]) => {
      const ext = fromDetail(await getJson(`${API}/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}`, 'The theme’s details'))
      if (!ext) return null
      // The listing can say a theme's license is open while its package says otherwise.
      const manifest = await getJson(ext.manifestUrl, 'The theme’s manifest')
      return isRecord(manifest) && themeContributions(manifest).length > 0 && licenseMatches(manifest, ext.license) ? ext : null
    }),
  )
  signal?.throwIfAborted()
  const done = details.filter((d) => d.status === 'fulfilled')
  if (names.length && !done.length) throw new Error('Open VSX’s theme details aren’t available right now. Try again.')
  return done.flatMap((d) => (d.value ? [d.value] : [])).slice(0, 8)
}

/** Downloads an extension's icon, as a URL the page can show. */
export async function iconUrl(theme: OpenVsxTheme): Promise<string | null> {
  if (!theme.iconUrl) return null
  try {
    const bytes = await getBytes(theme.iconUrl, 512 * 1024, 'The icon')
    const ext = theme.iconUrl.split('.').pop()?.toLowerCase()
    const type = ext === 'svg' ? 'image/svg+xml' : ext === 'jpg' || ext === 'jpeg' ? 'image/jpeg' : 'image/png'
    // An SVG shown through <img> can't run scripts or load anything.
    return URL.createObjectURL(new Blob([bytes as BlobPart], { type }))
  } catch {
    return null
  }
}

async function sha256(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', bytes as BufferSource)
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('')
}

/** Checks a ZIP's directory before JSZip reads it: entry count, total size and compression ratio. */
function checkZip(bytes: Uint8Array): Uint8Array {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const min = Math.max(0, bytes.byteLength - 65_557)
  let end = bytes.byteLength - 22
  while (end >= min && (view.getUint32(end, true) !== 0x06054b50 || end + 22 + view.getUint16(end + 20, true) !== bytes.byteLength)) end--
  if (end < min) throw new Error('That theme’s package isn’t a ZIP file.')
  const size = view.getUint32(end + 12, true)
  const start = view.getUint32(end + 16, true)
  if (start + size !== end) throw new Error('That theme’s package is damaged.')
  let entries = 0
  let total = 0
  for (let at = start; at < end; ) {
    if (at + 46 > end || view.getUint32(at, true) !== 0x02014b50) throw new Error('That theme’s package is damaged.')
    if (++entries > MAX_ZIP_ENTRIES) throw new Error('That theme’s package has too many files.')
    const compressed = view.getUint32(at + 20, true)
    const uncompressed = view.getUint32(at + 24, true)
    if (compressed === 0xffffffff || uncompressed === 0xffffffff) throw new Error('That theme’s package is too big.')
    total += uncompressed
    if (total > MAX_UNCOMPRESSED_BYTES) throw new Error('That theme’s package is too big once unpacked.')
    if (uncompressed > 0 && (compressed === 0 || uncompressed / compressed > MAX_COMPRESSION_RATIO)) throw new Error('That theme’s package is compressed suspiciously.')
    at += 46 + view.getUint16(at + 28, true) + view.getUint16(at + 30, true) + view.getUint16(at + 32, true)
  }
  // Without its comment, which JSZip can mistake for the directory.
  if (!view.getUint16(end + 20, true)) return bytes
  const out = bytes.slice(0, end + 22)
  out[end + 20] = 0
  out[end + 21] = 0
  return out
}

/** A path inside the package, refusing one that leaves it. */
function packagePath(path: string, from = 'extension/'): string {
  if (path.length > 1024 || path.includes('\0') || path.startsWith('/') || /^[a-zA-Z]:/.test(path)) throw new Error('A theme’s path leaves its package.')
  const parts = from.split('/').slice(0, -1)
  for (const part of path.replaceAll('\\', '/').split('/')) {
    if (!part || part === '.') continue
    if (part === '..') {
      if (parts.length <= 1) throw new Error('A theme’s path leaves its package.')
      parts.pop()
    } else parts.push(part)
  }
  if (parts[0] !== 'extension') parts.unshift('extension')
  return parts.join('/')
}

async function zipText(zip: JSZip, path: string): Promise<string> {
  const file = zip.file(path)
  if (!file) throw new Error(`${path} is missing from the package.`)
  const bytes = await file.async('uint8array')
  if (bytes.byteLength > MAX_THEME_BYTES) throw new Error(`${path} is too big.`)
  return new TextDecoder().decode(bytes)
}

function jsonc(text: string): Record<string, unknown> {
  const v: unknown = JSON.parse(stripJsonComments(text))
  if (!isRecord(v)) throw new Error('A theme file isn’t a JSON object.')
  return v
}

/** A theme file's colors, following `include` to the files it builds on. */
async function themeColors(zip: JSZip, path: string, depth = 0, seen = new Set<string>()): Promise<Record<string, unknown>> {
  if (depth > MAX_INCLUDE_DEPTH || seen.has(path)) throw new Error('A theme includes itself.')
  seen.add(path)
  const file = jsonc(await zipText(zip, path))
  const own = isRecord(file.colors) ? file.colors : {}
  if (typeof file.include !== 'string' || !file.include.endsWith('.json')) return own
  return { ...(await themeColors(zip, packagePath(file.include, path), depth + 1, seen)), ...own }
}

const hashId = async (text: string) => `ovx-${(await sha256(new TextEncoder().encode(text))).slice(0, 10)}`

/**
 * Downloads a theme extension and makes Writan themes of its color themes. An extension with one
 * light and one dark theme becomes one Writan theme with both.
 */
export async function importExtension(ext: OpenVsxTheme): Promise<ThemeDef[]> {
  const pkg = await getBytes(ext.vsixUrl, 10 * 1024 * 1024, 'That theme')
  const expected = new TextDecoder().decode(await getBytes(ext.sha256Url, 256, 'The checksum')).trim().split(/\s+/)[0]
  if (!/^[a-f\d]{64}$/i.test(expected ?? '') || (await sha256(pkg)) !== expected.toLowerCase()) throw new Error('That theme’s download didn’t match its checksum, so it wasn’t added.')
  const { default: JSZip } = await import('jszip')
  const zip = await JSZip.loadAsync(checkZip(pkg))
  const manifest = jsonc(await zipText(zip, 'extension/package.json'))
  if (`${String(manifest.publisher)}.${String(manifest.name)}`.toLowerCase() !== ext.id.toLowerCase() || manifest.version !== ext.version) throw new Error('That package isn’t the theme you picked.')
  if (!licenseMatches(manifest, ext.license)) throw new Error('That package’s license isn’t the one Open VSX lists.')
  const contributions = themeContributions(manifest)
  if (!contributions.length) throw new Error('That extension has no color themes.')
  if (contributions.length > MAX_THEMES) throw new Error('That extension has too many color themes.')

  const parsed: { label: string; mode: 'light' | 'dark'; theme: Omit<ThemeDef, 'id'> }[] = []
  for (const c of contributions) {
    if (typeof c.path !== 'string') continue
    const mode = c.uiTheme === 'vs' || c.uiTheme === 'hc-light' ? 'light' : 'dark'
    const label = (typeof c.label === 'string' && c.label.trim()) || ext.name
    const colors = await themeColors(zip, packagePath(c.path))
    try {
      parsed.push({ label, mode, theme: parseThemeFile({ name: label, type: mode, colors }, label) })
    } catch {
      // A theme with no workbench colors (only syntax colors) has nothing for Writan.
    }
  }
  if (!parsed.length) throw new Error('That extension’s themes don’t color the window, only code, so Writan can’t use them.')

  // A light theme pairs with the dark one of the same name: “Tokyo Night Light” with “Tokyo Night”,
  // or the only two an extension has.
  const lights = parsed.filter((p) => p.mode === 'light')
  const darks = parsed.filter((p) => p.mode === 'dark')
  const base = (label: string) => label.toLowerCase().replace(/[\s-]*\b(light|day|dawn|latte)\b/g, '').replace(/[\s-]*\b(dark|night|moon|mocha)\b$/g, '').trim()
  const pairs = new Map<(typeof parsed)[number], (typeof parsed)[number]>()
  if (lights.length === 1 && darks.length === 1) pairs.set(darks[0], lights[0])
  else
    for (const l of lights) {
      const d = darks.find((x) => !pairs.has(x) && (x.label.toLowerCase() === base(l.label) || base(x.label) === base(l.label)))
      if (d) pairs.set(d, l)
    }
  const paired = new Set(pairs.values())
  const out: ThemeDef[] = []
  for (const p of parsed) {
    if (paired.has(p)) continue
    const l = pairs.get(p)
    if (l) {
      const d = p.theme
      out.push({
        id: await hashId(`${ext.id}:${p.label}:${l.label}`),
        name: (darks.length === 1 ? ext.name : p.label).slice(0, 48),
        accent: d.accent,
        light: l.theme.light,
        dark: d.dark,
        variants: { light: { accent: l.theme.accent, ...l.theme.variants?.light }, dark: { ...d.variants?.dark } },
        custom: true,
      })
      continue
    }
    out.push({
      ...p.theme,
      // Its other appearance stays in its own hue rather than an unrelated default.
      ...(p.mode === 'dark' ? { light: otherTone(p.theme.dark, 'dark') } : { dark: otherTone(p.theme.light, 'light') }),
      id: await hashId(`${ext.id}:${p.label}`),
      name: (parsed.length > 1 && p.label !== ext.name ? p.label : ext.name).slice(0, 48),
      custom: true,
    })
  }
  return out
}
