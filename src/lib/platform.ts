// Thin wrapper over the native shell. Inside the Tauri app everything goes to
// Rust; in a plain browser (`npm run dev` without Tauri) it falls back to file
// pickers and downloads so the UI can still be developed and tested.

import type { DocKind } from './kinds'
import { KINDS, kindFromPath } from './kinds'
import { showDialog } from './dialogs'

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
export const isMac = typeof navigator !== 'undefined' && /Mac/.test(navigator.platform || navigator.userAgent)
export const isWindows = typeof navigator !== 'undefined' && /Win/.test(navigator.platform || navigator.userAgent)
export const isLinux = typeof navigator !== 'undefined' && /Linux/.test(navigator.platform || navigator.userAgent) && !/Android/.test(navigator.userAgent)

/** “Show in Finder” on a Mac, “Show in File Explorer” on Windows. */
export const revealLabel = isMac ? 'Show in Finder' : isWindows ? 'Show in File Explorer' : 'Show in folder'

const KEY_NAMES: Record<string, string> = {
  '↩': 'Enter',
  '−': 'Minus',
  '+': 'Plus',
  '⌫': 'Backspace',
  '⌦': 'Delete',
  '⇥': 'Tab',
  '←': 'Left',
  '→': 'Right',
  '↑': 'Up',
  '↓': 'Down',
}

/**
 * Shortcuts are written the Mac way in the source (“Duplicate (⇧⌘D)”). Everywhere else they
 * read Ctrl+Shift+D, the way Windows apps write them.
 */
export function shortcut(text: string): string {
  if (isMac) return text
  return text.replace(/([⌃⌥⇧⌘]+)([^\s)]+)|↩/g, (match, mods: string | undefined, key: string | undefined) => {
    if (!mods || !key) return KEY_NAMES[match] ?? match
    const parts: string[] = []
    if (/[⌘⌃]/.test(mods)) parts.push('Ctrl')
    if (mods.includes('⌥')) parts.push('Alt')
    if (mods.includes('⇧')) parts.push('Shift')
    parts.push(KEY_NAMES[key] ?? key)
    return parts.join('+')
  })
}

const core = () => import('@tauri-apps/api/core')
const dialog = () => import('@tauri-apps/plugin-dialog')

export interface OpenedFile {
  path: string
  name: string
  bytes: Uint8Array
}

// Browser fallback keeps picked files in memory under a fake path.
const browserFiles = new Map<string, Uint8Array>()

export function basename(path: string): string {
  return path.split(/[\\/]/).pop() || path
}

/** Shorten a path in your home folder to “~/…”. */
export function tildify(path: string): string {
  return path.replace(/^\/Users\/[^/]+/, '~').replace(/^\/home\/[^/]+/, '~').replace(/^[A-Z]:\\Users\\[^\\]+/i, '~')
}

export function dirname(path: string): string {
  const i = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))
  return i <= 0 ? path.slice(0, i + 1) : path.slice(0, i)
}

export function joinPath(dir: string, rel: string): string {
  const sep = dir.includes('\\') && !dir.includes('/') ? '\\' : '/'
  const parts = (dir + sep + rel).split(/[\\/]/)
  const out: string[] = []
  for (const part of parts) {
    if (part === '..') out.pop()
    else if (part !== '.' && (part !== '' || out.length === 0)) out.push(part)
  }
  return out.join(sep)
}

export async function readFile(path: string): Promise<Uint8Array> {
  if (!isTauri) {
    const bytes = browserFiles.get(path)
    if (bytes) return bytes
    const res = await fetch(path)
    if (!res.ok) throw new Error(`Couldn't open “${basename(path)}”`)
    return new Uint8Array(await res.arrayBuffer())
  }
  const { invoke } = await core()
  const data = await invoke<ArrayBuffer | number[]>('read_file', { path })
  return data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data)
}

/**
 * Save bytes to a file. `history` also keeps this version in the file's version history, named
 * `label` if given (see src-tauri/src/history.rs).
 */
export async function writeFile(path: string, bytes: Uint8Array, opts: { history?: boolean; label?: string } = {}): Promise<void> {
  if (!isTauri) {
    if (opts.history) browserHistory(path, bytes, opts.label)
    browserFiles.set(path, bytes)
    download(basename(path), bytes)
    return
  }
  const { invoke } = await core()
  const headers: Record<string, string> = { 'x-path': encodeURIComponent(path) }
  if (opts.history) headers['x-history'] = '1'
  if (opts.history && opts.label?.trim()) headers['x-history-label'] = encodeURIComponent(opts.label.trim())
  await invoke('write_file', bytes, { headers })
}

// ── Version history

export interface FileVersion {
  id: string
  hash: string
  /** ms since 1970 */
  time: number
  size: number
  label?: string
  /** 'save'; 'original', the file as it was before Writan first saved over it; or 'outside', a change another app made before Writan saved over it. */
  source: string
}

// In the browser, versions live in memory for the session.
const browserVersions = new Map<string, { version: FileVersion; bytes: Uint8Array }[]>()
function browserHistory(path: string, bytes: Uint8Array, label?: string) {
  const list = browserVersions.get(path) ?? []
  const old = browserFiles.get(path)
  const time = Date.now()
  if (!list.length && old) list.unshift({ version: { id: `${time}-o`, hash: '', time: time - 1, size: old.length, source: 'original' }, bytes: old })
  list.unshift({ version: { id: `${time}-${list.length}`, hash: '', time, size: bytes.length, label: label?.trim() || undefined, source: 'save' }, bytes: bytes.slice() })
  browserVersions.set(path, list)
}

/** A file's saved versions, newest first. */
export async function listVersions(path: string): Promise<FileVersion[]> {
  if (!isTauri) return (browserVersions.get(path) ?? []).map((v) => v.version)
  const { invoke } = await core()
  return invoke<FileVersion[]>('history_list', { path })
}

export async function readVersion(path: string, id: string): Promise<Uint8Array> {
  if (!isTauri) {
    const hit = browserVersions.get(path)?.find((v) => v.version.id === id)
    if (!hit) throw new Error('That version is gone.')
    return hit.bytes
  }
  const { invoke } = await core()
  const data = await invoke<ArrayBuffer | number[]>('history_read', { path, id })
  return data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data)
}

export async function labelVersion(path: string, id: string, label: string | null): Promise<void> {
  if (!isTauri) {
    const hit = browserVersions.get(path)?.find((v) => v.version.id === id)
    if (hit) hit.version.label = label?.trim() || undefined
    return
  }
  const { invoke } = await core()
  await invoke('history_label', { path, id, label })
}

export async function deleteVersion(path: string, id: string): Promise<void> {
  if (!isTauri) {
    browserVersions.set(path, (browserVersions.get(path) ?? []).filter((v) => v.version.id !== id))
    return
  }
  const { invoke } = await core()
  await invoke('history_delete', { path, id })
}

/** Bytes every file's history takes on disk. */
export async function historyUsage(): Promise<number> {
  if (!isTauri) return [...browserVersions.values()].flat().reduce((n, v) => n + v.bytes.length, 0)
  const { invoke } = await core()
  return invoke<number>('history_usage')
}

export async function clearAllHistory(): Promise<void> {
  if (!isTauri) {
    browserVersions.clear()
    return
  }
  const { invoke } = await core()
  await invoke('history_clear_all')
}

/** Fetch a public https resource (used for video thumbnails). */
export async function fetchBytes(url: string): Promise<Uint8Array> {
  if (!isTauri) {
    const res = await fetch(url)
    if (!res.ok) throw new Error(`Couldn't download ${url}`)
    return new Uint8Array(await res.arrayBuffer())
  }
  const { invoke } = await core()
  const data = await invoke<ArrayBuffer | number[]>('fetch_bytes', { url })
  return data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data)
}

export interface DirEntry {
  name: string
  path: string
  isDir: boolean
}

/** Test builds can fake a file system for the folder sidebar (see docs/internals/development.md). */
type FsFake = { list(path: string): DirEntry[]; isDir(path: string): boolean }
const fsFake = () => (window as unknown as { __fsFake?: FsFake }).__fsFake

/** A folder's contents: subfolders first, then files, hidden ones left out. */
export async function listDir(path: string): Promise<DirEntry[]> {
  if (!isTauri) return fsFake()?.list(path) ?? []
  const { invoke } = await core()
  return invoke<DirEntry[]>('list_dir', { path })
}

/** Files under a folder with these extensions, up to `limit` of them. */
export async function walkDir(root: string, extensions: string[], limit = 5000): Promise<string[]> {
  if (!isTauri) {
    const fake = fsFake()
    if (!fake) return []
    const out: string[] = []
    const visit = (dir: string) => {
      for (const e of fake.list(dir)) {
        if (e.isDir) visit(e.path)
        else if (extensions.includes(e.name.split('.').pop()!.toLowerCase())) out.push(e.path)
      }
    }
    visit(root)
    return out.slice(0, limit)
  }
  const { invoke } = await core()
  return invoke<string[]>('walk_dir', { root, extensions, limit })
}

export async function isDirectory(path: string): Promise<boolean> {
  if (!isTauri) return fsFake()?.isDir(path) ?? false
  const { invoke } = await core()
  const info = await invoke<{ exists: boolean; isDir: boolean }>('path_info', { path })
  return info.exists && info.isDir
}

/** Ask for a folder. Null when cancelled. */
export async function pickFolder(): Promise<string | null> {
  if (!isTauri) return fsFake() ? '/fake' : null
  const { open } = await dialog()
  const picked = await open({ directory: true, multiple: false })
  return typeof picked === 'string' ? picked : null
}

/** A file's size and modified time, to tell later whether something else changed it. Null in the browser, or when there's no file. */
export async function fileStamp(path: string): Promise<string | null> {
  if (!isTauri || path.startsWith('browser:')) return null
  const { invoke } = await core()
  const info = await invoke<{ exists: boolean; size: number; modifiedMs: number }>('path_info', { path })
  return info.exists ? `${info.size}:${info.modifiedMs}` : null
}

export async function pathExists(path: string): Promise<boolean> {
  if (!isTauri) return browserFiles.has(path) || !!fsFake()?.isDir(path)
  const { invoke } = await core()
  const info = await invoke<{ exists: boolean }>('path_info', { path })
  return info.exists
}

const ALL_EXTENSIONS = Object.values(KINDS).flatMap((k) => k.extensions)

export async function pickFiles(): Promise<OpenedFile[]> {
  if (!isTauri) return pickFilesInBrowser()
  const { open } = await dialog()
  const picked = await open({
    multiple: true,
    directory: false,
    filters: [
      { name: 'Documents', extensions: ALL_EXTENSIONS },
      ...Object.values(KINDS).map((k) => ({ name: k.label, extensions: k.extensions })),
    ],
  })
  if (!picked) return []
  const paths = Array.isArray(picked) ? picked : [picked]
  return Promise.all(paths.map(async (path) => ({ path, name: basename(path), bytes: await readFile(path) })))
}

/** ⌘O: documents or a folder, from one dialog (see open_dialog.rs). Empty when cancelled. */
export async function pickPaths(): Promise<string[]> {
  const { invoke } = await core()
  return invoke<string[]>('pick_paths', { extensions: ALL_EXTENSIONS })
}

function pickFilesInBrowser(): Promise<OpenedFile[]> {
  return new Promise((resolve) => {
    const input = document.createElement('input')
    input.type = 'file'
    input.multiple = true
    input.accept = ALL_EXTENSIONS.map((e) => '.' + e).join(',')
    input.onchange = async () => {
      const files = Array.from(input.files ?? [])
      resolve(await Promise.all(files.map(fileToOpened)))
    }
    input.oncancel = () => resolve([])
    input.click()
  })
}

const IMAGE_TYPES: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  heic: 'image/heic',
  svg: 'image/svg+xml',
}

/** Ask for pictures. Resolves to none when cancelled. */
export async function pickImages({ multiple = false } = {}): Promise<File[]> {
  if (!isTauri) {
    return new Promise((resolve) => {
      const input = document.createElement('input')
      input.type = 'file'
      input.multiple = multiple
      input.accept = Object.values(IMAGE_TYPES).join(',')
      input.onchange = () => resolve(Array.from(input.files ?? []))
      input.oncancel = () => resolve([])
      input.click()
    })
  }
  const { open } = await dialog()
  const picked = await open({ multiple, directory: false, filters: [{ name: 'Images', extensions: Object.keys(IMAGE_TYPES) }] })
  if (!picked) return []
  const paths = Array.isArray(picked) ? picked : [picked]
  return Promise.all(
    paths.map(async (path) => {
      const name = basename(path)
      const type = IMAGE_TYPES[name.split('.').pop()!.toLowerCase()] ?? ''
      return new File([(await readFile(path)) as Uint8Array<ArrayBuffer>], name, { type })
    }),
  )
}

export async function fileToOpened(file: File): Promise<OpenedFile> {
  const bytes = new Uint8Array(await file.arrayBuffer())
  const path = `browser:/${crypto.randomUUID()}/${file.name}`
  browserFiles.set(path, bytes)
  return { path, name: file.name, bytes }
}

/** Ask where to save. Returns null when cancelled. */
export async function pickSavePath(suggestedName: string, kind: DocKind): Promise<string | null> {
  const ext = KINDS[kind].extensions[0]
  const name = suggestedName.toLowerCase().endsWith('.' + ext) ? suggestedName : `${suggestedName}.${ext}`
  if (!isTauri) return `browser:/${crypto.randomUUID()}/${name}`
  const { save } = await dialog()
  const path = await save({
    defaultPath: name,
    filters: [{ name: KINDS[kind].label, extensions: KINDS[kind].saveExtensions ?? KINDS[kind].extensions }],
  })
  if (!path) return null
  // The macOS save panel can drop the extension if the user deletes it.
  return kindFromPath(path) ? path : `${path}.${ext}`
}

/** Asks a yes-or-no question in the app's own dialog. Resolves true for the OK button. */
export async function confirmDialog(
  message: string,
  opts: { title?: string; okLabel?: string; cancelLabel?: string; kind?: 'warning' | 'info'; danger?: boolean } = {},
): Promise<boolean> {
  const answer = await showDialog({
    kind: opts.kind ?? 'warning',
    title: opts.title ?? message,
    message: opts.title ? message : undefined,
    buttons: [
      { label: opts.cancelLabel ?? 'Cancel', value: 'cancel' },
      { label: opts.okLabel ?? 'OK', value: 'ok', primary: true, danger: opts.danger },
    ],
    cancelValue: 'cancel',
  })
  return answer === 'ok'
}

export async function showMessage(message: string, title?: string): Promise<void> {
  await showDialog({
    kind: 'info',
    title: title ?? message,
    message: title ? message : undefined,
    buttons: [{ label: 'OK', value: 'ok', primary: true }],
    cancelValue: 'ok',
  })
}

export async function showError(message: string, title?: string): Promise<void> {
  await showDialog({
    kind: 'error',
    title: title ?? message,
    message: title ? message : undefined,
    buttons: [{ label: 'OK', value: 'ok', primary: true }],
    cancelValue: 'ok',
  })
}

/** Web and email links: the only kinds a document may open (never javascript:, file: or another app's scheme). */
export const isWebLink = (url: string) => /^(https?:\/\/|mailto:)/i.test(url)

/** Open a link in the browser or mail app. Anything else is refused with a message, never thrown. */
export async function openExternal(url: string): Promise<void> {
  if (!isWebLink(url)) {
    await showError(url.length > 200 ? url.slice(0, 200) + '…' : url, 'Writan opens only web and email links')
    return
  }
  if (!isTauri) {
    window.open(url, '_blank', 'noopener')
    return
  }
  try {
    const { openUrl } = await import('@tauri-apps/plugin-opener')
    await openUrl(url)
  } catch (err) {
    await showError(String(err), 'Couldn’t open the link')
  }
}

export async function revealInFinder(path: string): Promise<void> {
  if (!isTauri || path.startsWith('browser:')) return
  const { revealItemInDir } = await import('@tauri-apps/plugin-opener')
  await revealItemInDir(path)
}

export async function takePendingOpens(): Promise<string[]> {
  if (!isTauri) return []
  const { invoke } = await core()
  return invoke<string[]>('take_pending_opens')
}

export interface CliStatus {
  /** Settings can install it (a Mac). */
  supported: boolean
  installed: boolean
  /** Something else called `writan` is in the way. */
  taken: boolean
  path: string
}

/** The `writan` shell command (src-tauri/src/cli.rs). */
export async function cliStatus(): Promise<CliStatus> {
  if (!isTauri) return { supported: false, installed: false, taken: false, path: '' }
  const { invoke } = await core()
  return invoke<CliStatus>('cli_status')
}

/** Install or remove it. Resolves false when the password prompt was cancelled. */
export async function setCliInstalled(on: boolean): Promise<boolean> {
  const { invoke } = await core()
  try {
    await invoke(on ? 'cli_install' : 'cli_uninstall')
    return true
  } catch (err) {
    if (String(err) === 'cancelled') return false
    throw err
  }
}

export async function quitApp(): Promise<void> {
  if (!isTauri) return
  const { invoke } = await core()
  await invoke('quit_app')
}

type Unlisten = () => void

export async function listen<T>(event: string, handler: (payload: T) => void): Promise<Unlisten> {
  if (!isTauri) return () => {}
  const { listen } = await import('@tauri-apps/api/event')
  return listen<T>(event, (e) => handler(e.payload))
}

/** Files dragged from Finder onto the window. */
export async function onFileDrop(handler: (paths: string[]) => void, onHover?: (hovering: boolean) => void): Promise<Unlisten> {
  if (!isTauri) return () => {}
  const { getCurrentWebview } = await import('@tauri-apps/api/webview')
  return getCurrentWebview().onDragDropEvent((event) => {
    const p = event.payload
    if (p.type === 'over' || p.type === 'enter') onHover?.(true)
    else if (p.type === 'leave') onHover?.(false)
    else if (p.type === 'drop') {
      onHover?.(false)
      handler(p.paths)
    }
  })
}

/** Intercept the window close button so unsaved tabs can prompt. */
export async function onCloseRequested(shouldClose: () => Promise<boolean>): Promise<Unlisten> {
  if (!isTauri) return () => {}
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  const win = getCurrentWindow()
  return win.onCloseRequested(async (event) => {
    event.preventDefault()
    if (await shouldClose()) await win.destroy()
  })
}

/** “Do you want to save the changes…?”, asked before a document with unsaved changes closes. */
export async function askSaveChanges(name: string): Promise<'save' | 'discard' | 'cancel'> {
  const answer = await showDialog({
    kind: 'warning',
    title: `Do you want to save the changes you made to “${name}”?`,
    message: 'Your changes will be lost if you don’t save them.',
    buttons: [
      { label: 'Don’t Save', value: 'discard', danger: true },
      { label: 'Cancel', value: 'cancel' },
      { label: 'Save', value: 'save', primary: true },
    ],
    cancelValue: 'cancel',
  })
  return answer as 'save' | 'discard' | 'cancel'
}

export async function setFullscreen(on: boolean): Promise<void> {
  if (!isTauri) {
    if (on) await document.documentElement.requestFullscreen?.().catch(() => {})
    else if (document.fullscreenElement) await document.exitFullscreen().catch(() => {})
    return
  }
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  await getCurrentWindow().setFullscreen(on)
}

export async function toggleFullscreen(): Promise<void> {
  if (!isTauri) return setFullscreen(!document.fullscreenElement)
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  const win = getCurrentWindow()
  await win.setFullscreen(!(await win.isFullscreen()))
}

export async function startWindowDrag(): Promise<void> {
  if (!isTauri) return
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  await getCurrentWindow().startDragging()
}

export async function toggleMaximize(): Promise<void> {
  if (!isTauri) return
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  await getCurrentWindow().toggleMaximize()
}

/** Save bytes somewhere the user picks (an exported picture). Resolves false when cancelled. */
export async function saveBytesAs(name: string, bytes: Uint8Array): Promise<boolean> {
  if (!isTauri) {
    download(name, bytes)
    return true
  }
  const ext = name.includes('.') ? name.split('.').pop()! : ''
  const { save } = await dialog()
  const path = await save({ defaultPath: name, filters: ext ? [{ name: ext.toUpperCase(), extensions: [ext] }] : [] })
  if (!path) return false
  await writeFile(path, bytes)
  return true
}

export function download(name: string, bytes: Uint8Array) {
  const url = URL.createObjectURL(new Blob([bytes as BlobPart]))
  const a = document.createElement('a')
  a.href = url
  a.download = name
  a.click()
  setTimeout(() => URL.revokeObjectURL(url), 10_000)
}
