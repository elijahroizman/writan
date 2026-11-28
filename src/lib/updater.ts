// Keeps the app current. A check runs shortly after launch and every few hours, again when the
// title-bar button is opened, and when the update channel changes (src/state/updateChannel.ts).
// A new version downloads in the background. Installing it on Windows hands off to the
// installer. On macOS the app quits and the same bundle is opened again, so the Dock icon stays
// put instead of a second copy appearing beside it.

import type { Update } from '@tauri-apps/plugin-updater'
import { create } from 'zustand'
import { getUpdateChannel, onUpdateChannelChange, type UpdateChannel } from '../state/updateChannel'
import { isLinux, isMac, isTauri } from './platform'
import { compareVersions, isNightlyVersion, newer } from './versions'

export { compareVersions, newer } from './versions'

const FIRST_CHECK = 10_000
const CHECK_EVERY = 4 * 60 * 60_000
const FEED_RETRY_DELAYS = [5_000, 30_000, 2 * 60_000]
const RELEASES_URL = 'https://api.github.com/repos/elijahroizman/writan/releases?per_page=50'
export const RELEASES_PAGE = 'https://github.com/elijahroizman/writan/releases'

export type UpdatePhase = 'idle' | 'checking' | 'downloading' | 'ready' | 'installing' | 'current' | 'error'

/**
 * How a Linux copy was installed: `manager` says apt, dnf or pacman is there, which is enough for a
 * nightly (it installs from its GitHub release), and `repo` says Writan's repository is there and on
 * as well, which a stable release comes from.
 */
export interface LinuxPackage {
  format: 'deb' | 'rpm' | 'pacman' | null
  manager: boolean
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
/** Goes up when the channel changes, so a check that started on the other channel is dropped. */
let generation = 0

/**
 * Release builds on a Mac or Windows can download and install. On Linux apt, dnf or pacman install
 * new versions (see `linuxPackage`). Dev and the browser can still read the notes.
 */
export const updatesInstallable = isTauri && !import.meta.env.DEV && !isLinux

const checksUpdates = isTauri && !import.meta.env.DEV

const NO_PACKAGE: LinuxPackage = { format: null, manager: false, repo: false }
let linuxSource: Promise<LinuxPackage> | null = null
/** How this Linux copy was installed. With its repository there, the Update button can use apt or dnf. */
function linuxPackage(): Promise<LinuxPackage> {
  if (!isLinux || !checksUpdates) return Promise.resolve(NO_PACKAGE)
  linuxSource ??= import('@tauri-apps/api/core').then(({ invoke }) => invoke<LinuxPackage>('linux_package')).catch(() => NO_PACKAGE)
  return linuxSource
}

/** Whether apt, dnf or pacman can install `version`: a nightly from its release, a stable release from Writan's repository. */
const linuxCanInstall = (linux: LinuxPackage, version: string) => (isNightlyVersion(version) ? linux.manager : linux.repo)

/** On Linux nothing downloads first: a newer release that apt, dnf or pacman can install is ready. */
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

// The release's version, for the releases `channel` follows. Drafts never reach the API. The
// other pre-releases are the APT, dnf and pacman repositories, the nightly feed, plugins and old
// dev builds, whose tags aren't versions, and the nightlies, which only the nightly channel lists.
const realVersion = (release: GithubRelease, channel: UpdateChannel): string | null => {
  if (release.draft) return null
  const version = versionFromTag(release.tag_name ?? '')
  if (!/^\d/.test(version)) return null
  const nightly = isNightlyVersion(version)
  return (release.prerelease ? nightly && channel === 'nightly' : !nightly) ? version : null
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
function releaseNotesFromList(data: GithubRelease[], current: string, truncated: boolean, channel: UpdateChannel): ReleaseNotes {
  const releases = data
    .map((release) => ({ release, version: realVersion(release, channel) }))
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

async function fetchRelease(channel: UpdateChannel): Promise<ReleaseNotes> {
  if (isTauri) {
    const { invoke } = await import('@tauri-apps/api/core')
    return invoke<ReleaseNotes>('release_notes', { channel })
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
  return releaseNotesFromList(releases, current, url !== null, channel)
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

/**
 * Asks the channel's update feed for a newer version. The plugin's own `check()` only reads the feed
 * in tauri.conf.json, so `check_update` (src-tauri/src/update.rs) builds the updater with the
 * nightly feed when that's the channel, and hands back the plugin's `Update` to download and install.
 */
async function findUpdate(channel: UpdateChannel): Promise<Update | null> {
  if (!updatesInstallable) return null
  const [{ invoke }, { Update }] = await Promise.all([import('@tauri-apps/api/core'), import('@tauri-apps/plugin-updater')])
  const metadata = await invoke<ConstructorParameters<typeof Update>[0] | null>('check_update', { channel, timeout: 15_000 })
  return metadata ? new Update(metadata) : null
}

/** Read the latest release notes and, in a release build, download a newer version. */
export async function checkForUpdates(): Promise<void> {
  if (checking || downloaded || packageReady) return
  checking = true
  const run = generation
  const stale = () => run !== generation
  const channel = getUpdateChannel()
  useUpdate.setState({ phase: 'checking', error: null })
  try {
    const [release, found] = await Promise.allSettled([fetchRelease(channel), findUpdate(channel)])
    if (found.status === 'rejected') console.warn('Update check failed', found.reason)
    const update = found.status === 'fulfilled' ? found.value : null
    if (stale()) {
      void update?.close()
      return
    }
    const info = remember(release.status === 'fulfilled' ? release.value : null, update)
    const releaseError = release.status === 'rejected' ? "Couldn't reach GitHub. Check your connection and try again." : null

    const linux = await linuxPackage()
    if (isLinux) useUpdate.setState({ linux })
    if (release.status === 'fulfilled' && newer(release.value.version, __APP_VERSION__) && linuxCanInstall(linux, release.value.version)) {
      packageReady = true
      useUpdate.setState({ phase: 'ready', ...info, error: null })
      return
    }

    if (update) {
      useUpdate.setState({ phase: 'downloading', ...info, error: null })
      try {
        // Tauri would otherwise reuse the short manifest timeout for the whole bundle.
        await update.download(undefined, { timeout: 15 * 60_000 })
        if (stale()) {
          void update.close()
          return
        }
        downloaded = update
        useUpdate.setState({ phase: 'ready', ...info, error: null })
      } catch (err) {
        console.warn('Update download failed', err)
        void update.close()
        if (!stale()) useUpdate.setState({ phase: 'error', ...info, error: "The update couldn't be downloaded." })
      }
      return
    }
    // Notes that didn't load only matter on their own when there's no updater to ask.
    const error = found.status === 'rejected' ? "Couldn't check for an update." : updatesInstallable ? null : releaseError
    if (error) useUpdate.setState({ phase: 'error', ...info, error })
    else useUpdate.setState({ phase: 'current', ...info, error: releaseError })
  } finally {
    checking = false
    // The channel changed during this check: check the new one.
    if (stale()) void checkForUpdates()
  }
}

// A new channel drops what the old one found: a downloaded version waiting to install on quit, or a
// Linux package ready for apt, dnf or pacman. Then it checks the new channel. Moving to stable from a
// nightly finds nothing until a stable release newer than this copy is out, since only a newer
// version installs.
onUpdateChannelChange(() => {
  // An install already running finishes; the next scheduled check reads the new channel.
  if (useUpdate.getState().phase === 'installing') return
  generation++
  if (downloaded) {
    void downloaded.close()
    downloaded = null
  }
  packageReady = false
  if (!checking) useUpdate.setState({ phase: 'idle', error: null })
  if (checksUpdates || useUpdate.getState().open) void checkForUpdates()
})

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
  let attempts = 0
  let retry: number | undefined
  const unsubscribe = useUpdate.subscribe(({ phase }, prev) => {
    if (phase === prev.phase || !updatesInstallable) return
    window.clearTimeout(retry)
    if (phase === 'error' && prev.phase === 'checking') {
      const delay = FEED_RETRY_DELAYS[attempts++]
      if (delay !== undefined) retry = window.setTimeout(() => void checkForUpdates(), delay)
    } else if (phase !== 'checking') {
      attempts = 0
    }
  })
  const scheduledCheck = () => {
    attempts = 0
    void checkForUpdates()
  }
  const first = window.setTimeout(scheduledCheck, FIRST_CHECK)
  const timer = window.setInterval(scheduledCheck, CHECK_EVERY)
  return () => {
    unsubscribe()
    window.clearTimeout(retry)
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
