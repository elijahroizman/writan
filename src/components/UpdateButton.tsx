import { CircleArrowUp } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { isLinux, isMac, openExternal } from '../lib/platform'
import { checkForUpdates, closeUpdatePanel, newer, packageManager, RELEASES_PAGE, toggleUpdatePanel, updatesInstallable, useUpdate, type LinuxPackage, type ReleaseEntry, type UpdatePhase } from '../lib/updater'
import { displayVersion, isNightlyVersion } from '../lib/versions'
import { useUpdateChannel, type UpdateChannel } from '../state/updateChannel'
import { runCommand } from '../App'
import { Spinner } from './ui'

function buttonLabel(phase: UpdatePhase): string {
  switch (phase) {
    case 'checking':
      return 'Checking…'
    case 'downloading':
      return 'Downloading…'
    case 'installing':
      return 'Updating…'
    case 'ready':
      return 'Update'
    default:
      return 'Check for updates'
  }
}

/** The check finished and found nothing newer than this copy. */
const onLatest = (phase: UpdatePhase, version: string | null) => phase === 'current' && !(version && newer(version, __APP_VERSION__))

function heading(phase: UpdatePhase, version: string | null): string {
  if (phase === 'error' && !version) return "Couldn't check for updates"
  if (phase === 'checking' && !version) return 'Checking for updates'
  if (onLatest(phase, version)) return "You're on the latest version"
  return version ? `Writan ${displayVersion(version)}` : 'Updates'
}

function detail(phase: UpdatePhase, error: string | null, publishedAt: string | null, version: string | null, linux: LinuxPackage | null, channel: UpdateChannel): string {
  const manager = packageManager(linux?.format ?? null)
  const date = formatDate(publishedAt)
  if (phase === 'error') return error ?? "Couldn't check for updates."
  if (phase === 'checking') return 'Looking at the latest GitHub release'
  if (phase === 'downloading') return 'Downloading the update'
  if (phase === 'installing') return isMac ? 'Writan will quit and reopen' : isLinux ? `Installing with ${manager}, then Writan reopens` : 'The installer will restart Writan'
  if (phase === 'ready' && error) return error
  if (phase === 'ready' && isLinux) return `Installs with ${manager} after asking for your password`
  if (phase === 'ready') return date ? `Ready to install · ${date}` : 'Ready to install'
  if (isLinux && version && newer(version, __APP_VERSION__)) {
    const shown = displayVersion(version)
    return linux?.format && !isNightlyVersion(version)
      ? `Writan ${shown} is out. Install the new .${linux.format === 'pacman' ? 'pkg.tar.zst' : linux.format} from GitHub, and later versions come through ${manager}.`
      : `Writan ${shown} is out. Download it from GitHub.`
  }
  // Only a newer version installs, so a nightly stays put until a stable release passes it.
  if (channel === 'stable' && isNightlyVersion(__APP_VERSION__)) {
    return 'This copy is a nightly. Writan moves to stable at the next stable release newer than it.'
  }
  if (onLatest(phase, version)) return `Writan ${displayVersion(__APP_VERSION__)}`
  if (!updatesInstallable) return date ? `Latest GitHub release · ${date}` : 'Latest GitHub release'
  return date ? `This is the newest release · ${date}` : 'This is the newest release'
}

function formatDate(iso: string | null): string | null {
  if (!iso) return null
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return null
  return date.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })
}

let cachedMd: { render(src: string): string } | null = null
let cachedSanitize: ((html: string) => string) | null = null
async function renderNotes(markdown: string): Promise<string> {
  if (!cachedMd || !cachedSanitize) {
    const [{ default: MarkdownIt }, { default: DOMPurify }] = await Promise.all([import('markdown-it'), import('dompurify')])
    cachedMd ??= new MarkdownIt({ html: false, linkify: true, breaks: true })
    cachedSanitize ??= DOMPurify.sanitize.bind(DOMPurify)
  }
  return cachedSanitize(cachedMd.render(markdown))
}

export function UpdateButton() {
  const phase = useUpdate((s) => s.phase)
  const open = useUpdate((s) => s.open)
  const version = useUpdate((s) => s.version)
  const notes = useUpdate((s) => s.notes)
  const releases = useUpdate((s) => s.releases)
  const truncated = useUpdate((s) => s.truncated)
  const url = useUpdate((s) => s.url)
  const publishedAt = useUpdate((s) => s.publishedAt)
  const error = useUpdate((s) => s.error)
  const linux = useUpdate((s) => s.linux)
  const channel = useUpdateChannel()
  const button = useRef<HTMLButtonElement>(null)
  const panel = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ x: 0, y: 0, maxHeight: 420 })
  const [html, setHtml] = useState('')
  // The rendered notes kept with the release list they came from, so a refresh can't show new
  // version headings over the previous list's notes.
  const [sections, setSections] = useState<{ releases: ReleaseEntry[]; html: string[] } | null>(null)

  const busy = phase === 'checking' || phase === 'downloading' || phase === 'installing'
  // Up to date: say so, and leave the notes one click away. The link is for this version's own
  // notes, so a nightly waiting for the next stable release, whose latest release is older, has none.
  const latest = onLatest(phase, version)
  const ownNotes = !!version && version === __APP_VERSION__
  const label = buttonLabel(phase)

  useEffect(() => {
    if (!open) return
    if (!notes.trim()) {
      setHtml('')
      return
    }
    let cancel = false
    void renderNotes(notes).then((rendered) => {
      if (!cancel) setHtml(rendered)
    })
    return () => {
      cancel = true
    }
  }, [open, notes])

  useEffect(() => {
    if (!open) return
    if (!releases.length) {
      setSections(null)
      return
    }
    let cancel = false
    void Promise.all(releases.map((release) => renderNotes(release.notes))).then((rendered) => {
      if (!cancel) setSections({ releases, html: rendered })
    })
    return () => {
      cancel = true
    }
  }, [open, releases])

  useLayoutEffect(() => {
    if (!open) return
    const place = () => {
      const rect = button.current?.getBoundingClientRect()
      const el = panel.current
      if (!rect || !el) return
      const width = el.offsetWidth
      const y = rect.bottom + 8
      setPos({
        x: Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8)),
        y,
        maxHeight: Math.max(240, window.innerHeight - y - 8),
      })
    }
    place()
    window.addEventListener('resize', place)
    return () => window.removeEventListener('resize', place)
  }, [open, html, sections, phase])

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') closeUpdatePanel()
    }
    // A press anywhere else closes the panel. The button toggles it itself.
    const onDown = (e: PointerEvent) => {
      const target = e.target as Node
      if (!panel.current?.contains(target) && !button.current?.contains(target)) closeUpdatePanel()
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('pointerdown', onDown, true)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('pointerdown', onDown, true)
    }
  }, [open])

  const primary = () => {
    if (phase === 'ready') {
      void runCommand('install-update')
      return
    }
    void checkForUpdates()
  }

  const action = phase === 'error' ? 'Try again' : busy || phase === 'ready' ? label : 'Check again'
  const title = phase === 'ready' && version ? `Writan ${displayVersion(version)} is ready to install` : busy ? label : 'Check for updates'

  return (
    <>
      <button
        ref={button}
        type="button"
        className={`update-btn ${phase === 'ready' ? 'ready' : ''}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={title}
        title={title}
        onClick={() => toggleUpdatePanel()}
      >
        {busy ? <Spinner size={14} /> : <CircleArrowUp size={14} />}
      </button>
      {open &&
        createPortal(
          <div
            ref={panel}
            className="update-panel"
            role="dialog"
            aria-labelledby="update-title"
            style={{ left: pos.x, top: pos.y, maxHeight: pos.maxHeight }}
          >
            <header className="update-panel-head">
              <h2 id="update-title">{heading(phase, version)}</h2>
              <p>{detail(phase, error, publishedAt, version, linux, channel)}</p>
            </header>
            {!latest && (
              <div
                className="update-notes"
                onClick={(e) => {
                  const link = (e.target as HTMLElement).closest('a')
                  const href = link?.getAttribute('href')
                  if (!link || !href || href.startsWith('#')) return
                  e.preventDefault()
                  void openExternal(href)
                }}
              >
                {phase === 'checking' && !html && !sections ? (
                  <p className="update-notes-empty">Fetching the release notes…</p>
                ) : sections ? (
                  <>
                    {sections.releases.map((release, i) => (
                      <section key={release.version} className="update-release">
                        <h3>Changes in {displayVersion(release.version)}</h3>
                        <div dangerouslySetInnerHTML={{ __html: sections.html[i] ?? '' }} />
                        <p className="update-release-link">
                          <a href={release.url}>View on GitHub</a>
                        </p>
                      </section>
                    ))}
                    {truncated && (
                      <p className="update-release-link">
                        <a href={RELEASES_PAGE}>Earlier versions on GitHub</a>
                      </p>
                    )}
                  </>
                ) : html ? (
                  <div dangerouslySetInnerHTML={{ __html: html }} />
                ) : (
                  <p className="update-notes-empty">{error ? 'The release notes couldn’t be loaded.' : 'This release has no notes.'}</p>
                )}
              </div>
            )}
            <footer className="update-panel-foot">
              {url && !sections && (!latest || ownNotes) && (
                <button type="button" className="update-link" onClick={() => void openExternal(url)}>
                  {latest ? 'What’s new in this version' : 'View on GitHub'}
                </button>
              )}
              <button type="button" className="update-go" disabled={busy} onClick={primary}>
                {action}
              </button>
            </footer>
          </div>,
          document.body,
        )}
    </>
  )
}
