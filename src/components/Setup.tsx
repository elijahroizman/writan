import { ArrowLeft, ArrowRight } from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useState } from 'react'
import { canSetDefaults, defaultStatus, makeDefault } from '../lib/defaults'
import { isMac, showError } from '../lib/platform'
import { PLUGINS, isBuiltIn } from '../plugins/catalog'
import { CORE_FONTS, offerCoreFonts, useExtraFonts } from '../state/fonts'
import { enabledKinds, usePlugins } from '../state/plugins'
import { useSetup } from '../state/setup'
import { useWorkspace } from '../state/workspace'
import { keysFor } from '../state/shortcuts'
import { CoreFontsOffer, CoreFontsSpecimen, NO_CALIBRI } from './CoreFonts'
import { PluginRow } from './PluginList'
import { Button, Checkbox, DocBadge, Hint, Kbd, LinkButton, Logo, Modal, Spinner, StepDots, Switch } from './ui'

type Step = 'welcome' | 'palette' | 'word' | 'slides' | 'pdf' | 'extras' | 'fonts' | 'split' | 'defaults' | 'windows'

/**
 * The first-launch screen (and Settings › General › Show the tour again): a short tour of what
 * Writan does, the optional extensions, Microsoft's fonts on Linux, and on a Mac or Windows which
 * file types it should open. Every step can be skipped.
 */
export default function SetupScreen() {
  const coreFonts = useExtraFonts(offerCoreFonts)
  const steps: Step[] = [
    'welcome',
    'palette',
    'word',
    'slides',
    'pdf',
    'extras',
    ...(coreFonts ? (['fonts'] as const) : []),
    'split',
    ...(canSetDefaults ? (['defaults'] as const) : []),
  ]
  const [step, setStep] = useState<Step>('welcome')
  const at = steps.indexOf(step)
  const finish = () => useSetup.getState().finish()
  const next = () => (at + 1 < steps.length ? setStep(steps[at + 1]) : finish())
  const back = () => at > 0 && setStep(steps[at - 1])

  // ← and → step through the tour (not while typing or on a checkbox).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.metaKey || e.ctrlKey || e.altKey) return
      if (e.key === 'ArrowRight' && step !== 'defaults' && step !== 'windows') next()
      else if (e.key === 'ArrowLeft' && step !== 'windows') back()
      else return
      e.preventDefault()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  return (
    <Modal dim="blur" layer="welcome">
      <div className="welcome" role="dialog" aria-modal="true" aria-labelledby="welcome-title">
        {step === 'welcome' ? (
          <Welcome onTour={next} onSkip={finish} />
        ) : step === 'defaults' ? (
          <DefaultsStep onBack={back} onDone={(windowsNote) => (windowsNote ? setStep('windows') : finish())} />
        ) : step === 'windows' ? (
          <div className="welcome-hero">
            <Logo size={64} />
            <h1 id="welcome-title">One more thing in Windows Settings</h1>
            <p className="muted">In the Settings window that opened, choose Writan for each file type. Everything else is ready.</p>
            <Button variant="primary" autoFocus onClick={finish}>
              Start using Writan
            </Button>
          </div>
        ) : (
          <>
            <TourStep step={step} />
            <footer className="tour-footer">
              <StepDots count={steps.length - 1} current={at - 1} onPick={(i) => setStep(steps[i + 1])} label="Tour steps" />
              <span className="tour-footer-actions">
                {at + 1 < steps.length && <LinkButton onClick={finish}>Skip tour</LinkButton>}
                <Button onClick={back} aria-label="Back">
                  <ArrowLeft size={14} />
                </Button>
                <Button variant="primary" autoFocus onClick={next}>
                  {at + 1 < steps.length ? (
                    <>
                      Next <ArrowRight size={14} />
                    </>
                  ) : (
                    'Start using Writan'
                  )}
                </Button>
              </span>
            </footer>
          </>
        )}
      </div>
    </Modal>
  )
}

function Welcome({ onTour, onSkip }: { onTour: () => void; onSkip: () => void }) {
  const builtIn = PLUGINS.filter(isBuiltIn).flatMap((p) => p.kinds)
  return (
    <div className="welcome-hero">
      <Logo size={88} label="Writan" />
      <h1 id="welcome-title">Welcome to Writan</h1>
      <p className="muted">A small, fast app for the documents most people actually open.</p>
      <div className="welcome-kinds">
        {builtIn.map((k) => (
          <span key={k.id} className="welcome-kind">
            <DocBadge kind={k.id} size={18} />
            {k.prose === 'text' || k.prose === 'image' ? k.short : k.prose}
          </span>
        ))}
      </div>
      <div className="welcome-actions">
        <Button variant="primary" autoFocus onClick={onTour}>
          Show me around <ArrowRight size={14} />
        </Button>
        <Button variant="quiet" onClick={onSkip}>
          Skip
        </Button>
      </div>
    </div>
  )
}

/** A shortcut and what it does, in a tour step's list. */
function KeyRow({ id, keys, children }: { id?: string; keys?: string; children: ReactNode }) {
  const shown = keys ?? (id ? keysFor(id) : '')
  return (
    <li>
      <span>{children}</span>
      {shown ? <Kbd keys={shown} /> : null}
    </li>
  )
}

function TourStep({ step }: { step: Step }) {
  const page = TOUR[step as keyof typeof TOUR]
  return (
    <div className="tour">
      <div className="tour-art" aria-hidden>
        {page.art}
      </div>
      <div className="tour-text">
        <span className="tour-eyebrow">{page.eyebrow}</span>
        <h1 id="welcome-title">{page.title}</h1>
        <p className="muted">{page.body}</p>
        {page.keys && <ul className="tour-keys">{page.keys}</ul>}
        {page.extra}
      </div>
    </div>
  )
}

interface TourPage {
  eyebrow: string
  title: string
  body: string
  keys: ReactNode
  art: ReactNode
  extra?: ReactNode
}

const TOUR: Record<'palette' | 'word' | 'slides' | 'pdf' | 'extras' | 'fonts' | 'split', TourPage> = {
  palette: {
    eyebrow: 'Command palette',
    title: 'Everything is one search away',
    body: 'Type a few letters to run any command, open a recent file, jump to a tab or change a setting. Commands show their shortcuts, so you pick them up as you go.',
    keys: (
      <>
        <KeyRow id="command-palette">Command palette</KeyRow>
        <KeyRow id="new">New file</KeyRow>
        <KeyRow id="open">Open a file or folder</KeyRow>
      </>
    ),
    art: <Shot name="palette" />,
  },
  word: {
    eyebrow: 'Word documents',
    title: 'Pages that look like they will print',
    body: 'Edit .docx files with styles, lists, tables, pictures, links, comments, headers and footers. The outline is on the left, formatting on the right.',
    keys: (
      <>
        <KeyRow id="replace">Find and replace</KeyRow>
        <KeyRow id="docx.comment">Add a comment</KeyRow>
        <KeyRow id="focus-mode">Focus mode</KeyRow>
      </>
    ),
    art: <Shot name="word" />,
  },
  slides: {
    eyebrow: 'Presentations',
    title: 'Slides, notes and the show',
    body: 'Edit text, shapes, tables and pictures on PowerPoint slides. Drag thumbnails to reorder, write speaker notes under the slide, add transitions and present full screen.',
    keys: (
      <>
        <KeyRow id="pptx.new-slide">New slide</KeyRow>
        <KeyRow id="pptx.duplicate">Duplicate</KeyRow>
        <KeyRow id="pptx.present">Play from this slide</KeyRow>
      </>
    ),
    art: <Shot name="slides" />,
  },
  pdf: {
    eyebrow: 'PDF',
    title: 'Mark up, fill in, sign',
    body: 'Highlight, add text and drawings, fill in forms and sign with a saved signature. Rotate, move, insert or extract pages from the thumbnails, and save a flattened copy to send.',
    keys: (
      <>
        <KeyRow keys="">Sign: in the inspector, under Sign</KeyRow>
        <KeyRow keys="">Pages: drag thumbnails, or right-click one</KeyRow>
        <KeyRow id="find">Find in the PDF</KeyRow>
      </>
    ),
    art: <Shot name="pdf" />,
  },
  extras: {
    eyebrow: 'Fonts and extensions',
    title: 'Add only what you need',
    body: 'Writan stays small. Extra fonts for Word and PowerPoint download when you turn them on, and extensions add file types. Both live in Settings › Plugins.',
    keys: null,
    art: <Shot name="extras" />,
    extra: <ExtensionsList />,
  },
  // Linux only (offerCoreFonts): Mac and Windows come with these fonts.
  fonts: {
    eyebrow: 'Fonts for Office files',
    title: 'Use Microsoft’s own fonts',
    body: 'Most Word and PowerPoint files are set in Arial, Times New Roman or Courier New. Linux doesn’t include them, so Writan draws them with look-alikes that take the same space. Microsoft lets you download the originals if you accept its license.',
    keys: null,
    art: <CoreFontsSpecimen />,
    extra: (
      <>
        <CoreFontsOffer />
        <Hint tone="faint" as="p">
          {`${CORE_FONTS.about} From Microsoft’s original installers, which Writan checks before unpacking. ${NO_CALIBRI} You can add or remove them later in Settings › Plugins.`}
        </Hint>
      </>
    ),
  },
  split: {
    eyebrow: 'Split view',
    title: 'Two documents side by side',
    body: 'Put a Word document or PDF beside another one, or beside a second copy of itself: questions on one side, answers on the other. Right-click a tab and pick Open to the side… to start from there.',
    keys: (
      <>
        <KeyRow id="split-view">Split view</KeyRow>
        <KeyRow id="toggle-left">Sidebar</KeyRow>
        <KeyRow id="toggle-right">Inspector</KeyRow>
      </>
    ),
    art: <Shot name="split" />,
  },
}

/** The optional plugins, with switches: the setup screen's old plugin step, in the tour. */
function ExtensionsList() {
  const enabled = usePlugins((s) => s.enabled)
  const progress = usePlugins((s) => s.progress)
  const { enable, disable } = usePlugins.getState()
  return (
    <div className="tour-extensions">
      {PLUGINS.filter((p) => !isBuiltIn(p)).map((p) => (
        <PluginRow
          key={p.id}
          plugin={p}
          control={progress[p.id] ? <Spinner size={16} /> : <Switch label={`Use ${p.name}`} checked={!!enabled[p.id]} onChange={(on) => void (on ? enable(p.id) : disable(p.id))} />}
        />
      ))}
    </div>
  )
}

function DefaultsStep({ onBack, onDone }: { onBack: () => void; onDone: (windowsNote: boolean) => void }) {
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
        <span className="tour-eyebrow">Last step</span>
        <h1 id="welcome-title">Open files with Writan?</h1>
        <p className="muted">
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
        <span />
        <span className="tour-footer-actions">
          <Button disabled={busy} onClick={onBack} aria-label="Back">
            <ArrowLeft size={14} />
          </Button>
          <Button variant="primary" disabled={busy} onClick={() => void apply()}>
            {wanted.length ? 'Set as default' : 'Start using Writan'}
          </Button>
        </span>
      </footer>
    </>
  )
}

/* ───────────────────────── The tour's pictures ─────────────────────────
 * Screenshots of the app in light and dark, made by scripts/tour-shots.mjs. The palette's pictures show this
 * system's shortcuts.
 */

const SHOTS = import.meta.glob<string>('./tour/*.webp', { eager: true, import: 'default' })

function Shot({ name }: { name: string }) {
  const theme = useWorkspace((s) => s.theme)
  const system = name === 'palette' ? (isMac ? '-mac' : '-pc') : ''
  return <img className="tour-shot" src={SHOTS[`./tour/${name}-${theme}${system}.webp`]} alt="" draggable={false} />
}
