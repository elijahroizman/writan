import { mathjax } from 'mathjax-full/js/mathjax.js'
import { TeX } from 'mathjax-full/js/input/tex.js'
import { SVG } from 'mathjax-full/js/output/svg.js'
import { liteAdaptor } from 'mathjax-full/js/adaptors/liteAdaptor.js'
import { RegisterHTMLHandler } from 'mathjax-full/js/handlers/html.js'
import { AllPackages } from 'mathjax-full/js/input/tex/AllPackages.js'

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
}

/** Encodes a LaTeX equation specification into an OOXML alt-text tag (`wp:docPr/@descr` or `p:cNvPr/@descr`). */
export function encodeLatexTag(spec: LatexMathSpec): string {
  const payload: StoredLatexPayload = {
    v: 1,
    latex: spec.latex,
    display: spec.display,
    fontSizePt: Math.round(spec.fontSizePt * 10) / 10,
    color: normalizeHexColor(spec.color),
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

export function normalizeHexColor(color: string | null | undefined, fallback = '#000000'): string {
  if (!color) return fallback
  const hex = color.trim().replace(/^#/, '')
  if (/^[0-9a-fA-F]{6}$/.test(hex)) return `#${hex.toUpperCase()}`
  if (/^[0-9a-fA-F]{3}$/.test(hex)) {
    return `#${hex[0]}${hex[0]}${hex[1]}${hex[1]}${hex[2]}${hex[2]}`.toUpperCase()
  }
  return fallback
}

// Lazily initialized singleton MathJax TeX -> SVG document (pure vector paths, zero external fonts).
let mjxDoc: { convert: (math: string, opts: { display: boolean }) => unknown } | null = null
let mjxAdaptor: { innerHTML: (node: unknown) => string } | null = null

function ensureMathJax() {
  if (mjxDoc && mjxAdaptor) return { doc: mjxDoc, adaptor: mjxAdaptor }
  const adaptor = liteAdaptor()
  RegisterHTMLHandler(adaptor)
  const packages = AllPackages.filter((pkg: string) => pkg !== 'bussproofs')
  const tex = new TeX({
    packages,
    formatError: (_jax: unknown, err: Error) => {
      throw err
    },
  })
  const svg = new SVG({ fontCache: 'local' })
  mjxDoc = mathjax.document('', { InputJax: tex, OutputJax: svg }) as unknown as {
    convert: (math: string, opts: { display: boolean }) => unknown
  }
  mjxAdaptor = adaptor as unknown as { innerHTML: (node: unknown) => string }
  return { doc: mjxDoc, adaptor: mjxAdaptor }
}

/**
 * Renders LaTeX math synchronously to a self-contained SVG string with explicit pixel dimensions.
 * Returns `error` if the LaTeX cannot be parsed.
 */
export function renderLatexToSvg(spec: LatexMathSpec): RenderedLatexSvg {
  const source = spec.latex.trim()
  if (!source) {
    return { svg: '', width: 0, height: 0, error: 'Enter a LaTeX expression.' }
  }
  const fontSizePt = Math.max(6, Math.min(144, Number.isFinite(spec.fontSizePt) ? spec.fontSizePt : 12))
  const color = normalizeHexColor(spec.color)
  try {
    const { doc, adaptor } = ensureMathJax()
    const node = doc.convert(source, { display: spec.display })
    const rawHtml = adaptor.innerHTML(node)
    const start = rawHtml.indexOf('<svg')
    const end = rawHtml.lastIndexOf('</svg>')
    if (start === -1 || end === -1) {
      return { svg: '', width: 0, height: 0, error: 'Could not render equation.' }
    }
    let svg = rawHtml.slice(start, end + '</svg>'.length)

    // Extract viewBox="minX minY vbW vbH". MathJax TeX uses 1000 viewBox units = 1em.
    const vbMatch = svg.match(/viewBox="([^"]+)"/)
    const emPx = fontSizePt * (96 / 72)
    let width = Math.max(12, Math.round(emPx * 2))
    let height = Math.max(12, Math.round(emPx * 1.2))
    if (vbMatch) {
      const parts = vbMatch[1].trim().split(/\s+/).map(Number)
      if (parts.length === 4 && parts.every((n) => Number.isFinite(n))) {
        const [minX, minY, vbW, vbH] = parts
        const padVb = 80
        const paddedMinX = minX - padVb
        const paddedMinY = minY - padVb
        const paddedW = Math.max(1, vbW + padVb * 2)
        const paddedH = Math.max(1, vbH + padVb * 2)
        width = Math.max(8, Math.ceil((paddedW / 1000) * emPx))
        height = Math.max(8, Math.ceil((paddedH / 1000) * emPx))
        svg = svg.replace(
          /viewBox="[^"]+"/,
          `viewBox="${paddedMinX.toFixed(1)} ${paddedMinY.toFixed(1)} ${paddedW.toFixed(1)} ${paddedH.toFixed(1)}"`,
        )
      }
    }

    // Replace ex-based width/height with exact CSS pixel dimensions and apply the chosen color.
    svg = svg
      .replace(/\swidth="[^"]+"/, ` width="${width}"`)
      .replace(/\sheight="[^"]+"/, ` height="${height}"`)
      .replace(/\sstyle="[^"]*"/, '')
      .replace('<svg', `<svg style="color:${color};display:block"`)

    // Replace `currentColor` explicitly for maximum compatibility with canvas/image SVG decoders.
    svg = svg.replace(/currentColor/g, color)

    return { svg, width, height }
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    return { svg: '', width: 0, height: 0, error: message || 'Invalid LaTeX syntax.' }
  }
}

/**
 * Renders LaTeX math to a crisp high-DPI transparent PNG for embedding in Word (.docx) and Slides (.pptx).
 * Embeds cleanly in Writan, Microsoft Office, Apple Keynote, and LibreOffice.
 */
export async function renderLatexToPng(
  spec: LatexMathSpec,
  scale = 3,
): Promise<{ bytes: Uint8Array; width: number; height: number }> {
  const rendered = renderLatexToSvg(spec)
  if (rendered.error || !rendered.svg) {
    throw new Error(rendered.error ?? 'Failed to render LaTeX equation.')
  }
  const { svg, width, height } = rendered
  const blob = new Blob([svg], { type: 'image/svg+xml;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  try {
    const img = await new Promise<HTMLImageElement>((resolve, reject) => {
      const el = new Image()
      el.onload = () => resolve(el)
      el.onerror = () => reject(new Error('Failed to rasterize equation image.'))
      el.src = url
    })
    const canvas = document.createElement('canvas')
    canvas.width = Math.max(1, Math.round(width * scale))
    canvas.height = Math.max(1, Math.round(height * scale))
    const ctx = canvas.getContext('2d')
    if (!ctx) throw new Error('Canvas 2D context unavailable.')
    ctx.clearRect(0, 0, canvas.width, canvas.height)
    ctx.scale(scale, scale)
    ctx.drawImage(img, 0, 0, width, height)
    const pngBlob = await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('Failed to encode equation PNG.'))), 'image/png')
    })
    return { bytes: new Uint8Array(await pngBlob.arrayBuffer()), width, height }
  } finally {
    URL.revokeObjectURL(url)
  }
}

export interface LatexSnippet {
  label: string
  insert: string
  title: string
}

export const LATEX_SNIPPETS: LatexSnippet[] = [
  { label: 'a/b', insert: '\\frac{a}{b}', title: 'Fraction' },
  { label: '√x', insert: '\\sqrt{x}', title: 'Square root' },
  { label: 'xⁿ', insert: 'x^{n}', title: 'Superscript / power' },
  { label: 'xᵢ', insert: 'x_{i}', title: 'Subscript' },
  { label: '∑', insert: '\\sum_{i=1}^{n} x_i', title: 'Summation' },
  { label: '∫', insert: '\\int_{a}^{b} f(x)\\,dx', title: 'Integral' },
  { label: 'lim', insert: '\\lim_{x \\to \\infty}', title: 'Limit' },
  { label: '[ ]', insert: '\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}', title: '2×2 Matrix' },
  { label: '±', insert: '\\pm', title: 'Plus-minus' },
  { label: 'α β', insert: '\\alpha + \\beta', title: 'Greek letters' },
]
