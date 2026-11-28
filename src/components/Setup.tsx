import { ArrowLeft, ArrowRight, Check, Users } from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useRef, useState } from 'react'
import { AGENT_IDS, AGENTS, type AgentId } from '../collab/agents/identity'
import { useAgentsStatus } from '../collab/agents/status'
import { canSetDefaults, defaultStatus, makeDefault } from '../lib/defaults'
import type { CliStatus } from '../lib/platform'
import { cliStatus, isMac, isTauri, openExternal, setCliInstalled, showError } from '../lib/platform'
import { PLUGINS, isBuiltIn } from '../plugins/catalog'
import { useAi } from '../state/ai'
import { analyticsShownOn, setAnalytics, track, useAnalytics } from '../state/analytics'
import { enabledKinds, usePlugins } from '../state/plugins'
import { useSetup } from '../state/setup'
import { keysFor } from '../state/shortcuts'
import { INSTALL_CHOICES, useTex } from '../state/tex'
import type { UpdateChannel } from '../state/updateChannel'
import { getUpdateChannel, setUpdateChannel } from '../state/updateChannel'
import { useWorkspace } from '../state/workspace'
import { useAgentSetup } from './AiSettings'
import { disablePlugin, PluginRow } from './PluginList'
import { Button, Checkbox, DocBadge, Kbd, LinkButton, Logo, Modal, Spinner, StepDots, Switch } from './ui'

type Step = 'welcome' | 'collab' | 'agents' | 'extras' | 'defaults' | 'done'

const PRIVACY_URL = 'https://github.com/elijahroizman/writan/blob/main/PRIVACY.md'

/** What happened on the way, for `onboarding_finished`. */
interface Progress {
  skipped: boolean
  seen: Set<Step>
  connected: Set<AgentId>
  cli: boolean
}

/**
 * The onboarding, on the first launch of each version of it (state/setup.ts) and from Settings ›
 * General › Show the tour again: what Writan is, sharing, AI agents, the extras, the default
 * apps on a Mac or Windows, and last the update channel and usage data. Skip jumps to the last
 * page, which records those two choices, so it can't be dismissed.
 */
export default function SetupScreen() {
  const steps: Step[] = ['welcome', 'collab', 'agents', 'extras', ...(canSetDefaults ? (['defaults'] as const) : []), 'done']
  const [step, setStep] = useState<Step>('welcome')
  const [windowsNote, setWindowsNote] = useState(false)
  const progress = useRef<Progress>({ skipped: false, seen: new Set(['welcome']), connected: new Set(), cli: false })
  const at = steps.indexOf(step)
  const go = (to: Step) => {
    progress.current.seen.add(to)
    setStep(to)
  }
  const next = () => at + 1 < steps.length && go(steps[at + 1])
  const back = () => at > 0 && go(steps[at - 1])
  const skip = () => {
    progress.current.skipped = true
    go('done')
  }

  const stepRef = useRef({ step, next, back })
  stepRef.current = { step, next, back }

  // ← and → step through it, but not while typing or on a radio button. → doesn't leave the
  // default apps, which has its own button, or finish on the last page.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.metaKey || e.ctrlKey || e.altKey) return
      const { step: cur, next: goNext, back: goBack } = stepRef.current
      if (e.key === 'ArrowRight' && cur !== 'defaults' && cur !== 'done') goNext()
      else if (e.key === 'ArrowLeft') goBack()
      else return
      e.preventDefault()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const finish = (analytics: boolean, channel: UpdateChannel) => {
    setAnalytics(analytics)
    setUpdateChannel(channel)
    const p = progress.current
    track('onboarding_finished', {
      skipped: p.skipped,
      steps_seen: p.seen.size,
      steps_total: steps.length,
      agents_connected: p.connected.size,
      latex_enabled: !!usePlugins.getState().enabled.latex,
      cli_installed: p.cli,
      upgraded: useSetup.getState().upgraded,
    })
    useSetup.getState().finish()
  }

  const dots = <StepDots count={steps.length} current={at} onPick={(i) => go(steps[i])} label="Onboarding pages" />
  const skipLink = <LinkButton onClick={skip}>Skip</LinkButton>

  return (
    <Modal dim="blur" layer="welcome">
      <div className="welcome" role="dialog" aria-modal="true" aria-labelledby="welcome-title" aria-describedby="welcome-body" data-step={step}>
        {step === 'defaults' ? (
          <DefaultsStep
            key={step}
            dots={dots}
            skip={skipLink}
            onBack={back}
            onDone={(note) => {
              setWindowsNote(note)
              next()
            }}
          />
        ) : step === 'done' ? (
          <DoneStep key={step} dots={dots} onBack={back} windowsNote={windowsNote} onFinish={finish} />
        ) : (
          <>
            <Page key={step} {...pageFor(step, progress.current)} />
            <footer className="tour-footer">
              {dots}
              <span className="tour-footer-actions">
                {skipLink}
                {at > 0 && (
                  <Button onClick={back} aria-label="Back">
                    <ArrowLeft size={14} />
                  </Button>
                )}
                <Button key={step} variant="primary" autoFocus onClick={next}>
                  {step === 'welcome' ? 'Get started' : 'Next'} <ArrowRight size={14} />
                </Button>
              </span>
            </footer>
          </>
        )}
      </div>
    </Modal>
  )
}

interface PageInfo {
  eyebrow: string
  title: string
  body: ReactNode
  art: ReactNode
  extra?: ReactNode
}

function Page({ eyebrow, title, body, art, extra }: PageInfo) {
  return (
    <div className="tour">
      <div className="tour-art" aria-hidden>
        {art}
      </div>
      <div className="tour-text">
        <span className="tour-eyebrow">{eyebrow}</span>
        <h1 id="welcome-title">{title}</h1>
        <p id="welcome-body" className="muted">
          {body}
        </p>
        {extra}
      </div>
    </div>
  )
}

function pageFor(step: Step, progress: Progress): PageInfo {
  switch (step) {
    case 'welcome':
      return {
        eyebrow: 'Welcome to Writan',
        title: 'One app for the documents people send you',
        body: 'Writan opens and edits Word documents, presentations, spreadsheets and PDFs right on your computer. You can work on them with other people at the same time, or let an AI agent help.',
        art: <Collage />,
        extra: <Formats />,
      }
    case 'collab':
      return {
        eyebrow: 'Live collaboration',
        title: 'Edit together, live',
        body: (
          <>
            Click Share{' '}
            <span className="tour-inline-btn" aria-label="the people button">
              <Users size={13} />
            </span>{' '}
            at the top right of a document to get a six-digit code. Whoever types it in on Home joins you, and you all see each other’s carets and edits as they happen.
          </>
        ),
        art: <Shot name="collab" />,
        extra: (
          <ul className="tour-facts">
            <li>The file stays on your computer, and you save it as usual.</li>
            <li>You can share Word documents, presentations and spreadsheets.</li>
          </ul>
        ),
      }
    case 'agents':
      return {
        eyebrow: 'AI agents',
        title: 'Bring your AI agent in',
        body: 'A connected agent such as Claude Code or Codex can write and rewrite text, fill in tables and build slides and charts in the documents you have open. Its caret and name show on the page, like a person’s.',
        art: <Shot name="agents" />,
        extra: <AgentsList onConnected={(id) => progress.connected.add(id)} />,
      }
    case 'extras':
      return {
        eyebrow: 'Extras',
        title: 'Add what you need',
        body: 'These are off until you turn them on. Settings › General has them later, too.',
        art: <Shot name="extras" />,
        extra: <Extras onCli={() => (progress.cli = true)} />,
      }
    default:
      throw new Error(`No page for ${step}`)
  }
}

/** The file types Writan opens without extensions, and the palette's shortcut. */
function Formats() {
  const builtIn = PLUGINS.filter(isBuiltIn).flatMap((p) => p.kinds)
  return (
    <>
      <div className="welcome-kinds">
        {builtIn.map((k) => (
          <span key={k.id} className="welcome-kind">
            <DocBadge kind={k.id} size={18} />
            {k.prose === 'text' || k.prose === 'image' ? k.short : k.prose}
          </span>
        ))}
      </div>
      <ul className="tour-keys">
        <li>
          <span>Find any command, file or setting</span>
          <Kbd keys={keysFor('command-palette')} />
        </li>
      </ul>
    </>
  )
}

/* ───────────────────────── AI agents ───────────────────────── */

/** The MCP address, turning agents on first if someone had turned them off. */
async function agentsAddress(): Promise<string> {
  if (!useAi.getState().agents) useAi.getState().setAgents(true)
  const ready = (s: ReturnType<typeof useAgentsStatus.getState>) => (s.state === 'on' && s.local) || (s.state === 'error' && new Error(s.message ?? 'Agents can’t connect.'))
  const now = ready(useAgentsStatus.getState())
  const settled =
    now ||
    (await new Promise<string | Error>((resolve) => {
      const timer = window.setTimeout(() => (stop(), resolve(new Error('Agents didn’t start. Try again from Settings › AI.'))), 20_000)
      const stop = useAgentsStatus.subscribe((s) => {
        const r = ready(s)
        if (!r) return
        stop()
        window.clearTimeout(timer)
        resolve(r)
      })
    }))
  if (settled instanceof Error) throw settled
  return settled
}

/** Each agent Writan found on this computer, with a button that connects it. */
function AgentsList({ onConnected }: { onConnected: (id: AgentId) => void }) {
  const [changes, setChanges] = useState(0)
  const [present, setPresent] = useState<Record<AgentId, boolean> | null>(null)
  const changed = () => setChanges((n) => n + 1)

  useEffect(() => {
    if (!isTauri) return
    void (async () => {
      const { invoke } = await import('@tauri-apps/api/core')
      const found = await Promise.all(AGENT_IDS.map((id) => invoke<{ present: boolean }>('agents_installed', { agent: id }).then((r) => !!r?.present, () => false)))
      setPresent(Object.fromEntries(AGENT_IDS.map((id, i) => [id, found[i]])) as Record<AgentId, boolean>)
    })()
  }, [])

  // In a browser there's nothing to find: list them all, as the app would.
  const shown = !isTauri ? AGENT_IDS : present ? AGENT_IDS.filter((id) => present[id]) : null
  if (!shown) return <Spinner size={16} />
  const missing = AGENT_IDS.filter((id) => !shown.includes(id))
  return (
    <div className="tour-agents">
      {shown.length === 0 ? (
        <p className="settings-hint">Writan didn’t find an agent on this computer. Once you install one, connect it from Settings › AI.</p>
      ) : (
        shown.map((id) => <AgentConnect key={id} id={id} changes={changes} onChange={changed} onConnected={onConnected} />)
      )}
      {shown.length > 0 && missing.length > 0 && <p className="settings-hint tour-agents-more">Settings › AI also connects {listOf(missing.map((id) => AGENTS[id].product))}.</p>}
    </div>
  )
}

function listOf(names: string[]) {
  return names.length < 2 ? names.join('') : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`
}

function AgentConnect({ id, changes, onChange, onConnected }: { id: AgentId; changes: number; onChange: () => void; onConnected: (id: AgentId) => void }) {
  const { agent, installed, busy, note, connect } = useAgentSetup(id, async () => `${await agentsAddress()}?agent=${id}`, changes, onChange)
  const connected = !!installed?.mcp && !!installed?.skill
  useEffect(() => {
    if (note && !note.error) onConnected(id)
  }, [note, id, onConnected])
  return (
    <div className="tour-agent">
      <span className="agent-logo" aria-hidden>
        <img src={agent.logo} alt="" style={{ transform: `scale(${agent.logoScale ?? 1})` }} />
      </span>
      <span className="tour-agent-text">
        <span className="settings-label">{agent.product}</span>
        {note?.error && <span className="settings-hint error">{note.text}</span>}
      </span>
      {connected ? (
        <span className="tour-agent-done">
          <Check size={14} /> Connected
        </span>
      ) : (
        <Button disabled={!isTauri || busy !== null} onClick={() => void connect()} aria-label={`Connect ${agent.product}`}>
          {busy ? <Spinner size={14} /> : null} Connect
        </Button>
      )}
    </div>
  )
}

/* ───────────────────────── Extras ───────────────────────── */

function Extras({ onCli }: { onCli: () => void }) {
  return (
    <div className="tour-extensions">
      <LatexRow />
      <CommandRow onInstalled={onCli} />
    </div>
  )
}

/** The LaTeX plugin's switch, and the TeX install offer while it's on and there's no TeX. */
function LatexRow() {
  const enabled = usePlugins((s) => s.enabled)
  const progress = usePlugins((s) => s.progress)
  const tex = useTex((s) => s.status)
  const installing = useTex((s) => s.installing)
  const { enable } = usePlugins.getState()
  const plugin = PLUGINS.find((p) => p.services?.includes('tex'))
  const on = !!(plugin && enabled[plugin.id])
  useEffect(() => {
    if (on) void useTex.getState().refresh()
  }, [on])
  if (!plugin) return null
  return (
    <PluginRow
      plugin={plugin}
      control={progress[plugin.id] ? <Spinner size={16} /> : <Switch label={`Use ${plugin.name}`} checked={on} onChange={(v) => void (v ? enable(plugin.id) : disablePlugin(plugin.id))} />}
    >
      {on && tex && !tex.installed && (
        <span className="tour-tex">
          <span className="settings-hint">{installing ? 'Installing TeX…' : 'TeX isn’t installed yet, so documents can’t compile.'}</span>
          {installing ? (
            <Button onClick={() => useTex.setState({ offer: true })}>Show progress</Button>
          ) : (
            INSTALL_CHOICES.length > 0 && <Button onClick={() => void useTex.getState().offerInstall()}>Install TeX…</Button>
          )}
        </span>
      )}
    </PluginRow>
  )
}

/** The `writan` command: a button on a Mac, a note on Linux, where the package installs it, and
 * nothing on Windows. */
function CommandRow({ onInstalled }: { onInstalled: () => void }) {
  const [status, setStatus] = useState<CliStatus | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    void cliStatus().then(setStatus, () => {})
  }, [])
  if (!status || (!status.supported && !status.installed)) return null
  const install = async () => {
    setBusy(true)
    try {
      if (await setCliInstalled(true)) onInstalled()
    } catch (err) {
      await showError(String(err), 'Couldn’t install the command')
    } finally {
      setStatus(await cliStatus())
      setBusy(false)
    }
  }
  const usage = (
    <>
      <code>writan .</code> opens the folder you’re in, <code>writan notes.md</code> a file.
    </>
  )
  return (
    <div className="plugin-row">
      <span className="tour-terminal" aria-hidden>
        &gt;_
      </span>
      <span className="plugin-text">
        <span className="settings-label">Terminal command</span>
        <span className="settings-hint">
          {status.taken ? (
            `Something else is already at ${status.path}, so Writan can’t put its command there.`
          ) : status.supported && !status.installed ? (
            <>Open files and folders from Terminal: {usage} Installing asks for your password.</>
          ) : (
            <>
              {status.supported ? 'Installed' : 'Already installed'} at {status.path}. {usage}
            </>
          )}
        </span>
      </span>
      <span className="plugin-control">
        {status.supported && !status.taken && !status.installed ? (
          <Button disabled={busy} onClick={() => void install()}>
            {busy ? <Spinner size={14} /> : null} Install
          </Button>
        ) : status.installed ? (
          <span className="tour-agent-done">
            <Check size={14} /> Installed
          </span>
        ) : null}
      </span>
    </div>
  )
}

/* ───────────────────────── Default apps ───────────────────────── */

function DefaultsStep({ dots, skip, onBack, onDone }: { dots: ReactNode; skip: ReactNode; onBack: () => void; onDone: (windowsNote: boolean) => void }) {
  const enabled = usePlugins((s) => s.enabled)
  const kinds = enabledKinds(enabled)
  const [defaults, setDefaults] = useState<Record<string, boolean>>({})
  const [already, setAlready] = useState<Record<string, boolean | null>>({})
  const [busy, setBusy] = useState(false)
  const wanted = kinds.filter((k) => defaults[k.id] && !already[k.id])
  const allOn = kinds.every((k) => defaults[k.id] || already[k.id])

  useEffect(() => {
    void defaultStatus(kinds.map((k) => k.id)).then(setAlready, () => {})
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const apply = async () => {
    if (!wanted.length) return onDone(false)
    setBusy(true)
    try {
      onDone(await makeDefault(wanted.map((k) => k.id)))
    } catch (err) {
      await showError(String(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <div className="welcome-defaults">
        <span className="tour-eyebrow">Default apps</span>
        <h1 id="welcome-title">Open files with Writan?</h1>
        <p id="welcome-body" className="muted">
          {isMac
            ? 'Tick the file types Writan should open when you double-click them in Finder.'
            : 'Tick the file types Writan should open when you double-click them. Windows asks you to confirm in its Settings.'}
        </p>
        <div className="setup-defaults">
          {kinds.map((k) => (
            <Checkbox
              key={k.id}
              className="setup-default"
              checked={!!already[k.id] || !!defaults[k.id]}
              disabled={!!already[k.id] || busy}
              onChange={(on) => setDefaults({ ...defaults, [k.id]: on })}
              label={
                <>
                  <DocBadge kind={k.id} size={24} />
                  <span className="setup-default-name">
                    {k.label}
                    <span className="faint"> {k.extensions.map((e) => `.${e}`).join(' ')}</span>
                  </span>
                  {already[k.id] && <span className="faint">Already opens in Writan</span>}
                </>
              }
            />
          ))}
        </div>
        <LinkButton disabled={busy} onClick={() => setDefaults(Object.fromEntries(kinds.map((k) => [k.id, !allOn])))}>
          {allOn ? 'Untick all' : 'Tick all'}
        </LinkButton>
      </div>
      <footer className="tour-footer">
        {dots}
        <span className="tour-footer-actions">
          {skip}
          <Button disabled={busy} onClick={onBack} aria-label="Back">
            <ArrowLeft size={14} />
          </Button>
          <Button variant="primary" autoFocus disabled={busy} onClick={() => void apply()}>
            {wanted.length ? 'Set as default' : 'Next'} <ArrowRight size={14} />
          </Button>
        </span>
      </footer>
    </>
  )
}

/* ───────────────────────── The last page ───────────────────────── */

const CHANNELS: { value: UpdateChannel; label: string; hint: string }[] = [
  { value: 'stable', label: 'Stable', hint: 'Tested releases. Best for most people.' },
  { value: 'nightly', label: 'Nightly', hint: 'New features as soon as they land. A nightly may break things.' },
]

function DoneStep({ dots, onBack, windowsNote, onFinish }: { dots: ReactNode; onBack: () => void; windowsNote: boolean; onFinish: (analytics: boolean, channel: UpdateChannel) => void }) {
  const [channel, setChannel] = useState<UpdateChannel>(getUpdateChannel)
  const [analytics, setAnalyticsOn] = useState(() => analyticsShownOn(useAnalytics.getState().choice))
  return (
    <>
      <div className="welcome-done">
        <Logo size={56} />
        <h1 id="welcome-title">You’re all set</h1>
        <p id="welcome-body" className="muted">
          {windowsNote ? 'In the Windows Settings window that opened, choose Writan for each file type. Then pick two last things here.' : 'Two last things, and you can change both later in Settings › General.'}
        </p>
        <div className="welcome-choices">
          <fieldset className="welcome-choice">
            <legend className="settings-label">Updates</legend>
            <div className="channel-tiles">
              {CHANNELS.map((c) => (
                <label key={c.value} className="channel-tile">
                  <input type="radio" name="update-channel" value={c.value} checked={channel === c.value} onChange={() => setChannel(c.value)} />
                  <span className="channel-tile-text">
                    <span className="settings-label">{c.label}</span>
                    <span className="settings-hint">{c.hint}</span>
                  </span>
                </label>
              ))}
            </div>
          </fieldset>
          <div className="welcome-choice welcome-usage">
            <span className="plugin-text">
              <span className="settings-label">Help make Writan better</span>
              <span className="settings-hint">
                Send anonymous usage data, such as which features get used and how fast documents open. It never includes your documents or anything in them.{' '}
                <LinkButton onClick={() => void openExternal(PRIVACY_URL)}>What Writan collects</LinkButton>
              </span>
            </span>
            <Switch checked={analytics} onChange={setAnalyticsOn} label="Help make Writan better" />
          </div>
        </div>
      </div>
      <footer className="tour-footer">
        {dots}
        <span className="tour-footer-actions">
          <Button onClick={onBack} aria-label="Back">
            <ArrowLeft size={14} />
          </Button>
          <Button variant="primary" autoFocus onClick={() => onFinish(analytics, channel)}>
            Start using Writan
          </Button>
        </span>
      </footer>
    </>
  )
}

/* ───────────────────────── Pictures ─────────────────────────
 * Screenshots of the app in light and dark, made by scripts/tour-shots.mjs.
 */

const SHOTS = import.meta.glob<string>('./tour/*.webp', { eager: true, import: 'default' })

function Shot({ name, className = '' }: { name: string; className?: string }) {
  const theme = useWorkspace((s) => s.theme)
  return <img className={`tour-shot ${className}`} src={SHOTS[`./tour/${name}-${theme}.webp`]} alt="" draggable={false} />
}

/** The welcome page's picture: a PDF, a presentation and a Word document, stacked. */
function Collage() {
  return (
    <div className="tour-collage">
      <Shot name="pdf" className="tour-collage-back" />
      <Shot name="slides" className="tour-collage-middle" />
      <Shot name="word" className="tour-collage-front" />
    </div>
  )
}
