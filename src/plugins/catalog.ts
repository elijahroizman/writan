// Every plugin Writan knows about.
// Built-in plugins ship with the app and are always on. Word, PowerPoint, PDF, Markdown and text
// have their editors in src/editors. Images is built in too, but its viewer is a plugin bundle
// (`"bundled": true` in plugins/images/plugin.json): the app build copies it to /plugins/images/
// and PluginEditor loads it from there.
// Optional plugins are off until turned on in Settings or the setup screen. A `download` plugin
// is fetched as a signed archive from the Writan GitHub release (see src-tauri/src/plugins.rs
// and plugins/README.md). LaTeX is the only one.

import type { ComponentType } from 'react'
import { FileCode2, FileSpreadsheet, FileText, FileType2, Image as ImageIcon, NotepadText, Presentation, Sheet, Sigma } from 'lucide-react'
import latex from '../../plugins/latex/plugin.json'

export interface KindInfo {
  id: string
  label: string
  short: string
  /** In a sentence: “Open Word, PowerPoint and PDF files.” */
  prose: string
  extensions: string[]
  /** Extensions offered when saving; defaults to `extensions`. */
  saveExtensions?: string[]
  /** CSS custom property used for this kind's accent dot. */
  color: string
  icon: ComponentType<{ size?: number; strokeWidth?: number }>
  untitled: string
  /** The Home screen's “New …” card; none means you can't create one from scratch. */
  create?: { title: string; hint: string }
  /**
   * Whether the sidebar and inspector start open. An optional plugin's kind has a sidebar (the
   * editor's `context.sidebar`) only if `left` is true, and never an inspector.
   */
  panels: { left: boolean; right: boolean }
}

export interface PluginInfo {
  id: string
  name: string
  description: string
  kinds: KindInfo[]
  /** The engines' licenses, as SPDX ids (the full texts are in Settings › General › Licenses). */
  license: string
  homepage: string
  /** What the plugin's editor gets from Writan beyond the basics (see context.tex in api.ts). */
  services?: 'tex'[]
  /** Built in: always on. Otherwise how its files arrive. */
  source:
    | { type: 'builtin' }
    | {
        type: 'download'
        /** The plugin's own version; a new one is downloaded when this changes. */
        version: string
        /** Download size in bytes, shown before installing. */
        size: number
      }
}

export const PLUGINS: PluginInfo[] = [
  {
    id: 'office',
    name: 'Office',
    description: 'Word documents, PowerPoint presentations, Excel workbooks and CSV files, on Writan’s own engine.',
    homepage: 'https://github.com/elijahroizman/writan',
    license: 'OFL-1.1',
    source: { type: 'builtin' },
    kinds: [
      {
        id: 'docx',
        prose: 'Word',
        label: 'Word Document',
        short: 'Document',
        extensions: ['docx'],
        color: 'var(--kind-docx)',
        icon: FileText,
        untitled: 'Untitled Document',
        create: { title: 'New document', hint: 'Word (.docx)' },
        panels: { left: true, right: true },
      },
      {
        id: 'pptx',
        prose: 'PowerPoint',
        label: 'PowerPoint Presentation',
        short: 'Presentation',
        extensions: ['pptx', 'ppt'],
        saveExtensions: ['pptx'],
        color: 'var(--kind-pptx)',
        icon: Presentation,
        untitled: 'Untitled Presentation',
        create: { title: 'New presentation', hint: 'PowerPoint (.pptx)' },
        panels: { left: true, right: true },
      },
      {
        id: 'xlsx',
        prose: 'Excel',
        label: 'Excel Workbook',
        short: 'Workbook',
        extensions: ['xlsx', 'xlsm'],
        color: 'var(--kind-xlsx)',
        icon: FileSpreadsheet,
        untitled: 'Untitled Workbook',
        create: { title: 'New workbook', hint: 'Excel (.xlsx)' },
        panels: { left: true, right: true },
      },
      {
        id: 'csv',
        prose: 'CSV',
        label: 'CSV File',
        short: 'CSV',
        extensions: ['csv', 'tsv'],
        color: 'var(--kind-xlsx)',
        icon: Sheet,
        untitled: 'Untitled',
        panels: { left: false, right: true },
      },
    ],
  },
  {
    id: 'pdf',
    name: 'PDF',
    description: 'Read, annotate, fill in and rearrange PDFs, on PDFium.',
    homepage: 'https://pdfium.googlesource.com/pdfium/',
    license: 'BSD-3-Clause, MIT',
    source: { type: 'builtin' },
    kinds: [
      {
        id: 'pdf',
        prose: 'PDF',
        label: 'PDF Document',
        short: 'PDF',
        extensions: ['pdf'],
        color: 'var(--kind-pdf)',
        icon: FileType2,
        untitled: 'Untitled',
        panels: { left: true, right: true },
      },
    ],
  },
  {
    id: 'markdown',
    name: 'Markdown',
    description: 'Write and preview Markdown notes.',
    homepage: 'https://github.com/markdown-it/markdown-it',
    license: 'MIT',
    source: { type: 'builtin' },
    kinds: [
      {
        id: 'md',
        prose: 'Markdown',
        label: 'Markdown',
        short: 'Markdown',
        extensions: ['md', 'markdown'],
        color: 'var(--kind-md)',
        icon: FileCode2,
        untitled: 'Untitled Note',
        create: { title: 'New Markdown', hint: 'Notes (.md)' },
        // Markdown is mostly for reading, so its inspector starts hidden.
        panels: { left: true, right: false },
      },
    ],
  },
  {
    id: 'text',
    name: 'Plain text',
    description: 'Plain text files, without sidebars.',
    homepage: 'https://codemirror.net',
    license: 'MIT',
    source: { type: 'builtin' },
    kinds: [
      {
        id: 'txt',
        prose: 'text',
        label: 'Plain Text',
        short: 'Text',
        extensions: ['txt'],
        color: 'var(--kind-txt)',
        icon: NotepadText,
        untitled: 'Untitled Text',
        create: { title: 'New text file', hint: 'Plain text (.txt)' },
        panels: { left: false, right: false },
      },
    ],
  },
  {
    id: 'images',
    name: 'Images',
    description: 'A simple viewer for PNG, JPEG and HEIC pictures, with highlighting, drawing, cropping and turning.',
    homepage: 'https://github.com/elijahroizman/writan/tree/main/plugins/images',
    license: 'MIT, LGPL-3.0',
    source: { type: 'builtin' },
    kinds: [
      {
        id: 'image',
        prose: 'image',
        label: 'Image',
        short: 'Image',
        extensions: ['png', 'jpg', 'jpeg', 'heic', 'heif'],
        color: 'var(--kind-image)',
        icon: ImageIcon,
        untitled: 'Untitled Image',
        panels: { left: false, right: false },
      },
    ],
  },
  {
    id: 'latex',
    name: 'LaTeX',
    description: 'Write LaTeX and watch the PDF update as you go. Compiles with the TeX on your computer (MacTeX, MiKTeX or TeX Live), which Writan can install.',
    homepage: 'https://www.latex-project.org',
    license: 'MIT',
    services: ['tex'],
    source: { type: 'download', version: latex.version, size: latex.size },
    kinds: [
      {
        id: 'latex',
        prose: 'LaTeX',
        label: 'LaTeX Document',
        short: 'LaTeX',
        extensions: ['tex'],
        color: 'var(--kind-latex)',
        icon: Sigma,
        untitled: 'Untitled LaTeX',
        create: { title: 'New LaTeX document', hint: 'LaTeX (.tex)' },
        panels: { left: true, right: false },
      },
    ],
  },
]

export const pluginById = (id: string) => PLUGINS.find((p) => p.id === id)
export const pluginForKind = (kind: string) => PLUGINS.find((p) => p.kinds.some((k) => k.id === kind))!
export const isBuiltIn = (plugin: PluginInfo) => plugin.source.type === 'builtin'
