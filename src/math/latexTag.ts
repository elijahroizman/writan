/** Prefix stored in OOXML image/frame `descr` so Writan recognizes editable LaTeX equations. */
export const LATEX_MATH_PREFIX = 'writan:latex:'

export interface LatexMathSpec {
  latex: string
  display: boolean
  fontSizePt: number
  color: string
}

export interface RenderedLatexSvg {
  svg: string
  width: number
  height: number
  error?: string
}

interface StoredLatexPayload {
  v: 1
  latex: string
  display?: boolean
  fontSizePt?: number
  color?: string
  w?: number
  h?: number
  f?: string
}

export function normalizeHexColor(color: string | null | undefined, fallback = '#000000'): string {
  if (!color) return fallback
  const hex = color.trim().replace(/^#/, '')
  if (/^[0-9a-fA-F]{6}$/.test(hex)) return `#${hex.toUpperCase()}`
  if (/^[0-9a-fA-F]{3}$/.test(hex)) {
    return `#${hex[0]}${hex[0]}${hex[1]}${hex[1]}${hex[2]}${hex[2]}`.toUpperCase()
  }
  return fallback
}

/** Encodes a LaTeX equation specification (and optional rendered CSS-px size) into an OOXML metadata tag. */
export function encodeLatexTag(spec: LatexMathSpec, dims?: { width: number; height: number; fingerprint?: string }): string {
  const payload: StoredLatexPayload = {
    v: 1,
    latex: spec.latex,
    display: spec.display,
    fontSizePt: Math.round(spec.fontSizePt * 10) / 10,
    color: normalizeHexColor(spec.color),
  }
  if (dims && Number.isFinite(dims.width) && dims.width > 0 && Number.isFinite(dims.height) && dims.height > 0) {
    payload.w = Math.round(dims.width * 10) / 10
    payload.h = Math.round(dims.height * 10) / 10
  }
  if (dims?.fingerprint !== undefined) {
    payload.f = dims.fingerprint
  }
  return `${LATEX_MATH_PREFIX}${JSON.stringify(payload)}`
}

/** Decodes a `writan:latex:...` tag back into a LatexMathSpec, or returns null if not a Writan math tag. */
export function parseLatexTag(tag: string | null | undefined): LatexMathSpec | null {
  if (!tag || !tag.startsWith(LATEX_MATH_PREFIX)) return null
  const raw = tag.slice(LATEX_MATH_PREFIX.length).trim()
  if (!raw) return null
  try {
    const parsed = JSON.parse(raw) as Partial<StoredLatexPayload>
    if (typeof parsed.latex !== 'string') return null
    return {
      latex: parsed.latex,
      display: Boolean(parsed.display),
      fontSizePt: typeof parsed.fontSizePt === 'number' && Number.isFinite(parsed.fontSizePt) && parsed.fontSizePt > 0 ? parsed.fontSizePt : 12,
      color: normalizeHexColor(typeof parsed.color === 'string' ? parsed.color : '#000000'),
    }
  } catch {
    return null
  }
}
