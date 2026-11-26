import type { DocKind } from './kinds'

export type ConvertFormat = 'pdf' | 'md' | 'docx' | 'pptx' | 'xlsx' | 'csv' | 'txt'

export interface ConvertTarget {
  format: ConvertFormat
  label: string
  hint: string
  extension: string
}

export const TARGET_INFO: Record<ConvertFormat, Omit<ConvertTarget, 'format'>> = {
  md: { label: 'Markdown (LLM-ready)', hint: 'Clean headings, lists, tables and formatting', extension: 'md' },
  pdf: { label: 'PDF', hint: 'Rendered pages as a PDF document', extension: 'pdf' },
  docx: { label: 'Word (.docx)', hint: 'Editable Word document', extension: 'docx' },
  pptx: { label: 'PowerPoint (.pptx)', hint: 'Slides deck', extension: 'pptx' },
  xlsx: { label: 'Excel (.xlsx)', hint: 'Spreadsheet workbook', extension: 'xlsx' },
  csv: { label: 'CSV (.csv)', hint: 'Comma-separated values', extension: 'csv' },
  txt: { label: 'Plain text (.txt)', hint: 'Unformatted text', extension: 'txt' },
}

const TARGETS_BY_KIND: Record<string, ConvertFormat[]> = {
  docx: ['md', 'pdf', 'pptx', 'xlsx', 'txt'],
  pptx: ['pdf', 'md', 'docx', 'txt'],
  xlsx: ['pdf', 'md', 'csv', 'docx', 'pptx'],
  csv: ['xlsx', 'md', 'pdf', 'docx'],
  md: ['pdf', 'docx', 'pptx', 'txt'],
  txt: ['pdf', 'docx', 'md'],
  pdf: ['md', 'txt', 'docx'],
  image: ['pdf', 'txt', 'md', 'docx'],
}

/** Whether converting a `kind` tab to `format` recognizes its text. A picture becomes a plain PDF. */
export function usesOcr(kind: DocKind, format: ConvertFormat): boolean {
  return (kind === 'image' && format !== 'pdf') || (kind === 'pdf' && format === 'docx')
}

/** Target formats a tab of `kind` can be converted to in one click. */
export function targetsForKind(kind: DocKind): ConvertTarget[] {
  const list = TARGETS_BY_KIND[kind] ?? []
  return list.map((format) => ({
    format,
    ...TARGET_INFO[format],
    ...(usesOcr(kind, format) ? { hint: 'Recognize text with offline OCR' } : {}),
  }))
}

/** Whether a tab can be exported or copied as clean Markdown. */
export function canExportMarkdown(kind: DocKind): boolean {
  return kind === 'docx' || kind === 'pptx' || kind === 'xlsx' || kind === 'csv' || kind === 'pdf' || kind === 'md' || kind === 'txt'
}

/** Whether a tab can be included in a bundled PDF. */
export function canIncludeInPdfBundle(kind: DocKind): boolean {
  return kind === 'pdf' || kind === 'docx' || kind === 'pptx' || kind === 'xlsx' || kind === 'csv' || kind === 'md' || kind === 'txt' || kind === 'image'
}
