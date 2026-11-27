import { loadEngine } from '../../engine/load'
import type { OcrDocumentInput, OcrPageResult } from './types'

function orderedPages(pages: readonly OcrPageResult[]): OcrPageResult[] {
  return [...pages].sort((a, b) => a.pageIndex - b.pageIndex)
}

export function ocrResultsToPlainText(pages: readonly OcrPageResult[]): string {
  return orderedPages(pages).map(page => page.text.trim() || page.blocks
    .flatMap(block => block.paragraphs)
    .map(paragraph => paragraph.text.trim() || paragraph.lines.map(line => line.text.trim()).filter(Boolean).join('\n'))
    .filter(Boolean).join('\n\n')).filter(Boolean).join('\n\n')
}

function lineHeight(page: OcrPageResult, box: { w: number, h: number }): number {
  const sideways = page.detectedRotation === 90 || page.detectedRotation === 270
  const xScale = (page.widthPt ?? 0) / page.widthPx
  const yScale = (page.heightPt ?? 0) / page.heightPx
  const scales = sideways ? [xScale, yScale] : [yScale, xScale]
  return (sideways ? box.w : box.h) * (scales.find(scale => Number.isFinite(scale) && scale > 0) ?? 0.75)
}

function median(heights: number[]): number {
  const sorted = heights.filter(height => Number.isFinite(height) && height > 0).sort((a, b) => a - b)
  if (!sorted.length) return NaN
  const middle = Math.floor(sorted.length / 2)
  return sorted.length % 2 ? sorted[middle] : sorted[middle - 1] / 2 + sorted[middle] / 2
}

const directionMarks = /^[\u061C\u200E\u200F\u202A-\u202E\u2066-\u2069]+/u
const rtlText = /[\p{Script=Hebrew}\p{Script=Arabic}]/u

function listMarker(line: string): { prefix: string, text: string } | undefined {
  const marks = directionMarks.exec(line)?.[0] ?? ''
  const content = line.slice(marks.length)
  const bullet = /^(?:[-*]\s+|[•▪◦‣]\s*)(\S.*)$/u.exec(content)
  if (bullet) return { prefix: '-', text: marks + bullet[1] }
  const number = /^(?:(\d{1,9})[.)]|\((\d{1,9})\))\s+(\S.*)$/u.exec(content)
  if (number) return { prefix: (number[1] ?? number[2]) + '.', text: marks + number[3] }
  // Markdown has no native Hebrew ordered list marker.
  if (/^[א-ת]\.\s+\S/u.test(content)) return { prefix: '-', text: line }
}

function escapeMarkdown(text: string): string {
  return text.replace(/[\\\x60*_[\]<>~|]/g, '\\$&').replace(/^(?=[#+\-=])/u, '\\')
    .replace(/^(\d+)([.)])(?=\s)/u, '$1\\$2')
}

function joinLines(lines: string[]): string {
  let text = lines[0] ?? ''
  for (const next of lines.slice(1)) {
    text = /[A-Za-z]-$/.test(text) && /^[a-z]/.test(next) ? text.slice(0, -1) + next : text + ' ' + next
  }
  return text
}

function markdownParagraph(lines: string[], rtl: boolean, ratio: number): string {
  const fragments: string[] = []
  let group: string[] = []
  function flush(headingRatio: number) {
    if (!group.length) return
    const list = listMarker(group[0])
    const text = joinLines(list ? [list.text, ...group.slice(1)] : group)
    const mark = (rtl || rtlText.test(text)) && !directionMarks.test(text) ? '\u200f' : ''
    const heading = group.length <= 2 && [...text].length <= 120
      ? [1.65, 1.35, 1.18].findIndex(threshold => headingRatio >= threshold) + 1 : 0
    const prefix = list?.prefix ?? '#'.repeat(heading)
    fragments.push((prefix ? prefix + ' ' : '') + mark + escapeMarkdown(text))
    group = []
  }
  for (const value of lines) {
    const line = value.trim()
    if (!line || listMarker(line)) flush(NaN)
    if (line) group.push(line)
  }
  flush(fragments.length ? NaN : ratio)
  return fragments.join('\n\n')
}

export function ocrResultsToMarkdown(pages: readonly OcrPageResult[]): string {
  const typicalHeight = median(pages.flatMap(page => page.blocks.flatMap(block => block.paragraphs
    .flatMap(paragraph => paragraph.lines.map(line => lineHeight(page, line.bbox))))))
  return orderedPages(pages).map(page => {
    const paragraphs = page.blocks.flatMap(block => block.paragraphs).map(paragraph => {
      const lines = paragraph.lines.map(line => line.text)
      const rtl = paragraph.rtl || paragraph.lines.some(line => line.rtl)
      if (!lines.some(line => line.trim())) return markdownParagraph(paragraph.text.split(/\r?\n/u), rtl, NaN)
      const ratio = median(paragraph.lines.map(line => lineHeight(page, line.bbox))) / typicalHeight
      return markdownParagraph(lines, rtl, ratio)
    }).filter(Boolean)
    return paragraphs.length ? paragraphs.join('\n\n') : markdownParagraph(page.text.split(/\r?\n/u), false, NaN)
  }).filter(Boolean).join('\n\n---\n\n')
}

/** `page`: the size in points to set the document on (a scanned PDF's own); Word's default otherwise. */
export async function ocrResultsToDocx(pages: readonly OcrPageResult[], title?: string, page?: { width: number; height: number }): Promise<Uint8Array> {
  const input: OcrDocumentInput = { title, pages: [...pages], page }
  const engine = await loadEngine()
  return engine.ocrToDocx(JSON.stringify(input))
}
