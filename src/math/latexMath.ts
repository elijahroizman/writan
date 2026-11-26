import { mathjax } from 'mathjax-full/js/mathjax.js'
import { TeX } from 'mathjax-full/js/input/tex.js'
import { SVG } from 'mathjax-full/js/output/svg.js'
import { liteAdaptor } from 'mathjax-full/js/adaptors/liteAdaptor.js'
import { RegisterHTMLHandler } from 'mathjax-full/js/handlers/html.js'
import { AllPackages } from 'mathjax-full/js/input/tex/AllPackages.js'
import { encodeLatexTag, LATEX_MATH_PREFIX, normalizeHexColor, parseLatexTag } from './latexTag'
import type { LatexMathSpec, RenderedLatexSvg } from './latexTag'

export { encodeLatexTag, LATEX_MATH_PREFIX, normalizeHexColor, parseLatexTag, type LatexMathSpec, type RenderedLatexSvg }

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
    let message = 'Invalid LaTeX syntax.'
    if (err instanceof Error && err.message) {
      message = err.message
    } else if (err && typeof err === 'object' && 'message' in err && typeof (err as { message: unknown }).message === 'string' && (err as { message: string }).message.trim()) {
      message = (err as { message: string }).message
    } else if (typeof err === 'string' && err.trim()) {
      message = err
    }
    return { svg: '', width: 0, height: 0, error: message }
  }
}

/**
 * Synchronously renders a `writan:latex:...` metadata tag to UTF-8 SVG bytes for display in Writan's
 * Word and Slides `MediaCache`. Saved `.docx`/`.pptx` files store real Office Math (OMML), not images.
 */
export function renderLatexTagToSvgBytes(tag: string): { bytes: Uint8Array; type: string } | null {
  const spec = parseLatexTag(tag)
  if (!spec) return null
  const rendered = renderLatexToSvg(spec)
  if (rendered.error || !rendered.svg) return null
  return { bytes: new TextEncoder().encode(rendered.svg), type: 'image/svg+xml' }
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
