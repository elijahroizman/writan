// Keeps the app current. A check runs shortly after launch and every few hours, and again
// when the title-bar button is opened. A new version downloads in the background. Installing
// it on Windows hands off to the installer. On macOS the app quits and the same bundle is
// opened again, so the Dock icon stays put instead of a second copy appearing beside it.

import type { Update } from '@tauri-apps/plugin-updater'
import { create } from 'zustand'
import { isLinux, isMac, isTauri } from './platform'

const FIRST_CHECK = 10_000
const CHECK_EVERY = 4 * 60 * 60_000
const RELEASES_URL = 'https://api.github.com/repos/elijahroizman/writan/releases?per_page=50'
export const RELEASES_PAGE = 'https://github.com/elijahroizman/writan/releases'

export type UpdatePhase = 'idle' | 'checking' | 'downloading' | 'ready' | 'installing' | 'current' | 'error'

/** How a Linux copy was installed, and whether Writan's APT, dnf or pacman repository can update it. */
export interface LinuxPackage {
  format: 'deb' | 'rpm' | 'pacman' | null
  repo: boolean
}

/** The package manager that installs updates for a Linux package format. */
export const packageManager = (format: LinuxPackage['format']) => (format === 'rpm' ? 'dnf' : format === 'pacman' ? 'pacman' : 'apt')

/** One GitHub release's notes. `releases` holds every release newer than this copy. */
export interface ReleaseEntry {
  version: string
  notes: string
  url: string
}

export interface ReleaseNotes {
  version: string
  name: string
  notes: string
  url: string
  publishedAt: string | null
  releases: ReleaseEntry[]
  /** The missed-version list is incomplete: the page cap was hit or a later page failed. */
  truncated: boolean
}

interface UpdateSnapshot {
  phase: UpdatePhase
  open: boolean
  version: string | null
  notes: string
  releases: ReleaseEntry[]
  /** `releases` is incomplete: the page cap was hit or a later page failed. */
  truncated: boolean
  url: string | null
  publishedAt: string | null
  error: string | null
  /** Set on Linux after the first check. */
  linux: LinuxPackage | null
}

export const useUpdate = create<UpdateSnapshot>(() => ({
  phase: 'idle',
  open: false,
  version: null,
  notes: '',
  releases: [],
  truncated: false,
  url: null,
  publishedAt: null,
  error: null,
  linux: null,
}))

let downloaded: Update | null = null
let checking = false

/**
 * Release builds on a Mac or Windows can download and install. On Linux the .deb adds Writan's
 * APT repository and the .rpm its dnf repository, and those install new versions (see `linuxPackage`). Dev and the browser can still
 * read the notes.
 */
export const updatesInstallable = isTauri && !import.meta.env.DEV && !isLinux && !import.meta.env.VITE_DEV_PREVIEW

/** Dev preview builds (the dev-preview workflow) don't check: a release would replace the preview. */
const checksUpdates = isTauri && !import.meta.env.DEV && !import.meta.env.VITE_DEV_PREVIEW

const NO_PACKAGE: LinuxPackage = { format: null, repo: false }
let linuxSource: Promise<LinuxPackage> | null = null
/** How this Linux copy was installed. With its repository there, the Update button can use apt or dnf. */
function linuxPackage(): Promise<LinuxPackage> {
  if (!isLinux || !checksUpdates) return Promise.resolve(NO_PACKAGE)
  linuxSource ??= import('@tauri-apps/api/core').then(({ invoke }) => invoke<LinuxPackage>('linux_package')).catch(() => NO_PACKAGE)
  return linuxSource
}

/** Orders versions (both like 0.7.2): negative, zero or positive as `a` is older, equal or newer. */
export function compareVersions(a: string, b: string): number {
  const pa = a.split(/[.-]/).map((n) => parseInt(n, 10) || 0)
  const pb = b.split(/[.-]/).map((n) => parseInt(n, 10) || 0)
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const x = pa[i] ?? 0
    const y = pb[i] ?? 0
    if (x !== y) return x > y ? 1 : -1
  }
  return 0
}

/** Whether version `a` is newer than `b` (both like 0.7.2). */
export function newer(a: string, b: string): boolean {
  return compareVersions(a, b) > 0
}

/** On Linux nothing downloads first: a newer release with the repository is ready for apt or dnf. */
let packageReady = false

interface GithubRelease {
  tag_name?: string
  name?: string
  body?: string | null
  html_url?: string
  published_at?: string | null
  draft?: boolean
  prerelease?: boolean
}

const versionFromTag = (tag: string) => (/^v\d/.test(tag) ? tag.slice(1) : tag)

// The release's version, for real releases only. Drafts never reach the API; pre-releases are
// the pacman arch/* repositories, and a tag that isn't a version isn't one either.
const realVersion = (release: GithubRelease): string | null => {
  if (release.draft || release.prerelease) return null
  const version = versionFromTag(release.tag_name ?? '')
  return /^\d/.test(version) ? version : null
}

const releaseName = (release: GithubRelease, version: string) => release.name?.trim() || `Writan ${version}`

// The release workflow ends every release's body with this pointer to CHANGELOG.md and the
// install instructions. In a list of missed versions each entry links to GitHub instead.
const stripTrailer = (body: string) =>
  body
    .split('\n')
    .filter((line) => !line.trimStart().startsWith('Earlier versions:'))
    .join('\n')
    .trim()

/** The releases list, newest first: the latest release's notes plus every release newer than this copy. */
function releaseNotesFromList(data: GithubRelease[], current: string, truncated: boolean): ReleaseNotes {
  const releases = data
    .map((release) => ({ release, version: realVersion(release) }))
    .filter((e): e is { release: GithubRelease; version: string } => e.version !== null)
    .sort((a, b) => compareVersions(b.version, a.version))
  const latest = releases[0]
  if (!latest) throw new Error("Couldn't read the release notes.")
  return {
    version: latest.version,
    name: releaseName(latest.release, latest.version),
    notes: latest.release.body ?? '',
    url: latest.release.html_url ?? RELEASES_URL,
    publishedAt: latest.release.published_at ?? null,
    releases: releases
      .filter(({ version }) => compareVersions(version, current) > 0)
      .map(({ release, version }) => ({
        version,
        notes: stripTrailer(release.body ?? ''),
        url: release.html_url ?? RELEASES_URL,
      })),
    truncated,
  }
}

/** GitHub's `rel="next"` pagination link, if there is another page. */
function nextPage(link: string | null): string | null {
  for (const part of link?.split(',') ?? []) {
    const [url, rel] = part.split(';')
    if (rel?.trim() === 'rel="next"') return url.trim().replace(/^<|>$/g, '')
  }
  return null
}

async function fetchRelease(): Promise<ReleaseNotes> {
  if (isTauri) {
    const { invoke } = await import('@tauri-apps/api/core')
    return invoke<ReleaseNotes>('release_notes')
  }
  const current = __APP_VERSION__
  const releases: GithubRelease[] = []
  // The list runs newest first but not by version: a maintenance release for an old version
  // lands on the first page, so every page is read. Ten pages of 50 is a bound, not a filter;
  // `truncated` says it was hit.
  let url: string | null = RELEASES_URL
  for (let i = 0; url && i < 10; i++) {
    let page: GithubRelease[]
    let next: string | null
    try {
      const res: Response = await fetch(url, { headers: { Accept: 'application/vnd.github+json' } })
      if (!res.ok) throw new Error('fetch')
      next = nextPage(res.headers.get('link'))
      page = (await res.json()) as GithubRelease[]
    } catch {
      // A later page can fail without losing the releases already fetched. Leaving `url` set
      // marks the list truncated.
      if (!releases.length) {
        throw new Error("Couldn't reach GitHub. Check your connection and try again.")
      }
      break
    }
    releases.push(...page)
    url = next
  }
  return releaseNotesFromList(releases, current, url !== null)
}

function remember(release: ReleaseNotes | null, update: Update | null) {
  return {
    version: update?.version || release?.version || null,
    notes: release?.notes || update?.body || '',
    releases: release?.releases ?? [],
    truncated: release?.truncated ?? false,
    url: release?.url ?? null,
    publishedAt: release?.publishedAt ?? null,
  }
}

async function findUpdate(): Promise<Update | null> {
  if (!updatesInstallable) return null
  const { check } = await import('@tauri-apps/plugin-updater')
  return check()
}

/** Read the latest release notes and, in a release build, download a newer version. */
export async function checkForUpdates(): Promise<void> {
  if (checking || downloaded || packageReady) return
  checking = true
  useUpdate.setState({ phase: 'checking', error: null })
  try {
    const [release, found] = await Promise.allSettled([fetchRelease(), findUpdate()])
    if (found.status === 'rejected') console.warn('Update check failed', found.reason)
    const update = found.status === 'fulfilled' ? found.value : null
    const info = remember(release.status === 'fulfilled' ? release.value : null, update)
    const releaseError = release.status === 'rejected' ? "Couldn't reach GitHub. Check your connection and try again." : null

    const linux = await linuxPackage()
    if (isLinux) useUpdate.setState({ linux })
    if (release.status === 'fulfilled' && newer(release.value.version, __APP_VERSION__) && linux.repo) {
      packageReady = true
      useUpdate.setState({ phase: 'ready', ...info, error: null })
      return
    }

    if (update) {
      useUpdate.setState({ phase: 'downloading', ...info, error: null })
      try {
        await update.download()
        downloaded = update
        useUpdate.setState({ phase: 'ready', ...info, error: null })
      } catch (err) {
        console.warn('Update download failed', err)
        void update.close()
        useUpdate.setState({ phase: 'error', ...info, error: "The update couldn't be downloaded." })
      }
      return
    }
    // Notes that didn't load only matter on their own when there's no updater to ask.
    const error = found.status === 'rejected' ? "Couldn't check for an update." : updatesInstallable ? null : releaseError
    if (error) useUpdate.setState({ phase: 'error', ...info, error })
    else useUpdate.setState({ phase: 'current', ...info, error: releaseError })
  } finally {
    checking = false
  }
}

export function openUpdatePanel(): void {
  useUpdate.setState({ open: true })
  const { phase } = useUpdate.getState()
  if (phase === 'idle' || phase === 'error' || phase === 'current') void checkForUpdates()
}

export function toggleUpdatePanel(): void {
  if (useUpdate.getState().open) {
    useUpdate.setState({ open: false })
    return
  }
  openUpdatePanel()
}

export function closeUpdatePanel(): void {
  useUpdate.setState({ open: false })
}

export function startUpdateChecks(): () => void {
  if (!checksUpdates) return () => {}
  const first = window.setTimeout(() => void checkForUpdates(), FIRST_CHECK)
  const timer = window.setInterval(() => void checkForUpdates(), CHECK_EVERY)
  return () => {
    window.clearTimeout(first)
    window.clearInterval(timer)
  }
}

/**
 * Install a downloaded update. With `restart`, the new version opens right away.
 * On Windows the installer takes over and this process exits. On macOS we quit
 * first and open the same bundle once this process is gone.
 */
export async function installUpdate({ restart }: { restart: boolean }): Promise<void> {
  // apt and dnf ask for a password, so on Linux they only run when the button is pressed, not on quit.
  if (packageReady && restart) return installPackage()
  if (!downloaded) return
  const update = downloaded
  useUpdate.setState({ phase: 'installing', error: null })
  let installed = false
  try {
    if (isMac) {
      // `restartAfterInstall` is a Windows installer flag. macOS only swaps the
      // bundle; relaunching here with the plugin would run `open -n`.
      await update.install()
      installed = true
      downloaded = null
      if (restart) {
        const { invoke } = await import('@tauri-apps/api/core')
        await invoke('relaunch_in_place')
      }
      return
    }
    await update.install({ restartAfterInstall: restart })
    installed = true
    downloaded = null
    if (restart) {
      const { relaunch } = await import('@tauri-apps/plugin-process')
      await relaunch()
    }
  } catch (err) {
    console.warn('Update install failed', err)
    downloaded = null
    if (!installed) void update.close()
    useUpdate.setState({
      phase: 'error',
      error: installed ? 'The update is installed. Quit and reopen Writan to use it.' : "The update couldn't be installed.",
    })
  }
}

async function installPackage(): Promise<void> {
  useUpdate.setState({ phase: 'installing', error: null })
  const { invoke } = await import('@tauri-apps/api/core')
  try {
    await invoke('package_upgrade', { version: useUpdate.getState().version })
  } catch (err) {
    console.warn('Package upgrade failed', err)
    useUpdate.setState({ phase: 'ready', error: String(err) })
    return
  }
  packageReady = false
  const { relaunch } = await import('@tauri-apps/plugin-process')
  await relaunch()
}
