import caladeaBoldItalicUrl from '../../engine/fonts/Caladea-BoldItalic.ttf?url'
import caladeaBoldUrl from '../../engine/fonts/Caladea-Bold.ttf?url'
import caladeaItalicUrl from '../../engine/fonts/Caladea-Italic.ttf?url'
import caladeaRegularUrl from '../../engine/fonts/Caladea-Regular.ttf?url'
import carlitoBoldItalicUrl from '../../engine/fonts/Carlito-BoldItalic.ttf?url'
import carlitoBoldUrl from '../../engine/fonts/Carlito-Bold.ttf?url'
import carlitoItalicUrl from '../../engine/fonts/Carlito-Italic.ttf?url'
import carlitoRegularUrl from '../../engine/fonts/Carlito-Regular.ttf?url'
import liberationMonoBoldItalicUrl from '../../engine/fonts/LiberationMono-BoldItalic.ttf?url'
import liberationMonoBoldUrl from '../../engine/fonts/LiberationMono-Bold.ttf?url'
import liberationMonoItalicUrl from '../../engine/fonts/LiberationMono-Italic.ttf?url'
import liberationMonoRegularUrl from '../../engine/fonts/LiberationMono-Regular.ttf?url'
import liberationSansBoldItalicUrl from '../../engine/fonts/LiberationSans-BoldItalic.ttf?url'
import liberationSansBoldUrl from '../../engine/fonts/LiberationSans-Bold.ttf?url'
import liberationSansItalicUrl from '../../engine/fonts/LiberationSans-Italic.ttf?url'
import liberationSansRegularUrl from '../../engine/fonts/LiberationSans-Regular.ttf?url'
import liberationSerifBoldItalicUrl from '../../engine/fonts/LiberationSerif-BoldItalic.ttf?url'
import liberationSerifBoldUrl from '../../engine/fonts/LiberationSerif-Bold.ttf?url'
import liberationSerifItalicUrl from '../../engine/fonts/LiberationSerif-Italic.ttf?url'
import liberationSerifRegularUrl from '../../engine/fonts/LiberationSerif-Regular.ttf?url'
import notoArabicBoldUrl from '../../engine/fonts/pdf/NotoSansArabic-Bold.ttf?url'
import notoArabicRegularUrl from '../../engine/fonts/pdf/NotoSansArabic-Regular.ttf?url'
import notoMathRegularUrl from '../../engine/fonts/pdf/NotoSansMath-Regular.ttf?url'
import notoSansRegularUrl from '../../engine/fonts/pdf/NotoSans-Regular.ttf?url'
import { packBlobs } from './blobs'

const FONT_URLS: Record<string, string> = {
  'carlito:regular': carlitoRegularUrl,
  'carlito:bold': carlitoBoldUrl,
  'carlito:italic': carlitoItalicUrl,
  'carlito:bold-italic': carlitoBoldItalicUrl,
  'caladea:regular': caladeaRegularUrl,
  'caladea:bold': caladeaBoldUrl,
  'caladea:italic': caladeaItalicUrl,
  'caladea:bold-italic': caladeaBoldItalicUrl,
  'liberation-sans:regular': liberationSansRegularUrl,
  'liberation-sans:bold': liberationSansBoldUrl,
  'liberation-sans:italic': liberationSansItalicUrl,
  'liberation-sans:bold-italic': liberationSansBoldItalicUrl,
  'liberation-serif:regular': liberationSerifRegularUrl,
  'liberation-serif:bold': liberationSerifBoldUrl,
  'liberation-serif:italic': liberationSerifItalicUrl,
  'liberation-serif:bold-italic': liberationSerifBoldItalicUrl,
  'liberation-mono:regular': liberationMonoRegularUrl,
  'liberation-mono:bold': liberationMonoBoldUrl,
  'liberation-mono:italic': liberationMonoItalicUrl,
  'liberation-mono:bold-italic': liberationMonoBoldItalicUrl,
  'noto-arabic:regular': notoArabicRegularUrl,
  'noto-arabic:bold': notoArabicBoldUrl,
  'noto-math:regular': notoMathRegularUrl,
  'noto-sans:regular': notoSansRegularUrl,
}

const fontCache = new Map<string, Promise<Uint8Array>>()

async function fetchFont(key: string): Promise<Uint8Array> {
  const cached = fontCache.get(key)
  if (cached) return cached
  const url = FONT_URLS[key]
  if (!url) throw new Error(`Missing bundled PDF font: ${key}`)
  const pending = fetch(url)
    .then((res) => {
      if (!res.ok) throw new Error(`Failed to fetch PDF font ${key}: ${res.status}`)
      return res.arrayBuffer()
    })
    .then((buf) => new Uint8Array(buf))
    .catch((err) => {
      fontCache.delete(key)
      throw err
    })
  fontCache.set(key, pending)
  return pending
}

function familyBucket(raw: string): string | null {
  const name = raw.trim().replace(/^['"]|['"]$/g, '').toLowerCase()
  switch (name) {
    case 'calibri':
    case 'aptos':
    case 'carlito':
      return 'carlito'
    case 'cambria':
    case 'caladea':
      return 'caladea'
    case 'arial':
    case 'helvetica':
    case 'segoe ui':
    case 'liberation sans':
    case 'sans-serif':
    case 'system-ui':
      return 'liberation-sans'
    case 'times new roman':
    case 'times':
    case 'georgia':
    case 'liberation serif':
    case 'serif':
      return 'liberation-serif'
    case 'courier new':
    case 'courier':
    case 'liberation mono':
    case 'monospace':
      return 'liberation-mono'
    case 'noto sans arabic':
      return 'noto-arabic'
    case 'cambria math':
    case 'noto sans math':
      return 'noto-math'
    case 'noto sans':
      return 'noto-sans'
    default:
      return null
  }
}

function parseStyleFonts(css: string): { buckets: string[]; bold: boolean; italic: boolean } {
  let bold = false
  let italic = false
  let tail = css.trimStart()
  while (tail.length) {
    const end = tail.search(/[\s/]/)
    const token = end === -1 ? tail : tail.slice(0, end)
    tail = end === -1 ? '' : tail.slice(end).trimStart()
    if (/^\d+(?:\.\d+)?(?:px|pt)$/.test(token)) {
      if (tail.startsWith('/')) {
        tail = tail.slice(1).trimStart()
        const ws = tail.search(/\s/)
        tail = ws === -1 ? '' : tail.slice(ws).trimStart()
      }
      break
    }
    if (token === 'bold' || token === 'bolder' || (/^\d+$/.test(token) && Number(token) >= 600)) bold = true
    if (token === 'italic' || token === 'oblique') italic = true
    if (!token) break
  }
  const buckets: string[] = []
  for (const part of tail.split(',')) {
    const b = familyBucket(part)
    if (b && !buckets.includes(b)) buckets.push(b)
  }
  return { buckets, bold, italic }
}

const ARABIC_RE = /[\u0600-\u06FF\u0750-\u077F\u08A0-\u08FF\uFB50-\uFDFF\uFE70-\uFEFF]/
const HEBREW_RE = /[\u0590-\u05FF]/
const MATH_RE = /[\u0370-\u03FF\u1D00-\u1DBF\u2070-\u23FF\u25A0-\u27FF\u2900-\u2BFF]/
const EXTENDED_NON_LATIN_RE = /[^\u0000-\u024F]/

function addVariant(needed: Set<string>, bucket: string, bold: boolean, italic: boolean) {
  const variant = bold && italic ? 'bold-italic' : bold ? 'bold' : italic ? 'italic' : 'regular'
  const key = `${bucket}:${variant}`
  if (FONT_URLS[key]) needed.add(key)
  else if (bold && FONT_URLS[`${bucket}:bold`]) needed.add(`${bucket}:bold`)
  else if (FONT_URLS[`${bucket}:regular`]) needed.add(`${bucket}:regular`)
}

export async function packFonts(keys: Iterable<string>): Promise<Uint8Array> {
  const unique = [...new Set(keys)].filter((k) => k in FONT_URLS).sort()
  const entries = await Promise.all(unique.map(async (k): Promise<[string, Uint8Array]> => [k, await fetchFont(k)]))
  return packBlobs(entries)
}

export async function loadFontsForDocument(styles: ReadonlyArray<{ font: string }>, texts: Iterable<string>, hasSvg = false): Promise<Uint8Array> {
  const needed = new Set<string>(['liberation-sans:regular'])
  let anyBold = false
  let anyItalic = false
  for (const s of styles) {
    const parsed = parseStyleFonts(s.font)
    anyBold ||= parsed.bold
    anyItalic ||= parsed.italic
    for (const bucket of parsed.buckets) addVariant(needed, bucket, parsed.bold, parsed.italic)
  }
  if (hasSvg) {
    addVariant(needed, 'liberation-sans', false, false)
    addVariant(needed, 'liberation-serif', false, false)
  }
  for (const text of texts) {
    if (ARABIC_RE.test(text)) {
      needed.add('noto-arabic:regular')
      if (anyBold) needed.add('noto-arabic:bold')
    }
    if (HEBREW_RE.test(text)) {
      addVariant(needed, 'liberation-sans', anyBold, anyItalic)
      needed.add('noto-sans:regular')
    }
    if (MATH_RE.test(text)) {
      needed.add('noto-math:regular')
      needed.add('noto-sans:regular')
    } else if (EXTENDED_NON_LATIN_RE.test(text)) {
      needed.add('noto-sans:regular')
    }
  }
  return packFonts(needed)
}

export async function loadFontsForOcrLayers(layers: ReadonlyArray<{ words: ReadonlyArray<{ text: string }> }>): Promise<Uint8Array> {
  const needed = new Set<string>(['liberation-sans:regular'])
  for (const layer of layers) {
    for (const word of layer.words) {
      if (ARABIC_RE.test(word.text)) needed.add('noto-arabic:regular')
      if (HEBREW_RE.test(word.text) || EXTENDED_NON_LATIN_RE.test(word.text)) needed.add('noto-sans:regular')
      if (MATH_RE.test(word.text)) needed.add('noto-math:regular')
    }
  }
  return packFonts(needed)
}
