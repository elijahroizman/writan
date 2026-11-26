// The contract between Writan and an optional plugin's editor. A plugin's index.js is an ES
// module exporting `apiVersion` and `mount`. It brings its own UI framework and renders into
// the element it is given, so the app and its plugins don't have to share React versions.
// Keep this file free of imports: plugins compile against it too.

/** Bump when the contract changes in a way old plugins can't follow. */
export const PLUGIN_API_VERSION = 1

export type Theme = 'light' | 'dark'

/** Source-and-preview editors (Markdown, LaTeX): the preview, both side by side, or the source. */
export type ViewMode = 'read' | 'split' | 'write'

export interface PluginEditorContext {
  /** The file's bytes; null for a new, empty document. */
  bytes: Uint8Array | null
  /** The tab's title. */
  title: string
  theme: Theme
  /** Call after every change the user makes; Writan marks the tab unsaved and autosaves. */
  onChange(): void
  /** Open a link in the default browser. */
  openExternal(url: string): void
  /** Print this content (an image or a page of HTML) with the system print dialog. */
  print(content: Node): Promise<void>
  /**
   * An element in Writan's left sidebar to render into (a table of contents, say). Only for
   * kinds whose catalog entry has `panels.left`; Writan shows and hides it with ⌘\.
   */
  sidebar?: HTMLElement
  /** For editors with search(): the matches changed without a new query (more content loaded). */
  searchChanged?(result: SearchResult): void
  /** Where the file is on disk; null for a new document, and in the browser build. */
  path?: string | null
  /**
   * Seconds to wait after typing stops before rebuilding a slow preview (a LaTeX compile). The
   * user sets it in Settings › General, so read it each time.
   */
  previewDelay?(): number
  /** Open a document in a tab of its own, of the kind its name's extension says (a compiled PDF). */
  openDocument?(document: { bytes: Uint8Array; name: string }): void
  /**
   * Replaces the contents of the tab `openDocument` opened, without switching to it. Returns
   * false once that tab is closed, saved or edited (it's the user's now), and the plugin should
   * stop updating it.
   */
  updateDocument?(document: { bytes: Uint8Array; name: string }): boolean
  /** Save bytes to a file the user picks (an export). Resolves false if they cancel. */
  saveFile?(file: { bytes: Uint8Array; name: string }): Promise<boolean>
  /** The host's platform-specific shortcut label. */
  shortcut?(keys: string): string
  /** Local OCR, provided by the host to the bundled Images plugin. */
  imageOcr?: ImageOcrService
  /** A read-only view of a PDF in `element`, drawn by Writan's own PDF engine. */
  createPdfView?(element: HTMLElement): PdfView
  /** The system's TeX, for plugins whose catalog entry lists the `tex` service. */
  tex?: TexService
}

export interface ImageOcrService {
  selectedLanguages(): string[]
  recognize(image: { bytes: Uint8Array; mime: string }, options: {
    signal?: AbortSignal
    onProgress?(message: string): void
  }): Promise<ImageOcrResult>
}

export interface ImageOcrResult {
  text: string
  markdown: string
  /** The size of the picture that was read; `find`'s boxes are in its pixels. */
  width: number
  height: number
  /** Where `query` is in the picture: for each match, one box per line it spans. */
  find(query: string): { x: number; y: number; w: number; h: number }[][]
  saveDocx(): Promise<boolean>
  saveSearchablePdf(signal?: AbortSignal): Promise<boolean>
}

export interface PdfView {
  /** Shows a PDF, keeping the place and zoom of the one it replaces. Rejects if it can't be read. */
  show(bytes: Uint8Array): Promise<void>
  zoom(direction: 'in' | 'out' | 'reset'): void
  destroy(): void
}

export interface TexStatus {
  installed: boolean
  /** "TeX Live 2026", "MiKTeX 25.12"… */
  distribution?: string
  /** Engines found: pdflatex, xelatex, lualatex. */
  engines: string[]
}

export interface TexResult {
  /** The compiled PDF; null when TeX made none (a fatal error). */
  pdf: Uint8Array | null
  /** TeX's log, to read errors and warnings from. */
  log: string
  /** The file TeX compiled (a copy of the document); the log's errors in the document name it. */
  file: string
}

export interface TexService {
  status(): Promise<TexStatus>
  /**
   * Compiles `source` as the tab's document. Files it \input s or \includegraphics come from the
   * file's folder (a new document has none). A compile asked for while one runs waits its turn.
   */
  compile(source: string, options: { engine: string }): Promise<TexResult>
  /** Offers to install TeX (Writan's own dialog). Resolves with the status afterwards. */
  install(): Promise<TexStatus>
}

export interface SearchResult {
  count: number
  /** Index of the selected match; -1 for none. */
  current: number
  /** Still looking (reading the text in a picture): the count isn't known yet. */
  pending?: boolean
}

export interface PluginEditor {
  /** The document as it should be saved. */
  getBytes(): Promise<Uint8Array>
  /** The file was just saved. */
  markSaved?(): void
  setTheme?(theme: Theme): void
  /** The tab was shown (true) or hidden (false). */
  setActive?(active: boolean): void
  find?(): void
  /**
   * Instead of find(): Writan shows its own find bar and asks for matches. `step` moves the
   * selection to the next (1) or previous (-1) match; 0 is a new query. An empty query clears.
   */
  search?(query: string, step: 1 | -1 | 0): SearchResult
  zoom?(direction: 'in' | 'out' | 'reset'): void
  print?(): Promise<void>
  recognizeText?(): void
  /**
   * For source-and-preview editors: what each view shows, as the switch's tooltips put it
   * (`{ read: 'Diagram', split: 'Source and diagram', write: 'Source' }`). With this, Writan
   * shows the same view switch as Markdown and its shortcuts (⌘E cycles) call setView.
   */
  views?: Record<ViewMode, string>
  setView?(view: ViewMode): void
  /** The tab closed: release everything. */
  destroy(): void
}

export interface PluginModule {
  apiVersion: number
  mount(element: HTMLElement, context: PluginEditorContext): PluginEditor | Promise<PluginEditor>
}
