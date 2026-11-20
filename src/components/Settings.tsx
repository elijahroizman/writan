import { ArrowLeft, Blocks, Keyboard, Monitor, Moon, Palette, Pencil, Plus, RotateCcw, Save, Search, Settings2, Sun, Trash2, X } from 'lucide-react'
import type { ReactNode } from 'react'
import { lazy, Suspense, useEffect, useRef, useState } from 'react'
import { canSetDefaults, defaultStatus, makeDefault, openDefaultAppsSettings } from '../lib/defaults'
import type { CliStatus } from '../lib/platform'
import { clearAllHistory, cliStatus, confirmDialog, historyUsage, isLinux, isMac, isTauri, isWindows, openExternal, setCliInstalled, shortcut, showError } from '../lib/platform'
import type { ExtraFont } from '../state/fonts'
import { EXTRA_FONTS, useExtraFonts } from '../state/fonts'
import type { Appearance, ThemeDef } from '../lib/themes'
import { isHex, orbBackground } from '../lib/themes'
import type { LinuxPackage } from '../lib/updater'
import { linuxPackage, openUpdatePanel, packageManager, updatesInstallable } from '../lib/updater'
import { PLUGINS, isBuiltIn } from '../plugins/catalog'
import { allThemes, useAppearance } from '../state/appearance'
import { enabledKinds, formatSize, usePlugins } from '../state/plugins'
import type { SettingsPage } from '../state/settings'
import { PREVIEW_DELAYS, RECENT_DAYS, useSettings } from '../state/settings'
import { INSTALL_CHOICES, useTex } from '../state/tex'
import { useSetup } from '../state/setup'
import type { ShortcutDef } from '../state/shortcuts'
import { SHORTCUTS, isValidKeys, keysFor, keysFromEvent, syncMenu, useShortcuts } from '../state/shortcuts'
import { useFolder } from '../state/folder'
import { useWorkspace } from '../state/workspace'
import { PluginRow } from './PluginList'
import { formatSize as formatVersionSize } from './VersionHistory'
import { Button, Checkbox, ChoiceTiles, DocBadge, IconButton, LinkButton, Logo, Modal, Segmented, SettingsRow, Spinner, Switch, TextField } from './ui'

const LicensesPage = lazy(() => import('./Licenses'))

const DELAYS = [
  { value: '1', label: '1 sec' },
  { value: '3', label: '3 sec' },
  { value: '10', label: '10 sec' },
  { value: '30', label: '30 sec' },
]

const PAGES: { id: SettingsPage; label: string; icon: ReactNode }[] = [
  { id: 'general', label: 'General', icon: <Settings2 size={15} strokeWidth={1.75} /> },
  { id: 'plugins', label: 'Plugins', icon: <Blocks size={15} strokeWidth={1.75} /> },
  { id: 'autosave', label: 'Autosave', icon: <Save size={15} strokeWidth={1.75} /> },
  { id: 'appearance', label: 'Appearance', icon: <Palette size={15} strokeWidth={1.75} /> },
  { id: 'shortcuts', label: 'Keyboard Shortcuts', icon: <Keyboard size={15} strokeWidth={1.75} /> },
]

/** Set while a shortcut is being recorded, so esc cancels that instead of closing Settings. */
let recording = false

export default function SettingsPanel() {
  const setOpen = useSettings((s) => s.setOpen)
  const page = useSettings((s) => s.page)
  const setPage = useSettings((s) => s.setPage)
  const dialog = useRef<HTMLDivElement>(null)

  useEffect(() => {
    dialog.current?.focus()
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !recording) {
        e.stopPropagation()
        setOpen(false)
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [setOpen])

  return (
    <Modal placement="center" dim="normal" layer="sheet" onClose={() => setOpen(false)}>
      <div className="settings" role="dialog" aria-modal="true" aria-labelledby="settings-title" tabIndex={-1} ref={dialog}>
        <nav className="settings-nav" aria-label="Settings sections">
          <h2 id="settings-title">Settings</h2>
          {PAGES.map((p) => (
            <button
              key={p.id}
              type="button"
              className={`settings-nav-item ${page === p.id || (page === 'licenses' && p.id === 'general') ? 'on' : ''}`}
              aria-current={page === p.id}
              onClick={() => setPage(p.id)}
            >
              {p.icon}
              {p.label}
            </button>
          ))}
        </nav>
        <div className="settings-page">
          <header className="settings-page-header">
            {page === 'licenses' ? (
              <button type="button" className="settings-back" onClick={() => setPage('general')}>
                <ArrowLeft size={15} />
                <h3>Open-source licenses</h3>
              </button>
            ) : (
              <h3>{PAGES.find((p) => p.id === page)!.label}</h3>
            )}
            <IconButton label="Close (Esc)" onClick={() => setOpen(false)}>
              <X size={16} />
            </IconButton>
          </header>
          <div className="settings-scroll" key={page}>
            {page === 'general' && <GeneralPage />}
            {page === 'plugins' && <PluginsPage />}
            {page === 'autosave' && <AutosavePage />}
            {page === 'appearance' && <AppearancePage />}
            {page === 'shortcuts' && <ShortcutsPage />}
            {page === 'licenses' && (
              <Suspense fallback={<Spinner />}>
                <LicensesPage />
              </Suspense>
            )}
          </div>
        </div>
      </div>
    </Modal>
  )
}

/** How new versions arrive: by themselves, through the system's package manager, or by hand. */
function updateHint(linux: LinuxPackage | null): string {
  if (!isLinux) return updatesInstallable ? 'Updates download by themselves and install when you quit.' : 'Check for updates shows the latest release.'
  if (linux?.format && linux.repo) return `New versions come through ${packageManager(linux.format)} with your system’s other updates.`
  return 'Check for updates tells you when a new version is out.'
}

function GeneralPage() {
  const setOpen = useSettings((s) => s.setOpen)
  const setPage = useSettings((s) => s.setPage)
  const [linux, setLinux] = useState<LinuxPackage | null>(null)
  useEffect(() => void linuxPackage().then(setLinux), [])
  return (
    <>
      <SettingsRow label={`Writan ${__APP_VERSION__}`} hint={updateHint(linux)}>
        <Button onClick={() => (setOpen(false), openUpdatePanel())}>Check for updates</Button>
      </SettingsRow>

      <DefaultAppsRow />

      <RecentFilesRow />

      <CommandLineRow />

      <SettingsRow
        label="Welcome tour"
        hint={canSetDefaults ? 'The first-launch tour of the app, with extensions and default apps at the end.' : 'The first-launch tour of the app, with its extensions.'}
      >
        <Button onClick={() => (setOpen(false), useSetup.getState().show())}>Show the tour again</Button>
      </SettingsRow>

      <SettingsRow label="Open-source licenses" hint="Writan is built on open-source engines, including PDFium, CodeMirror and the Carlito, Caladea and Liberation fonts.">
        <Button onClick={() => setPage('licenses')}>View licenses</Button>
      </SettingsRow>
    </>
  )
}

/** The `writan` shell command: installed from here on a Mac, already there on Linux. */
function CommandLineRow() {
  const [status, setStatus] = useState<CliStatus | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    void cliStatus().then(setStatus)
  }, [])
  if (!status || (!status.supported && !status.installed)) return null
  const toggle = async () => {
    setBusy(true)
    try {
      await setCliInstalled(!status.installed)
    } catch (err) {
      await showError(String(err), status.installed ? 'Couldn’t remove the command' : 'Couldn’t install the command')
    } finally {
      setStatus(await cliStatus())
      setBusy(false)
    }
  }
  return (
    <SettingsRow
      label="Command line"
      hint={
        status.taken
          ? `Something else is already at ${status.path}, so Writan can’t put its command there.`
          : status.supported
            ? `Open files and folders from Terminal: writan . opens the folder you’re in, writan notes.md a file. ${status.installed ? `Installed at ${status.path}.` : 'Installing asks for your password.'}`
            : 'Open files and folders from a terminal: writan . opens the folder you’re in, writan notes.md a file.'
      }
    >
      {status.supported && !status.taken && (
        <Button disabled={busy} onClick={() => void toggle()}>
          {status.installed ? 'Remove command' : 'Install command'}
        </Button>
      )}
    </SettingsRow>
  )
}

/** Home's Recent list: how long things stay, which file types get a tab, and clearing it. */
function RecentFilesRow() {
  const recentDays = useSettings((s) => s.recentDays)
  const setRecentDays = useSettings((s) => s.setRecentDays)
  const files = useWorkspace((s) => s.recents.length)
  const folders = useFolder((s) => s.recent.length)
  const count = files + folders
  return (
    <SettingsRow column bare label="Recent files and folders" hint="Home lists the files and folders you open, newest first.">
      <div className="settings-inline">
        <span className="settings-hint">Keep them in Recent for</span>
        <Segmented value={String(recentDays)} options={RECENT_DAYS.map((d) => ({ value: String(d), label: d === 1 ? '1 day' : `${d} days` }))} onChange={(v) => setRecentDays(Number(v))} />
      </div>
      <div className="settings-inline">
        <Button disabled={!count} onClick={() => useWorkspace.getState().clearRecents()}>
          Clear file history
        </Button>
        <span className="settings-hint">{count ? `Forgets the ${count === 1 ? (files ? 'file' : 'folder') : `${count} ${folders ? 'files and folders' : 'files'}`} on Home. The files themselves stay where they are.` : 'There’s nothing in the history.'}</span>
      </div>
    </SettingsRow>
  )
}

/** Which file types open in Writan, and a button for each that doesn't. */
function DefaultAppsRow() {
  const plugins = usePlugins((s) => s.enabled)
  const kinds = enabledKinds(plugins)
  const [status, setStatus] = useState<Record<string, boolean | null>>({})
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const requestId = useRef(0)
  const loaded = useRef(false)
  const key = kinds.map((k) => k.id).join()

  const refresh = () => {
    const id = ++requestId.current
    if (!loaded.current) setLoading(true)
    void defaultStatus(kinds.map((k) => k.id)).then(
      (next) => {
        if (requestId.current !== id) return
        loaded.current = true
        setStatus(next)
        setError(null)
        setLoading(false)
      },
      (err) => {
        if (requestId.current !== id) return
        loaded.current = true
        setError(err instanceof Error ? err.message : String(err))
        setLoading(false)
      },
    )
  }

  useEffect(() => {
    if (!canSetDefaults) return
    refresh()
    // Windows: the user picks in Settings, then comes back.
    window.addEventListener('focus', refresh)
    return () => {
      requestId.current++
      window.removeEventListener('focus', refresh)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key])

  const set = async (ids: string[]) => {
    setBusy(true)
    try {
      await makeDefault(ids)
    } catch (err) {
      await showError(String(err))
    } finally {
      setBusy(false)
      refresh()
    }
  }

  const openWindowsSettings = async () => {
    setBusy(true)
    try {
      await openDefaultAppsSettings()
    } catch (err) {
      await showError(String(err))
    } finally {
      setBusy(false)
      refresh()
    }
  }

  if (!canSetDefaults) return null
  const missing = kinds.filter((k) => status[k.id] === false).map((k) => k.id)
  const hasUnknown = !loading && kinds.some((k) => status[k.id] == null)
  const detectionFailed = !loading && (error !== null || hasUnknown)
  return (
    <SettingsRow
      column
      bare
      label="Default apps"
      hint={isMac ? 'Double-clicking these files in Finder opens them in Writan.' : 'Windows asks you to confirm each one in Settings › Default apps.'}
    >
      {loading && (
        <p className="settings-hint" role="status">
          Checking default apps…
        </p>
      )}
      {!loading && error && (
        <p className="settings-hint" role="alert">
          Couldn’t check current default apps ({error}).{isWindows ? ' You can still open Windows Default Apps to choose Writan.' : ''}
        </p>
      )}
      {!loading && !error && hasUnknown && (
        <p className="settings-hint" role="status">
          Couldn’t detect the current default app for some file types.{isWindows ? ' You can still open Windows Default Apps to choose Writan.' : ''}
        </p>
      )}
      <div className="default-apps">
        {kinds.map((k) => (
          <div key={k.id} className="default-app">
            <DocBadge kind={k.id} size={22} />
            <span className="default-app-name">
              {k.label} <span className="faint">{k.extensions.map((e) => `.${e}`).join(' ')}</span>
            </span>
            {status[k.id] === true ? (
              <span className="faint">Opens in Writan</span>
            ) : (
              <Button disabled={busy || loading} onClick={() => void set([k.id])}>
                Make default
              </Button>
            )}
          </div>
        ))}
      </div>
      {isWindows && detectionFailed ? (
        <div className="settings-inline">
          <Button disabled={busy} onClick={() => void openWindowsSettings()}>
            Open Windows Default Apps
          </Button>
        </div>
      ) : (
        missing.length > 1 && (
          <div>
            <Button disabled={busy} onClick={() => void set(missing)}>
              Make Writan the default for all
            </Button>
          </div>
        )
      )}
    </SettingsRow>
  )
}

/** Everything optional in one place: plugins for more file types, and fonts to download. */
function PluginsPage() {
  const enabled = usePlugins((s) => s.enabled)
  const progress = usePlugins((s) => s.progress)
  const fonts = useExtraFonts((s) => s.installed)
  const { enable, disable } = usePlugins.getState()
  const optional = PLUGINS.filter((p) => !isBuiltIn(p))
  const count = (on: number, all: number) => <span className="plugin-count">{`${on} of ${all} on`}</span>
  return (
    <>
      <p className="settings-intro">Turn on only what you need. Word, PowerPoint, PDF, Markdown, text and image files always open.</p>
      <section className="plugin-group">
        <h4>
          File types {count(optional.filter((p) => enabled[p.id]).length, optional.length)}
        </h4>
        {optional.map((p) => (
          <PluginRow
            key={p.id}
            plugin={p}
            control={
              progress[p.id] ? (
                <Spinner size={16} />
              ) : (
                <Switch
                  label={`Use ${p.name}`}
                  checked={!!enabled[p.id]}
                  onChange={(on) => void (on ? enable(p.id) : disablePlugin(p.id, disable))}
                />
              )
            }
          >
            <LinkButton title={p.homepage} onClick={() => void openExternal(p.homepage)}>
              {linkText(p.homepage)}
            </LinkButton>
            {p.services?.includes('tex') && enabled[p.id] && <TexSettings />}
          </PluginRow>
        ))}
      </section>
      <section className="plugin-group">
        <h4>
          Fonts {count(EXTRA_FONTS.filter((f) => fonts[f.id]).length, EXTRA_FONTS.length)}
        </h4>
        <p className="settings-hint plugin-group-hint">
          {isTauri ? 'Downloaded when you turn one on, then in the font menus for Word and PowerPoint.' : 'Fonts download in the app, not in the browser version.'}
        </p>
        {EXTRA_FONTS.map((f) => (
          <FontRow key={f.id} font={f} />
        ))}
      </section>
    </>
  )
}

/** A font Writan downloads from its maker (state/fonts.ts): on downloads it, off removes it. */
function FontRow({ font }: { font: ExtraFont }) {
  const installed = useExtraFonts((s) => !!s.installed[font.id])
  const busy = useExtraFonts((s) => !!s.busy[font.id])
  const error = useExtraFonts((s) => s.errors[font.id])
  const { install, remove } = useExtraFonts.getState()
  // Shown in the font itself once it's there.
  const face = installed ? `'${font.family}', ${font.fallback}` : undefined
  return (
    <div className="plugin-row">
      <span className="font-badge" style={{ fontFamily: face }} aria-hidden="true">
        Aa
      </span>
      <span className="plugin-text">
        <span className="settings-label" style={{ fontFamily: face }}>
          {font.family}
        </span>
        <span className="settings-hint">{font.about}</span>
        <span className="settings-hint plugin-meta">
          {font.maker} · {formatSize(font.size)} download ·{' '}
          <LinkButton onClick={() => void openExternal(font.licenseUrl)}>License</LinkButton>
        </span>
        {error && <span className="plugin-error">{error}</span>}
      </span>
      <span className="plugin-control">
        {busy ? (
          <Spinner size={16} />
        ) : (
          <Switch label={`Use the ${font.family} font`} checked={installed} disabled={!isTauri} onChange={(on) => void (on ? install(font.id) : remove(font.id))} />
        )}
      </span>
    </div>
  )
}

/** Under LaTeX: which TeX compiles, and how soon after typing stops. */
function TexSettings() {
  const status = useTex((s) => s.status)
  const installing = useTex((s) => s.installing)
  const previewDelay = useSettings((s) => s.previewDelay)
  const setPreviewDelay = useSettings((s) => s.setPreviewDelay)
  useEffect(() => void useTex.getState().refresh(), [])
  return (
    <div className="plugin-extra">
      <div className="plugin-extra-row">
        <span className="settings-hint">
          {installing
            ? 'Installing TeX…'
            : !status
            ? 'Looking for TeX…'
            : status.installed
              ? `Compiles with ${status.distribution ?? 'TeX'} (${status.engines.join(', ')}).`
              : 'TeX isn’t installed, so documents can’t compile.'}
        </span>
        {installing ? (
          <Button onClick={() => useTex.setState({ offer: true })}>Show progress</Button>
        ) : (
          status &&
          !status.installed &&
          INSTALL_CHOICES.length > 0 && (
            <Button onClick={() => void useTex.getState().offerInstall()}>Install TeX…</Button>
          )
        )}
      </div>
      <div className="plugin-extra-row">
        <span className="settings-hint">Update the PDF after you stop typing for</span>
        <Segmented
          value={String(previewDelay)}
          options={PREVIEW_DELAYS.map((d) => ({ value: String(d), label: `${d} sec` }))}
          onChange={(v) => setPreviewDelay(Number(v))}
        />
      </div>
    </div>
  )
}

/** A plugin's homepage as a short link: its site's name, or “Source on GitHub”. */
function linkText(url: string) {
  const host = new URL(url).hostname.replace(/^www\./, '')
  return host === 'github.com' ? 'Source on GitHub' : host
}

/** Tabs of a plugin's files have to close first. */
async function disablePlugin(id: string, disable: (id: string) => Promise<void>) {
  const plugin = PLUGINS.find((p) => p.id === id)!
  const kinds = new Set(plugin.kinds.map((k) => k.id))
  const open = useWorkspace.getState().tabs.filter((t) => kinds.has(t.kind))
  if (open.length) {
    const tabs = open.length === 1 ? `“${open[0].title}”` : `${open.length} tabs`
    const ok = await confirmDialog(`${plugin.name} can’t turn off while ${tabs} ${open.length === 1 ? 'is' : 'are'} open. Writan asks before closing anything with unsaved changes.`, {
      title: `Close ${tabs} and turn off ${plugin.name}?`,
      okLabel: open.length === 1 ? 'Close Tab' : 'Close Tabs',
    })
    if (!ok) return
    for (const tab of open) if (!(await useWorkspace.getState().closeTab(tab.id))) return
  }
  // LaTeX uses the computer's own TeX; offer to remove that too (unticked unless they choose).
  if (plugin.services?.includes('tex') && !(await useTex.getState().confirmTurnOff())) return
  await disable(id)
}

function AutosavePage() {
  const autosave = useSettings((s) => s.autosave)
  const setAutosave = useSettings((s) => s.setAutosave)
  const plugins = usePlugins((s) => s.enabled)

  return (
    <>
      <p className="settings-intro">
        A file that already lives somewhere is saved there a few seconds after you stop editing. A new document stays unsaved until you save it with {shortcut('⌘S')}.
      </p>

      <SettingsRow label="Save automatically" hint={`Turn this off and every file waits for ${shortcut('Save')}.`}>
        <Switch label="Autosave" checked={autosave.enabled} onChange={(enabled) => setAutosave({ enabled })} />
      </SettingsRow>

      <fieldset className="settings-fields" disabled={!autosave.enabled}>
        <SettingsRow label="Save after you stop editing for" hint="If you keep going, it still saves every 30 seconds.">
          <Segmented value={String(autosave.delay)} options={DELAYS} onChange={(v) => setAutosave({ delay: Number(v) })} disabled={!autosave.enabled} />
        </SettingsRow>

        <SettingsRow column bare label="Autosave these files" hint={`Files you untick wait for ${shortcut('Save')}.`}>
          <div className="kind-checks">
            {enabledKinds(plugins).map((k) => (
              <Checkbox
                key={k.id}
                checked={autosave.kinds[k.id] !== false}
                onChange={(on) => setAutosave({ kinds: { ...autosave.kinds, [k.id]: on } })}
                label={k.short}
              />
            ))}
          </div>
        </SettingsRow>
      </fieldset>

      <HistoryRows />
    </>
  )
}

/** Version history: on or off, and how much room it takes. */
function HistoryRows() {
  const history = useSettings((s) => s.history)
  const setHistory = useSettings((s) => s.setHistory)
  const [usage, setUsage] = useState<number | null>(null)
  useEffect(() => {
    void historyUsage()
      .then(setUsage)
      .catch(() => setUsage(null))
  }, [])
  const clear = async () => {
    const ok = await confirmDialog('Every earlier version of every file is deleted. The files themselves aren’t touched.', { title: 'Delete all version history?', okLabel: 'Delete', danger: true })
    if (!ok) return
    try {
      await clearAllHistory()
      setUsage(0)
    } catch (err) {
      await showError(String(err))
    }
  }
  return (
    <>
      <SettingsRow
        label="Version history"
        hint="Keep a copy each time a file is saved, so you can compare with or go back to an earlier version (right-click a tab, then Version history). Every save from the last hour is kept, then fewer as they get older. Copies stay on this computer."
      >
        <Switch label="Version history" checked={history} onChange={setHistory} />
      </SettingsRow>
      <SettingsRow label="Space used" hint={usage === null ? '…' : usage === 0 ? 'Nothing yet.' : `${formatVersionSize(usage)} for every file’s versions.`}>
        <Button variant="danger" disabled={!usage} onClick={() => void clear()}>
          Delete all versions
        </Button>
      </SettingsRow>
    </>
  )
}

function AppearancePage() {
  const appIcon = useSettings((s) => s.appIcon)
  const setAppIcon = useSettings((s) => s.setAppIcon)
  const mode = useWorkspace((s) => s.themeMode)
  const appearance = useWorkspace((s) => s.theme)
  const setThemeMode = useWorkspace((s) => s.setThemeMode)
  const light = useAppearance((s) => s.light)
  const dark = useAppearance((s) => s.dark)
  const custom = useAppearance((s) => s.custom)
  const { use, remove } = useAppearance.getState()
  const [editing, setEditing] = useState<ThemeDef | null>(null)
  const themes = allThemes(custom)
  const current = themes.find((t) => t.id === (appearance === 'dark' ? dark : light)) ?? themes[0]

  return (
    <>
      <SettingsRow
        label="Light and dark"
        hint={shortcut(keysFor('toggle-theme') ? `System follows your computer. Switch between light and dark any time with ${keysFor('toggle-theme')}.` : 'System follows your computer.')}
      >
        <Segmented
          value={mode}
          onChange={setThemeMode}
          options={[
            { value: 'system', label: 'System', icon: <Monitor size={14} /> },
            { value: 'light', label: 'Light', icon: <Sun size={14} /> },
            { value: 'dark', label: 'Dark', icon: <Moon size={14} /> },
          ]}
        />
      </SettingsRow>

      <SettingsRow
        label="App icon"
        column
        hint={!isTauri ? 'Changes the icon in the desktop app. The browser icon stays the same.' : isMac ? 'Changes the Dock icon while the app is open. Finder always uses the default, as does the Dock when the app is closed.' : 'Changes the window and taskbar icon while the app is open.'}
      >
        <ChoiceTiles
          label="App icon"
          value={appIcon}
          onChange={setAppIcon}
          options={[
            { value: 'dark', label: 'Dark', icon: <Logo variant="dark" size={48} /> },
            { value: 'blue', label: 'Blue', icon: <Logo variant="blue" size={48} /> },
            { value: 'light', label: 'Light', icon: <Logo variant="light" size={48} /> },
          ]}
        />
      </SettingsRow>

      <section className="theme-section">
        <div className="theme-section-header">
          <h4>Themes</h4>
          {!editing && (
            <Button title="Starts as a copy of the theme you’re using" onClick={() => setEditing(copyOf(current))}>
              <Plus size={14} /> Create theme
            </Button>
          )}
        </div>
        {editing && (
          <ThemeEditor
            theme={editing}
            onChange={setEditing}
            onCancel={() => setEditing(null)}
            onSave={() => {
              useAppearance.getState().save(editing)
              // A new theme is used straight away.
              if (!custom.some((t) => t.id === editing.id)) use(editing.id)
              setEditing(null)
            }}
          />
        )}
        <div className="theme-grid">
          {themes.map((t) => (
            <div key={t.id} className="theme-card">
              <div className="theme-orbs">
                <Orb theme={t} mode="light" selected={light === t.id} onSelect={() => use(t.id, 'light')} />
                <Orb theme={t} mode="dark" selected={dark === t.id} onSelect={() => use(t.id, 'dark')} />
              </div>
              <div className="theme-card-foot">
                <button type="button" className="theme-name" title="Use for light and dark" onClick={() => use(t.id)}>
                  {t.name}
                </button>
                {t.custom && (
                  <>
                    <IconButton label="Edit theme" size="sm" onClick={() => setEditing({ ...t })}>
                      <Pencil size={13} />
                    </IconButton>
                    <IconButton
                      label="Delete theme"
                      size="sm"
                      onClick={() =>
                        void confirmDialog('You can’t undo this.', { title: `Delete the theme “${t.name}”?`, okLabel: 'Delete', danger: true }).then((ok) => ok && remove(t.id))
                      }
                    >
                      <Trash2 size={13} />
                    </IconButton>
                  </>
                )}
              </div>
            </div>
          ))}
        </div>
      </section>
    </>
  )
}

const newThemeId = () => `custom-${Math.random().toString(36).slice(2, 8)}`
const copyOf = (theme: ThemeDef): ThemeDef => ({ ...theme, id: newThemeId(), name: `${theme.name} copy`, custom: true })

/** Two orbs per theme: how it looks in light and in dark. Click one to use the theme for just that appearance. */
function Orb({ theme, mode, selected, onSelect }: { theme: ThemeDef; mode: Appearance; selected: boolean; onSelect: () => void }) {
  return (
    <button
      type="button"
      className={`theme-orb ${selected ? 'on' : ''}`}
      title={`Use for ${mode} mode only`}
      aria-label={`${theme.name}: use for ${mode} mode only`}
      aria-pressed={selected}
      onClick={onSelect}
    >
      <span className="theme-orb-ball" style={{ background: orbBackground(theme, mode) }} />
      {selected && <span className="theme-orb-badge">{mode === 'light' ? <Sun size={11} /> : <Moon size={11} />}</span>}
    </button>
  )
}

/** The name, the accent, and the tone each appearance takes its surfaces from. */
function ThemeEditor({ theme, onChange, onSave, onCancel }: { theme: ThemeDef; onChange: (theme: ThemeDef) => void; onSave: () => void; onCancel: () => void }) {
  const valid = theme.name.trim().length > 0 && isHex(theme.accent) && isHex(theme.light) && isHex(theme.dark)
  const swatch = (label: string, key: 'accent' | 'light' | 'dark') => (
    <label className="theme-color">
      <input type="color" value={theme[key]} onChange={(e) => onChange({ ...theme, [key]: e.currentTarget.value })} />
      <span>{label}</span>
    </label>
  )
  return (
    <div className="theme-editor">
      <div className="theme-orbs preview" aria-hidden="true">
        <span className="theme-orb-ball" style={{ background: orbBackground(theme, 'light') }} />
        <span className="theme-orb-ball" style={{ background: orbBackground(theme, 'dark') }} />
      </div>
      <div className="theme-editor-fields">
        <TextField value={theme.name} maxLength={40} placeholder="Theme name" onChange={(e) => onChange({ ...theme, name: e.currentTarget.value })} />
        <div className="theme-colors">
          {swatch('Accent', 'accent')}
          {swatch('Light tone', 'light')}
          {swatch('Dark tone', 'dark')}
        </div>
        <div className="theme-editor-actions">
          <Button variant="primary" disabled={!valid} onClick={onSave}>
            Save theme
          </Button>
          <Button onClick={onCancel}>Cancel</Button>
        </div>
      </div>
    </div>
  )
}

/** Every shortcut, grouped; click one and press new keys to change it. */
function ShortcutsPage() {
  const custom = useShortcuts((s) => s.custom)
  const resetAll = useShortcuts((s) => s.resetAll)
  const [query, setQuery] = useState('')
  const [editing, setEditing] = useState<string | null>(null)
  const [note, setNote] = useState<{ id: string; text: string } | null>(null)
  const q = query.trim().toLowerCase()
  const shown = SHORTCUTS.filter((s) => !q || s.label.toLowerCase().includes(q) || s.group.toLowerCase().includes(q) || shortcut(keysFor(s.id, custom)).toLowerCase().includes(q))
  const groups = [...new Set(shown.map((s) => s.group))]
  const changed = Object.keys(custom).length > 0

  return (
    <>
      <p className="settings-intro">Click a shortcut and press the keys you want instead. Press {isMac ? '⌫' : 'Backspace'} to remove one, or Esc to leave it as it was.</p>
      <div className="shortcut-tools">
        <TextField className="shortcut-search" icon={<Search size={13} className="faint" />} value={query} placeholder="Search shortcuts" onChange={(e) => setQuery(e.currentTarget.value)} />
        <Button disabled={!changed} onClick={() => (resetAll(), setNote(null))}>
          <RotateCcw size={13} /> Reset all
        </Button>
      </div>
      {groups.map((group) => (
        <section key={group} className="shortcut-group">
          <h4>{group}</h4>
          {shown
            .filter((s) => s.group === group)
            .map((s) => (
              <ShortcutRow
                key={s.id}
                def={s}
                keys={keysFor(s.id, custom)}
                changed={s.id in custom}
                editing={editing === s.id}
                note={note?.id === s.id ? note.text : null}
                onEdit={(on) => {
                  setEditing(on ? s.id : null)
                  if (on) setNote(null)
                }}
                onNote={(text) => setNote(text ? { id: s.id, text } : null)}
              />
            ))}
        </section>
      ))}
      {!groups.length && <p className="settings-hint">No shortcuts match “{query}”.</p>}
    </>
  )
}

function ShortcutRow({
  def,
  keys,
  changed,
  editing,
  note,
  onEdit,
  onNote,
}: {
  def: ShortcutDef
  keys: string
  changed: boolean
  editing: boolean
  note: string | null
  onEdit: (on: boolean) => void
  onNote: (text: string | null) => void
}) {
  const assign = useShortcuts((s) => s.assign)
  const reset = useShortcuts((s) => s.reset)
  const [hint, setHint] = useState<string | null>(null)
  // The page passes new callbacks every render; recording must not restart for that.
  const done = useRef({ onEdit, onNote })
  done.current = { onEdit, onNote }

  useEffect(() => {
    if (!editing) return
    const { onEdit, onNote } = done.current
    recording = true
    setHint(null)
    void syncMenu({ paused: true })
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault()
      e.stopPropagation()
      const pressed = keysFromEvent(e)
      if (!pressed) return
      if (pressed === 'Esc') return onEdit(false)
      if (pressed === '⌫' || pressed === '⌦') {
        assign(def.id, '')
        return onEdit(false)
      }
      if (!isValidKeys(pressed)) {
        setHint(shortcut(`Include ⌘ in the shortcut, or use a function key.`))
        return
      }
      const lost = assign(def.id, pressed)
      onNote(lost.length ? `Removed from ${lost.map((l) => l.label).join(', ')}.` : null)
      onEdit(false)
    }
    // Leaving the field, or clicking elsewhere, stops recording.
    const onDown = (e: MouseEvent) => !(e.target as HTMLElement).closest('.shortcut-keys.recording') && onEdit(false)
    window.addEventListener('keydown', onKey, true)
    window.addEventListener('mousedown', onDown, true)
    return () => {
      recording = false
      window.removeEventListener('keydown', onKey, true)
      window.removeEventListener('mousedown', onDown, true)
      void syncMenu()
    }
  }, [editing, def.id, assign])

  return (
    <div className="shortcut-row">
      <span className="shortcut-label">
        {def.label}
        {def.fixed && <span className="settings-hint"> · can’t be changed</span>}
        {note && <span className="shortcut-note">{note}</span>}
        {editing && hint && <span className="shortcut-note">{hint}</span>}
      </span>
      {changed && !editing && (
        <IconButton label="Use the default" size="sm" onClick={() => (reset(def.id), onNote(null))}>
          <RotateCcw size={13} />
        </IconButton>
      )}
      <button
        type="button"
        className={`shortcut-keys ${editing ? 'recording' : ''} ${keys ? '' : 'none'}`}
        disabled={def.fixed}
        aria-label={`${def.label}: ${keys ? shortcut(keys) : 'no shortcut'}. Change`}
        onClick={() => onEdit(!editing)}
      >
        {editing ? 'Press keys…' : keys ? <Keys keys={keys} /> : 'None'}
      </button>
    </div>
  )
}

/** Keys as separate caps on a Mac (⇧ ⌘ N) and as Ctrl+Shift+N on Windows. */
function Keys({ keys }: { keys: string }) {
  const text = shortcut(keys)
  const caps = text.includes('+') ? text.split('+') : Array.from(text.match(/[⌃⌥⇧⌘]|[^⌃⌥⇧⌘]+/g) ?? [text])
  return (
    <>
      {caps.map((c, i) => (
        <kbd key={i}>{c}</kbd>
      ))}
    </>
  )
}
