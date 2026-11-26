import DOMPurify from 'dompurify'
import hljs from 'highlight.js/lib/common'
import MarkdownIt from 'markdown-it'
import { parse as parseYaml } from 'yaml'
import anchor from 'markdown-it-anchor'
// @ts-expect-error: no bundled types
import taskLists from 'markdown-it-task-lists'

const escapeHtml = (s: string) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]!)

export interface Heading {
  level: number
  text: string
  slug: string
  line: number
}

export const slugify = (s: string) =>
  s
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s-]/gu, '')
    .replace(/\s+/g, '-')

const md = new MarkdownIt({
  html: true,
  linkify: true,
  typographer: true,
  highlight(code: string, lang: string): string {
    if (lang && hljs.getLanguage(lang)) {
      try {
        return `<pre class="hljs"><code>${hljs.highlight(code, { language: lang, ignoreIllegals: true }).value}</code></pre>`
      } catch {
        /* fall through */
      }
    }
    return `<pre class="hljs"><code>${escapeHtml(code)}</code></pre>`
  },
})
  .use(anchor, { slugify })
  .use(taskLists, { enabled: false, label: true })

// Tag top-level blocks with their source line so the preview can follow the editor's scroll.
md.core.ruler.push('source_lines', (state) => {
  for (const token of state.tokens) {
    if (token.map && token.nesting !== -1 && token.level === 0) token.attrSet('data-line', String(token.map[0]))
  }
})

const FRONTMATTER = /^---[ \t]*\r?\n([\s\S]*?)\r?\n(?:---|\.\.\.)[ \t]*(?:\r?\n|$)/

/**
 * Split off a YAML front-matter block (Agent Skills, Jekyll, Obsidian…). The block is replaced
 * by blank lines so line numbers in the rest of the document stay the same.
 */
export function splitFrontmatter(source: string): { data: Record<string, unknown> | null; body: string } {
  const match = FRONTMATTER.exec(source)
  if (!match) return { data: null, body: source }
  try {
    const data = parseYaml(match[1])
    if (!data || typeof data !== 'object' || Array.isArray(data)) return { data: null, body: source }
    return { data: data as Record<string, unknown>, body: '\n'.repeat(match[0].split('\n').length - 1) + source.slice(match[0].length) }
  } catch {
    return { data: null, body: source }
  }
}

function humanize(key: string) {
  const spaced = key.replace(/[-_]+/g, ' ').replace(/([a-z])([A-Z])/g, '$1 $2')
  return spaced.charAt(0).toUpperCase() + spaced.slice(1)
}

function renderValue(value: unknown, depth = 0): string {
  if (value === null || value === undefined || value === '') return '<span class="fm-empty">—</span>'
  if (typeof value === 'boolean') return `<span class="fm-chip">${value ? 'Yes' : 'No'}</span>`
  if (value instanceof Date) return escapeHtml(value.toISOString().slice(0, 10))
  if (Array.isArray(value)) {
    if (value.every((v) => typeof v !== 'object' || v === null)) return value.map((v) => `<span class="fm-chip">${escapeHtml(String(v))}</span>`).join(' ')
    return value.map((v) => `<div class="fm-item">${renderValue(v, depth + 1)}</div>`).join('')
  }
  if (typeof value === 'object') {
    if (depth > 2) return `<code>${escapeHtml(JSON.stringify(value))}</code>`
    return `<table class="fm-nested"><tbody>${Object.entries(value as Record<string, unknown>)
      .map(([k, v]) => `<tr><th>${escapeHtml(humanize(k))}</th><td>${renderValue(v, depth + 1)}</td></tr>`)
      .join('')}</tbody></table>`
  }
  return escapeHtml(String(value)).replace(/\n/g, '<br>')
}

/** Front matter as a properties table; Agent Skills (name + description) lead with a Skill row. */
function renderFrontmatter(data: Record<string, unknown>): string {
  const isSkill = typeof data.name === 'string' && typeof data.description === 'string'
  const entries = isSkill
    ? [['Skill', data.name] as const, ...Object.entries(data).filter(([k]) => k !== 'name').map(([k, v]) => [humanize(k), v] as const)]
    : Object.entries(data).map(([k, v]) => [humanize(k), v] as const)
  const title = isSkill ? '' : `<div class="fm-title"><span class="fm-kind">Properties</span></div>`
  const rows = entries.map(([k, v]) => `<tr><th>${escapeHtml(k)}</th><td>${renderValue(v)}</td></tr>`).join('')
  return `<section class="frontmatter" data-line="0">${title}<table><tbody>${rows}</tbody></table></section>`
}

export function renderMarkdown(source: string): { html: string; headings: Heading[]; minHeadingLevel: number; frontmatter: Record<string, unknown> | null } {
  const env = {}
  const { data: frontmatter, body } = splitFrontmatter(source)
  const tokens = md.parse(body, env)
  const headings: Heading[] = []
  let minHeadingLevel = 6
  for (let i = 0; i < tokens.length; i++) {
    const token = tokens[i]
    if (token.type === 'heading_open') {
      const inline = tokens[i + 1]
      let text = inline?.content ?? ''
      if (inline?.children?.length) {
        text = ''
        for (const c of inline.children) text += c.content
      }
      const level = Number(token.tag.slice(1))
      if (level < minHeadingLevel) minHeadingLevel = level
      headings.push({
        level,
        text,
        slug: String(token.attrGet('id') ?? slugify(text)),
        line: token.map?.[0] ?? 0,
      })
    }
  }
  const rendered = (frontmatter ? renderFrontmatter(frontmatter) : '') + md.renderer.render(tokens, md.options, env)
  const html = DOMPurify.sanitize(rendered, {
    ADD_ATTR: ['data-line', 'target'],
    // Inline <script>/<style>/forms from untrusted files never run.
    FORBID_TAGS: ['style', 'form'],
  })
  return { html, headings, minHeadingLevel: headings.length ? minHeadingLevel : 1, frontmatter }
}
