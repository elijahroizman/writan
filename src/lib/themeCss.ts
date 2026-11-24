// A theme's own CSS. It comes from imported files and from agents, so it must not fetch: a url()
// on an attribute selector can send what's on screen to another site. The browser parses the
// text first, so the checks see normalized values and CSS escapes in a function name can't dodge
// them. The checks are T3 Code's (MIT, © 2026 T3 Tools Inc.), which has the same problem.

const FETCHES_BY_STRING = /image-set\(|\bsrc\(/i
// Only CSS whitespace may come first. \s also matches U+00A0, which makes the address relative.
const URL_NOT_LOCAL = /url\((?![ \t\n\f\r]*["']?(?:data:|#))/i
// Tokens left to right, as CSS reads them: a url( or /* inside a string loads nothing, and a quote
// in an unquoted url( or after a backslash isn't a string. A quoted url() that isn't data: or #id
// keeps `url(` for the check.
const ESCAPE = String.raw`\\(?:(?:[0-9a-fA-F]{6}|[0-9a-fA-F]{1,5}(?![0-9a-fA-F]))(?:\r\n|[\n\r\f])?|\r\n|[^0-9a-fA-F])`
const STRING = String.raw`"(?:${ESCAPE}|[^"\\\n\r\f])*"|'(?:${ESCAPE}|[^'\\\n\r\f])*'`
const URL_OR_STRING = String.raw`url\((?![ \t\n\f\r]*["'])([^)]*)|url\([ \t\n\f\r]*(${STRING})|${STRING}|${ESCAPE}`
const CSS_TOKENS = new RegExp(String.raw`\/\*[\s\S]*?\*\/|${URL_OR_STRING}`, 'gi')

/**
 * The code with its strings and comments removed, keeping `url(` where a quoted url() can load a
 * file. A declaration value keeps a comment or quoted url() that holds an escape. In a stylesheet
 * an unquoted url() is judged by its address too.
 */
function withoutStrings(code: string, stylesheet: boolean): string {
  return code.replace(CSS_TOKENS, (token: string, address?: string, urlString?: string) => {
    const url = stylesheet ? (address ?? urlString) : urlString
    if (url !== undefined) {
      if (!stylesheet && url.includes('\\')) return token
      return /^[ \t\n\f\r]*["']?(?:data:|#)/i.test(url) ? '' : 'url('
    }
    if (token.startsWith('/*')) {
      if (stylesheet) return ''
      return token.includes('\\') ? token : ' '
    }
    return /^["']/.test(token) ? '' : token
  })
}

const fetchesOutsideStrings = (code: string) => FETCHES_BY_STRING.test(code) || URL_NOT_LOCAL.test(code)

/**
 * Whether one declaration value can load a file. Values with var() can keep escapes that spell
 * `var` or `url`, so escapes that survive removing plain strings count as loading.
 */
export function cssValueFetches(value: string): boolean {
  const code = withoutStrings(value, false)
  return code.includes('\\') || fetchesOutsideStrings(code)
}

/** A text check, for telling an agent early. CSS escapes can dodge it; `applyThemeCss` is the guard. */
export function cssLoadsFiles(css: string): boolean {
  const code = withoutStrings(css, true)
  return /@import/i.test(code) || fetchesOutsideStrings(code)
}

let sheet: CSSStyleSheet | null = null
let applied = ''

type RuleParent = Pick<CSSStyleSheet, 'cssRules' | 'deleteRule'>

// @keyframes also has both, but its deleteRule takes a keyframe selector.
const isRuleParent = (rule: CSSRule): rule is CSSRule & RuleParent => 'cssRules' in rule && 'deleteRule' in rule && !(rule instanceof CSSKeyframesRule)

function stripDeclarations(rule: CSSRule): boolean {
  if (!('style' in rule) || !(rule.style instanceof CSSStyleDeclaration)) return false
  for (let i = rule.style.length - 1; i >= 0; i--) {
    const property = rule.style.item(i)
    if (cssValueFetches(rule.style.getPropertyValue(property))) rule.style.removeProperty(property)
  }
  return true
}

/** Keeps only kinds of rule it can check, so a new at-rule that fetches is dropped. */
function stripFetches(parent: RuleParent) {
  for (let i = parent.cssRules.length - 1; i >= 0; i--) {
    const rule = parent.cssRules[i]
    if (!rule) continue
    const hasStyle = stripDeclarations(rule)
    if (rule instanceof CSSKeyframesRule) {
      for (const frame of rule.cssRules) stripDeclarations(frame)
    } else if (isRuleParent(rule)) {
      stripFetches(rule)
    } else if (typeof CSSPropertyRule !== 'undefined' && rule instanceof CSSPropertyRule) {
      // A registered property's initial value is used wherever var() reads it.
      if (cssValueFetches(rule.initialValue ?? '')) parent.deleteRule(i)
    } else if (!hasStyle && !(typeof CSSLayerStatementRule !== 'undefined' && rule instanceof CSSLayerStatementRule) && !(rule instanceof CSSNamespaceRule)) {
      parent.deleteRule(i)
    }
  }
}

/** Puts a theme's CSS after the app's own styles, without anything that loads a file. */
export function applyThemeCss(css: string | undefined) {
  // Constructed sheets came in Safari 16.4; on older WebKit (macOS 12) a theme keeps its colors
  // and goes without its CSS.
  if (typeof document === 'undefined' || !('adoptedStyleSheets' in document) || typeof CSSStyleSheet === 'undefined' || !('replaceSync' in CSSStyleSheet.prototype)) return
  const next = css ?? ''
  if (next === applied) return
  applied = next
  try {
    // A new sheet, checked before it's adopted, so nothing in it can load before it's stripped.
    const fresh = new CSSStyleSheet()
    fresh.replaceSync(next)
    stripFetches(fresh)
    // Adopted sheets come after the document's own, and can't hold @import.
    document.adoptedStyleSheets = [...document.adoptedStyleSheets.filter((s) => s !== sheet), fresh]
    sheet = fresh
  } catch (err) {
    document.adoptedStyleSheets = document.adoptedStyleSheets.filter((s) => s !== sheet)
    sheet = null
    console.warn('Theme CSS left out', err)
  }
}
