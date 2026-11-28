// Every usage event Writan can send, and every value its properties can take. A property is a
// number, a boolean or one of the strings listed here, so a call site can't pass a file name,
// a path, document text or an error message: the type checker refuses it. To add an event, add
// it to `Events` and to the table in docs/internals/analytics.md.

/** A document type, as plugins/catalog.ts names them. Anything else is `other`. */
export const KINDS = ['docx', 'pptx', 'xlsx', 'csv', 'pdf', 'md', 'txt', 'image', 'latex'] as const
export type Kind = (typeof KINDS)[number] | 'other'

/** File extensions Writan opens. Anything else is `other`. */
export const EXTENSIONS = [
  'docx', 'pptx', 'ppt', 'xlsx', 'xlsm', 'csv', 'tsv', 'pdf', 'md', 'markdown', 'txt', 'tex',
  'png', 'jpg', 'jpeg', 'heic', 'heif', 'webp',
] as const
export type Extension = (typeof EXTENSIONS)[number] | 'other'

export type SizeBucket = '<100KB' | '100KB-1MB' | '1-10MB' | '10-50MB' | '50MB+'

export type MinutesBucket = '<5' | '5-15' | '15-60' | '60-180' | '180+'

/** Where opening a document stopped. */
export type OpenStage =
  /** Reading the file from disk. */
  | 'read'
  /** The editor couldn't read what was in it. */
  | 'parse'
  /** The editor crashed while drawing it. */
  | 'render'
  /** An optional plugin's editor didn't start. */
  | 'plugin'
  /** Writan doesn't open the file's type. */
  | 'unsupported'

/** How the document got opened. */
export type OpenSource = 'open' | 'restore'

export type Feature =
  | 'split_view'
  | 'focus_mode'
  | 'typewriter_scrolling'
  | 'pdf_sign'
  | 'pdf_form_fill'
  | 'pdf_to_word'
  | 'ocr'
  | 'version_history'
  | 'command_palette'
  | 'find'
  | 'find_replace'
  | 'theme_editor'
  | 'latex_compile'
  | 'present_slides'
  | 'print'
  | 'open_folder'
  | 'new_document'
  | 'bundle_pdf'

/** What a document was exported or converted to (lib/convertMeta.ts). */
export type ExportFormat = 'pdf' | 'docx' | 'pptx' | 'xlsx' | 'csv' | 'md' | 'txt'

/** Where the converted copy went: a new tab, a file, or the clipboard. */
export type ExportDelivery = 'open' | 'save' | 'copy'

/** Which part of the app an error came from: the editor in front, or the shell around it. */
export type ErrorArea = Kind | 'home' | 'shell'

export type ErrorSource = 'uncaught' | 'rejection' | 'render'

/** JavaScript's and the browser's own error names. A name of Writan's own, or a minified one, is `other`. */
export const ERROR_NAMES = [
  'Error', 'TypeError', 'RangeError', 'ReferenceError', 'SyntaxError', 'URIError', 'EvalError', 'AggregateError',
  'AbortError', 'QuotaExceededError', 'NotAllowedError', 'NotFoundError', 'NotSupportedError', 'SecurityError',
  'NetworkError', 'DataCloneError', 'InvalidStateError', 'OperationError', 'TimeoutError',
  'CompileError', 'LinkError', 'RuntimeError',
] as const
/** `string` is a rejection with a plain string, which is how the native side reports its errors. */
export type ErrorName = (typeof ERROR_NAMES)[number] | 'string' | 'other'

export type CollabRole = 'host' | 'guest'

/** The agents Writan knows (src/collab/agents/identity.ts). Another MCP client is `other`. */
export type AgentName = 'claude' | 'codex' | 'antigravity' | 'grok' | 'hermes' | 'pi' | 'other'

/** The optional plugins in plugins/catalog.ts. */
export type PluginId = 'latex' | 'other'

declare const VERSION: unique symbol
/** A version number like 0.19.0, made by `versionOf` only. */
export type Version = string & { readonly [VERSION]: true }

export interface Events {
  /** Once per launch, when the last session has been restored. */
  app_launched: {
    /** No earlier Writan state was on this computer. */
    first_launch: boolean
    /** From the window starting to load until the restored session is on screen. */
    launch_ms: number
  }
  /** Once per calendar day the app runs, at launch or while it stays open. */
  app_active: Record<never, never>
  /** On the first launch after the app's version changed. */
  app_updated: { from_version: Version; to_version: Version }
  document_opened: {
    kind: Kind
    extension: Extension
    size_bucket: SizeBucket
    source: OpenSource
    /** From asking to open it until its editor shows it. Left out when a password was asked for. */
    open_ms?: number
    /** Office documents: whether the document engine had to start for this one. */
    engine_cold?: boolean
  }
  document_open_failed: { kind: Kind; extension: Extension; stage: OpenStage }
  document_saved: { kind: Kind; autosave: boolean }
  document_exported: { from_kind: Kind; to_format: ExportFormat; delivery: ExportDelivery }
  feature_used: { feature: Feature; kind?: Kind }
  collab_started: { kind: Kind; role: CollabRole; agents_only: boolean }
  collab_ended: { kind: Kind; role: CollabRole; agents_only: boolean; peak_participants: number; minutes_bucket: MinutesBucket }
  agent_connected: { agent: AgentName; local: boolean }
  plugin_enabled: { plugin: PluginId }
  plugin_disabled: { plugin: PluginId }
  error: { area: ErrorArea; name: ErrorName; source: ErrorSource }
  /** Recorded by the welcome screen when someone finishes it. */
  onboarding_finished: Record<never, never>
}

export type EventName = keyof Events

const KNOWN_KINDS: ReadonlySet<string> = new Set(KINDS)
const KNOWN_EXTENSIONS: ReadonlySet<string> = new Set(EXTENSIONS)
const KNOWN_ERRORS: ReadonlySet<string> = new Set(ERROR_NAMES)

export function kindOf(kind: string | null | undefined): Kind {
  return kind && KNOWN_KINDS.has(kind) ? (kind as Kind) : 'other'
}

/** The extension of a path, from the list above; the path itself goes no further. */
export function extensionOf(path: string): Extension {
  const name = path.split(/[\\/]/).pop() ?? ''
  const dot = name.lastIndexOf('.')
  const ext = dot > 0 ? name.slice(dot + 1).toLowerCase() : ''
  return KNOWN_EXTENSIONS.has(ext) ? (ext as Extension) : 'other'
}

export function sizeBucket(bytes: number): SizeBucket {
  const mb = bytes / (1024 * 1024)
  if (bytes < 100 * 1024) return '<100KB'
  if (mb < 1) return '100KB-1MB'
  if (mb < 10) return '1-10MB'
  if (mb < 50) return '10-50MB'
  return '50MB+'
}

export function minutesBucket(minutes: number): MinutesBucket {
  if (minutes < 5) return '<5'
  if (minutes < 15) return '5-15'
  if (minutes < 60) return '15-60'
  if (minutes < 180) return '60-180'
  return '180+'
}

/**
 * A duration in coarse steps: 50 ms under a second, 250 ms under ten, whole seconds after that,
 * and at most ten minutes. Precise enough for percentiles, too coarse to tell one run from another.
 */
export function roundMs(ms: number): number {
  if (!Number.isFinite(ms) || ms < 0) return 0
  if (ms < 1000) return Math.round(ms / 50) * 50
  if (ms < 10_000) return Math.round(ms / 250) * 250
  return Math.min(600_000, Math.round(ms / 1000) * 1000)
}

const VERSION_RE = /^\d{1,4}\.\d{1,4}\.\d{1,4}(?:-[0-9A-Za-z.]{1,32})?$/

/** A version number, or `other` for anything that doesn't look like one. */
export function versionOf(version: string): Version {
  return (VERSION_RE.test(version) ? version : 'other') as Version
}

export function errorNameOf(error: unknown): ErrorName {
  if (typeof error === 'string') return 'string'
  if (error && typeof error === 'object' && 'name' in error && typeof error.name === 'string' && KNOWN_ERRORS.has(error.name)) {
    return error.name as ErrorName
  }
  return 'other'
}

export function agentOf(id: string | null | undefined): AgentName {
  switch (id) {
    case 'claude':
    case 'codex':
    case 'antigravity':
    case 'grok':
    case 'hermes':
    case 'pi':
      return id
    default:
      return 'other'
  }
}

export function pluginOf(id: string): PluginId {
  return id === 'latex' ? 'latex' : 'other'
}
