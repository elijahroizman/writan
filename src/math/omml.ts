import { mathjax } from 'mathjax-full/js/mathjax.js'
import { TeX } from 'mathjax-full/js/input/tex.js'
import { AllPackages } from 'mathjax-full/js/input/tex/AllPackages.js'
import { liteAdaptor } from 'mathjax-full/js/adaptors/liteAdaptor.js'
import { RegisterHTMLHandler } from 'mathjax-full/js/handlers/html.js'
import { STATE } from 'mathjax-full/js/core/MathItem.js'
import { normalizeHexColor, type LatexMathSpec } from './latexMath'

export const NS_M = 'http://schemas.openxmlformats.org/officeDocument/2006/math'
export const NS_W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'
export const NS_A = 'http://schemas.openxmlformats.org/drawingml/2006/main'

export type OmmlTarget = 'word' | 'slides'

interface MmlAttrStore {
  get?: (name: string) => unknown
  getExplicit?: (name: string) => unknown
}

interface MmlNodeLike {
  kind: string
  isToken?: boolean
  childNodes?: MmlNodeLike[]
  attributes?: MmlAttrStore
  getText?: () => string
}

interface ConvertCtx {
  target: OmmlTarget
  fontSizePt: number
  colorHex: string // 6 uppercase hex digits, no '#'
  display: boolean
}

const NARY_OPS = new Set([
  '\u2211', // ∑ sum
  '\u220F', // ∏ prod
  '\u2210', // ∐ coprod
  '\u222B', // ∫ int
  '\u222C', // ∬ iint
  '\u222D', // ∭ iiint
  '\u222E', // ∮ oint
  '\u222F', // ∯ oiint
  '\u2230', // ∰ oiiint
  '\u22C0', // ⋀ bigwedge
  '\u22C1', // ⋁ bigvee
  '\u22C2', // ⋂ bigcap
  '\u22C3', // ⋃ bigcup
  '\u2A00', // ⨀ bigodot
  '\u2A01', // ⨁ bigoplus
  '\u2A02', // ⨂ bigotimes
  '\u2A04', // ⨄ biguplus
  '\u2A06', // ⨆ bigsqcup
])

const INTEGRAL_OPS = new Set(['\u222B', '\u222C', '\u222D', '\u222E', '\u222F', '\u2230'])

const ACCENT_MAP: Record<string, string> = {
  '^': '\u0302',
  '\u02C6': '\u0302',
  '\u0302': '\u0302',
  '~': '\u0303',
  '\u02DC': '\u0303',
  '\u0303': '\u0303',
  '\u00B4': '\u0301',
  '\u0301': '\u0301',
  '`': '\u0300',
  '\u0300': '\u0300',
  '\u02D9': '\u0307',
  '\u0307': '\u0307',
  '\u00A8': '\u0308',
  '\u0308': '\u0308',
  '\u02C7': '\u030C',
  '\u030C': '\u030C',
  '\u02D8': '\u0306',
  '\u0306': '\u0306',
  '\u2192': '\u20D7',
  '\u20D7': '\u20D7',
  '\u2190': '\u20D6',
  '\u20D6': '\u20D6',
}

const BAR_CHARS = new Set(['\u00AF', '\u0304', '\u0305', '\u203E', '_', '\u0332', '\u2015', '-'])

const OPEN_DELIMS = new Set(['(', '[', '{', '|', '\u2016', '\u2308', '\u230A', '\u27E8', '\u2329'])
const CLOSE_DELIMS = new Set([')', ']', '}', '|', '\u2016', '\u2309', '\u230B', '\u27E9', '\u232A'])

let mmlDoc: { convert: (math: string, opts: { display: boolean; end: number }) => MmlNodeLike } | null = null

function ensureMmlDoc() {
  if (mmlDoc) return mmlDoc
  const adaptor = liteAdaptor()
  RegisterHTMLHandler(adaptor)
  const packages = AllPackages.filter((pkg: string) => pkg !== 'bussproofs')
  const tex = new TeX({
    packages,
    formatError: (_jax: unknown, err: Error) => {
      throw err
    },
  })
  mmlDoc = mathjax.document('', { InputJax: tex }) as unknown as {
    convert: (math: string, opts: { display: boolean; end: number }) => MmlNodeLike
  }
  return mmlDoc
}

function escapeXml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;')
}

function attrStr(node: MmlNodeLike, name: string): string | undefined {
  const val = node.attributes?.getExplicit?.(name) ?? node.attributes?.get?.(name)
  return typeof val === 'string' && val.length > 0 ? val : undefined
}

function nodeText(node: MmlNodeLike | undefined): string {
  if (!node) return ''
  if (node.kind === 'text' && typeof node.getText === 'function') {
    return node.getText()
  }
  return (node.childNodes ?? []).map(nodeText).join('')
}

function unwrapSingle(node: MmlNodeLike): MmlNodeLike {
  let cur = node
  while (
    (cur.kind === 'mrow' || cur.kind === 'inferredMrow' || cur.kind === 'TeXAtom') &&
    cur.childNodes?.length === 1
  ) {
    cur = cur.childNodes[0]
  }
  return cur
}

function formatRunProps(ctx: ConvertCtx): string {
  const halfPt = Math.max(12, Math.min(288, Math.round(ctx.fontSizePt * 2)))
  const hundredthsPt = Math.max(600, Math.min(14400, Math.round(ctx.fontSizePt * 100)))
  if (ctx.target === 'word') {
    const colorXml = ctx.colorHex !== '000000' ? `<w:color w:val="${ctx.colorHex}"/>` : ''
    return `<w:rPr><w:rFonts w:ascii="Cambria Math" w:hAnsi="Cambria Math"/>${colorXml}<w:sz w:val="${halfPt}"/><w:szCs w:val="${halfPt}"/></w:rPr>`
  }
  const fillXml = `<a:solidFill><a:srgbClr val="${ctx.colorHex}"/></a:solidFill>`
  return `<a:rPr lang="en-US" sz="${hundredthsPt}">${fillXml}<a:latin typeface="Cambria Math"/></a:rPr>`
}

function emitMathRun(
  text: string,
  ctx: ConvertCtx,
  opts?: { normal?: boolean; style?: 'p' | 'b' | 'i' | 'bi' },
): string {
  if (!text) return ''
  let mRPr = ''
  if (opts?.normal) {
    mRPr += '<m:nor/>'
  }
  if (opts?.style) {
    mRPr += `<m:sty m:val="${opts.style}"/>`
  }
  const mRPrXml = mRPr ? `<m:rPr>${mRPr}</m:rPr>` : ''
  const hostRPr = formatRunProps(ctx)
  const spaceAttr = /^\s|\s$|\s{2}/.test(text) ? ' xml:space="preserve"' : ''
  return `<m:r>${mRPrXml}${hostRPr}<m:t${spaceAttr}>${escapeXml(text)}</m:t></m:r>`
}

const DOUBLE_STRUCK_EXCEPTIONS: Record<string, string> = {
  C: '\u2102',
  H: '\u210D',
  N: '\u2115',
  P: '\u2119',
  Q: '\u211A',
  R: '\u211D',
  Z: '\u2124',
}

const SCRIPT_EXCEPTIONS: Record<string, string> = {
  B: '\u212C',
  E: '\u2130',
  F: '\u2131',
  H: '\u210B',
  I: '\u2110',
  L: '\u2112',
  M: '\u2133',
  R: '\u211B',
  e: '\u212F',
  g: '\u210A',
  o: '\u2134',
}

const FRAKTUR_EXCEPTIONS: Record<string, string> = {
  C: '\u212D',
  H: '\u210C',
  I: '\u2111',
  R: '\u211C',
  Z: '\u2128',
}

/** Maps ASCII letters/digits to Unicode Mathematical Alphanumeric Symbols for OMML alphabets. */
function mapMathVariantChars(text: string, variant?: string): { text: string; mapped: boolean } {
  if (!variant) return { text, mapped: false }
  let upperBase = 0
  let lowerBase = 0
  let digitBase = 0
  let exceptions: Record<string, string> | null = null
  switch (variant) {
    case 'double-struck':
      upperBase = 0x1d538
      lowerBase = 0x1d552
      digitBase = 0x1d7d8
      exceptions = DOUBLE_STRUCK_EXCEPTIONS
      break
    case 'script':
      upperBase = 0x1d49c
      lowerBase = 0x1d4b6
      exceptions = SCRIPT_EXCEPTIONS
      break
    case 'bold-script':
      upperBase = 0x1d4d0
      lowerBase = 0x1d4ea
      break
    case 'fraktur':
      upperBase = 0x1d504
      lowerBase = 0x1d51e
      exceptions = FRAKTUR_EXCEPTIONS
      break
    case 'bold-fraktur':
      upperBase = 0x1d56c
      lowerBase = 0x1d586
      break
    case 'sans-serif':
      upperBase = 0x1d5a0
      lowerBase = 0x1d5ba
      digitBase = 0x1d7e2
      break
    case 'bold-sans-serif':
      upperBase = 0x1d5d4
      lowerBase = 0x1d5ee
      digitBase = 0x1d7ec
      break
    case 'sans-serif-italic':
      upperBase = 0x1d608
      lowerBase = 0x1d622
      break
    case 'sans-serif-bold-italic':
      upperBase = 0x1d63c
      lowerBase = 0x1d656
      break
    case 'monospace':
      upperBase = 0x1d670
      lowerBase = 0x1d68a
      digitBase = 0x1d7f6
      break
    default:
      return { text, mapped: false }
  }
  let out = ''
  let anyMapped = false
  for (const ch of text) {
    if (exceptions && exceptions[ch]) {
      out += exceptions[ch]
      anyMapped = true
      continue
    }
    const code = ch.charCodeAt(0)
    if (upperBase && code >= 0x41 && code <= 0x5a) {
      out += String.fromCodePoint(upperBase + (code - 0x41))
      anyMapped = true
    } else if (lowerBase && code >= 0x61 && code <= 0x7a) {
      out += String.fromCodePoint(lowerBase + (code - 0x61))
      anyMapped = true
    } else if (digitBase && code >= 0x30 && code <= 0x39) {
      out += String.fromCodePoint(digitBase + (code - 0x30))
      anyMapped = true
    } else {
      out += ch
    }
  }
  return { text: out, mapped: anyMapped }
}

function styleFromMathVariant(variant: string | undefined, defaultKind: 'mi' | 'mn' | 'mo' | 'mtext'): { normal?: boolean; style?: 'p' | 'b' | 'i' | 'bi' } {
  if (defaultKind === 'mtext') {
    if (variant === 'bold') return { normal: true, style: 'b' }
    if (variant === 'italic') return { normal: true, style: 'i' }
    if (variant === 'bold-italic') return { normal: true, style: 'bi' }
    return { normal: true, style: 'p' }
  }
  if (!variant) {
    if (defaultKind === 'mn' || defaultKind === 'mo') return { style: 'p' }
    return {}
  }
  switch (variant) {
    case 'normal':
    case 'sans-serif':
    case 'monospace':
    case 'double-struck':
    case 'fraktur':
    case 'script':
      return { style: 'p' }
    case 'bold':
    case 'bold-sans-serif':
    case 'bold-fraktur':
    case 'bold-script':
      return { style: 'b' }
    case 'italic':
    case 'sans-serif-italic':
      return { style: 'i' }
    case 'bold-italic':
    case 'sans-serif-bold-italic':
      return { style: 'bi' }
    default:
      return {}
  }
}

function getNaryOp(node: MmlNodeLike): string | null {
  const inner = unwrapSingle(node)
  if (inner.kind === 'mo') {
    const ch = nodeText(inner).trim()
    if (NARY_OPS.has(ch)) return ch
  }
  return null
}

function convertChildren(nodes: MmlNodeLike[], ctx: ConvertCtx): string {
  let out = ''
  for (let i = 0; i < nodes.length; i++) {
    const node = nodes[i]
    // Check if this is an n-ary operator (with or without limits) that should consume the following term as its <m:e>.
    const nary = tryExtractNary(node)
    if (nary) {
      // Collect remaining siblings up to a top-level binary relation/additive operator, or at least the next sibling.
      const bodyNodes: MmlNodeLike[] = []
      while (i + 1 < nodes.length) {
        const next = nodes[i + 1]
        const nextInner = unwrapSingle(next)
        if (
          bodyNodes.length > 0 &&
          nextInner.kind === 'mo' &&
          ['=', '<', '>', '\u2264', '\u2265', '\u2260', '\u2248', '+', '-', '\u2212', '\u00B1', ','].includes(nodeText(nextInner).trim())
        ) {
          break
        }
        bodyNodes.push(next)
        i++
      }
      const subXml = nary.sub ? convertNode(nary.sub, ctx) : ''
      const supXml = nary.sup ? convertNode(nary.sup, ctx) : ''
      const limLoc = nary.forceSubSup || INTEGRAL_OPS.has(nary.chr) ? 'subSup' : 'undOvr'
      const pr = [
        `<m:chr m:val="${escapeXml(nary.chr)}"/>`,
        `<m:limLoc m:val="${limLoc}"/>`,
        nary.sub ? '' : '<m:subHide m:val="1"/>',
        nary.sup ? '' : '<m:supHide m:val="1"/>',
      ].join('')
      const bodyXml = convertChildren(bodyNodes, ctx)
      out += `<m:nary><m:naryPr>${pr}</m:naryPr><m:sub>${subXml}</m:sub><m:sup>${supXml}</m:sup><m:e>${bodyXml}</m:e></m:nary>`
      continue
    }
    out += convertNode(node, ctx)
  }
  return out
}

function tryExtractNary(node: MmlNodeLike): { chr: string; sub?: MmlNodeLike; sup?: MmlNodeLike; forceSubSup: boolean } | null {
  const inner = unwrapSingle(node)
  if (inner.kind === 'munderover' || inner.kind === 'msubsup') {
    const [base, sub, sup] = inner.childNodes ?? []
    const chr = base ? getNaryOp(base) : null
    if (chr) return { chr, sub, sup, forceSubSup: inner.kind === 'msubsup' }
  }
  if (inner.kind === 'munder' || inner.kind === 'msub') {
    const [base, sub] = inner.childNodes ?? []
    const chr = base ? getNaryOp(base) : null
    if (chr) return { chr, sub, forceSubSup: inner.kind === 'msub' }
  }
  if (inner.kind === 'mover' || inner.kind === 'msup') {
    const [base, sup] = inner.childNodes ?? []
    const chr = base ? getNaryOp(base) : null
    if (chr) return { chr, sup, forceSubSup: inner.kind === 'msup' }
  }
  const direct = getNaryOp(inner)
  if (direct) return { chr: direct, forceSubSup: INTEGRAL_OPS.has(direct) }
  return null
}

function convertNode(node: MmlNodeLike, ctx: ConvertCtx): string {
  const children = node.childNodes ?? []
  const explicitColor = attrStr(node, 'mathcolor')
  const activeCtx = explicitColor
    ? { ...ctx, colorHex: normalizeHexColor(explicitColor, `#${ctx.colorHex}`).slice(1) }
    : ctx

  switch (node.kind) {
    case 'math':
    case 'inferredMrow':
    case 'TeXAtom':
    case 'mstyle':
    case 'mpadded':
    case 'mphantom':
    case 'semantics':
      return convertChildren(children, activeCtx)

    case 'mrow': {
      // Detect \left( ... \right) delimiter groups: first child OPEN mo, last child CLOSE mo.
      if (children.length >= 2) {
        const first = unwrapSingle(children[0])
        const last = unwrapSingle(children[children.length - 1])
        const firstTxt = first.kind === 'mo' ? nodeText(first).trim() : ''
        const lastTxt = last.kind === 'mo' ? nodeText(last).trim() : ''
        const isDelimited =
          (attrStr(first, 'data-mjx-texclass') === 'OPEN' || (OPEN_DELIMS.has(firstTxt) && attrStr(first, 'stretchy') === 'true')) &&
          (attrStr(last, 'data-mjx-texclass') === 'CLOSE' || (CLOSE_DELIMS.has(lastTxt) && attrStr(last, 'stretchy') === 'true'))
        if (isDelimited) {
          const mid = children.slice(1, -1)
          const beg = firstTxt ? `<m:begChr m:val="${escapeXml(firstTxt)}"/>` : '<m:begChr m:val=""/>'
          const end = lastTxt ? `<m:endChr m:val="${escapeXml(lastTxt)}"/>` : '<m:endChr m:val=""/>'
          return `<m:d><m:dPr>${beg}${end}</m:dPr><m:e>${convertChildren(mid, activeCtx)}</m:e></m:d>`
        }
      }
      return convertChildren(children, activeCtx)
    }

    case 'mi': {
      const rawText = nodeText(node)
      const variant = attrStr(node, 'mathvariant')
      const { text, mapped } = mapMathVariantChars(rawText, variant)
      // Multi-character identifiers like "sin", "cos", "lim" should be upright.
      const style = mapped
        ? { style: 'p' as const }
        : variant
          ? styleFromMathVariant(variant, 'mi')
          : text.length > 1
            ? { style: 'p' as const }
            : {}
      return emitMathRun(text, activeCtx, style)
    }

    case 'mn': {
      const rawText = nodeText(node)
      const variant = attrStr(node, 'mathvariant')
      const { text, mapped } = mapMathVariantChars(rawText, variant)
      return emitMathRun(text, activeCtx, mapped ? { style: 'p' } : styleFromMathVariant(variant, 'mn'))
    }

    case 'mo': {
      const text = nodeText(node)
      const variant = attrStr(node, 'mathvariant')
      return emitMathRun(text, activeCtx, styleFromMathVariant(variant, 'mo'))
    }

    case 'mtext':
    case 'ms': {
      const text = nodeText(node)
      const variant = attrStr(node, 'mathvariant')
      return emitMathRun(text, activeCtx, styleFromMathVariant(variant, 'mtext'))
    }

    case 'mspace':
      return emitMathRun(' ', activeCtx, { style: 'p' })

    case 'mfrac': {
      const [num, den] = children
      const thickness = attrStr(node, 'linethickness')
      const fPr = thickness === '0' || thickness === '0pt' ? '<m:fPr><m:type m:val="noBar"/></m:fPr>' : ''
      return `<m:f>${fPr}<m:num>${num ? convertNode(num, activeCtx) : ''}</m:num><m:den>${den ? convertNode(den, activeCtx) : ''}</m:den></m:f>`
    }

    case 'msqrt':
      return `<m:rad><m:radPr><m:degHide m:val="1"/></m:radPr><m:deg/><m:e>${convertChildren(children, activeCtx)}</m:e></m:rad>`

    case 'mroot': {
      const [base, deg] = children
      return `<m:rad><m:deg>${deg ? convertNode(deg, activeCtx) : ''}</m:deg><m:e>${base ? convertNode(base, activeCtx) : ''}</m:e></m:rad>`
    }

    case 'msup': {
      const [base, sup] = children
      return `<m:sSup><m:e>${base ? convertNode(base, activeCtx) : ''}</m:e><m:sup>${sup ? convertNode(sup, activeCtx) : ''}</m:sup></m:sSup>`
    }

    case 'msub': {
      const [base, sub] = children
      return `<m:sSub><m:e>${base ? convertNode(base, activeCtx) : ''}</m:e><m:sub>${sub ? convertNode(sub, activeCtx) : ''}</m:sub></m:sSub>`
    }

    case 'msubsup': {
      const [base, sub, sup] = children
      return `<m:sSubSup><m:e>${base ? convertNode(base, activeCtx) : ''}</m:e><m:sub>${sub ? convertNode(sub, activeCtx) : ''}</m:sub><m:sup>${sup ? convertNode(sup, activeCtx) : ''}</m:sup></m:sSubSup>`
    }

    case 'munder': {
      const [base, under] = children
      const underText = under ? nodeText(unwrapSingle(under)).trim() : ''
      if (BAR_CHARS.has(underText)) {
        return `<m:bar><m:barPr><m:pos m:val="bot"/></m:barPr><m:e>${base ? convertNode(base, activeCtx) : ''}</m:e></m:bar>`
      }
      return `<m:limLow><m:e>${base ? convertNode(base, activeCtx) : ''}</m:e><m:lim>${under ? convertNode(under, activeCtx) : ''}</m:lim></m:limLow>`
    }

    case 'mover': {
      const [base, over] = children
      const overText = over ? nodeText(unwrapSingle(over)).trim() : ''
      if (BAR_CHARS.has(overText)) {
        return `<m:bar><m:barPr><m:pos m:val="top"/></m:barPr><m:e>${base ? convertNode(base, activeCtx) : ''}</m:e></m:bar>`
      }
      const accent = ACCENT_MAP[overText]
      if (accent) {
        return `<m:acc><m:accPr><m:chr m:val="${escapeXml(accent)}"/></m:accPr><m:e>${base ? convertNode(base, activeCtx) : ''}</m:e></m:acc>`
      }
      return `<m:limUpp><m:e>${base ? convertNode(base, activeCtx) : ''}</m:e><m:lim>${over ? convertNode(over, activeCtx) : ''}</m:lim></m:limUpp>`
    }

    case 'munderover': {
      const [base, under, over] = children
      const baseXml = base ? convertNode(base, activeCtx) : ''
      const low = `<m:limLow><m:e>${baseXml}</m:e><m:lim>${under ? convertNode(under, activeCtx) : ''}</m:lim></m:limLow>`
      return `<m:limUpp><m:e>${low}</m:e><m:lim>${over ? convertNode(over, activeCtx) : ''}</m:lim></m:limUpp>`
    }

    case 'mfenced': {
      const open = attrStr(node, 'open') ?? '('
      const close = attrStr(node, 'close') ?? ')'
      const sep = attrStr(node, 'separators')
      const pr = [
        `<m:begChr m:val="${escapeXml(open)}"/>`,
        sep !== undefined ? `<m:sepChr m:val="${escapeXml(sep.charAt(0))}"/>` : '',
        `<m:endChr m:val="${escapeXml(close)}"/>`,
      ].join('')
      const items = children.map((c) => `<m:e>${convertNode(c, activeCtx)}</m:e>`).join('')
      return `<m:d><m:dPr>${pr}</m:dPr>${items || '<m:e/>'}</m:d>`
    }

    case 'mtable': {
      const rows = children
        .filter((c) => c.kind === 'mtr' || c.kind === 'mlabeledtr')
        .map((row) => {
          const cells = (row.childNodes ?? [])
            .filter((c) => c.kind === 'mtd')
            .map((cell) => `<m:e>${convertChildren(cell.childNodes ?? [], activeCtx)}</m:e>`)
            .join('')
          return `<m:mr>${cells}</m:mr>`
        })
        .join('')
      return `<m:m>${rows}</m:m>`
    }

    case 'annotation':
    case 'annotation-xml':
      return ''

    default:
      if (children.length > 0) return convertChildren(children, activeCtx)
      const fallback = nodeText(node)
      return fallback ? emitMathRun(fallback, activeCtx) : ''
  }
}

/**
 * Converts a LaTeX math expression to native Office Math Markup Language (OMML):
 * - For `word`: `<m:oMath>` (inline) or `<m:oMathPara><m:oMath>...</m:oMath></m:oMathPara>` (display)
 * - For `slides`: `<m:oMathPara><m:oMath>...</m:oMath></m:oMathPara>` with DrawingML `<a:rPr>` run formatting
 */
export function latexToOmml(spec: LatexMathSpec, target: OmmlTarget): { omml: string; linear: string } {
  const source = spec.latex.trim()
  if (!source) {
    throw new Error('Enter a LaTeX expression.')
  }
  const fontSizePt = Math.max(6, Math.min(144, Number.isFinite(spec.fontSizePt) ? spec.fontSizePt : 12))
  const colorHex = normalizeHexColor(spec.color).slice(1)
  const doc = ensureMmlDoc()
  const root = doc.convert(source, { display: spec.display, end: STATE.CONVERT })
  const ctx: ConvertCtx = { target, fontSizePt, colorHex, display: spec.display }
  const body = convertNode(root, ctx)
  const hostNs = target === 'word' ? ` xmlns:w="${NS_W}"` : ` xmlns:a="${NS_A}"`
  const oMath = `<m:oMath xmlns:m="${NS_M}"${hostNs}>${body}</m:oMath>`
  const omml =
    target === 'slides' || spec.display
      ? `<m:oMathPara xmlns:m="${NS_M}"${hostNs}>${oMath}</m:oMathPara>`
      : oMath
  const linear = nodeText(root).replace(/\s+/g, ' ').trim() || source
  return { omml, linear }
}
