// The system's TeX, for the LaTeX plugin: whether it's installed, compiling, and installing it
// (src-tauri/src/tex.rs). Writan doesn't ship TeX; it offers to install it (MacTeX, MiKTeX or
// TeX Live, through Homebrew, winget or apt where those are there) when the plugin is turned on
// and there's none, and to remove it again when the plugin is turned off.

import { create } from 'zustand'
import type { TexResult, TexService, TexStatus } from '../plugins/api'
import { isLinux, isMac, isTauri, isWindows, listen } from '../lib/platform'
import { track } from './analytics'

const invoke = async <T>(cmd: string, args?: Record<string, unknown>) => (await import('@tauri-apps/api/core')).invoke<T>(cmd, args)

export type InstallVariant = 'mactex' | 'basictex' | 'miktex' | 'texlive' | 'texlive-basic'

export interface InstallProgress {
  stage: 'download' | 'verify' | 'install'
  received: number
  total: number
}

interface TexState {
  status: TexStatus | null
  /** The install offer is showing. */
  offer: boolean
  /**
   * The "turn off LaTeX" question is showing: how TeX would be removed (null while that's being
   * worked out, or when Writan can't remove it), and where that has got to.
   */
  removal: { method: string | null; checking: boolean; busy: boolean; error: string | null } | null
  installing: InstallProgress | null
  error: string | null
  refresh(): Promise<TexStatus>
  /** Shows the install offer; resolves with the status once it closes. */
  offerInstall(): Promise<TexStatus>
  install(variant: InstallVariant): Promise<void>
  close(): void
  /**
   * Called when the LaTeX plugin is turned off. Asks to confirm and, if Writan can take TeX off
   * the computer, whether to do that too (it's off unless they tick it). Resolves false if they
   * cancel turning the plugin off.
   */
  confirmTurnOff(): Promise<boolean>
  /** Answer the question `confirmTurnOff` is asking. */
  answerTurnOff(proceed: boolean, remove: boolean): Promise<void>
}

const NONE: TexStatus = { installed: false, engines: [] }
let closed: ((status: TexStatus) => void)[] = []
let turnOff: ((proceed: boolean) => void) | null = null

export const useTex = create<TexState>((set, get) => ({
  status: null,
  offer: false,
  removal: null,
  installing: null,
  error: null,

  refresh: async () => {
    const hook = texTestHook()
    const status = hook ? await hook.status() : isTauri ? await invoke<TexStatus>('tex_status').catch(() => NONE) : NONE
    set({ status })
    return status
  },

  offerInstall: () => {
    set({ offer: true, error: null })
    return new Promise((resolve) => closed.push(resolve))
  },

  install: async (variant) => {
    if (get().installing) return
    set({ installing: { stage: 'download', received: 0, total: 0 }, error: null })
    let stop = () => {}
    try {
      stop = await listen<InstallProgress>('tex-progress', (payload) => set({ installing: payload }))
      const status = await invoke<TexStatus>('tex_install', { variant })
      set({ status })
      if (status.installed) get().close()
      // If the offer was hidden while installing, it comes back to say what went wrong.
      else set({ error: 'TeX installed, but Writan can’t find it. Restart Writan and try again.', offer: true })
    } catch (err) {
      set({ error: String(err), offer: true })
    } finally {
      stop()
      set({ installing: null })
    }
  },

  confirmTurnOff: () => {
    // The question shows at once. Finding out how TeX was installed can take a while (winget is
    // slow to list packages), and turning LaTeX off never waits for it.
    set({ removal: { method: null, checking: true, busy: false, error: null } })
    void (async () => {
      const hook = texTestHook()
      const status = await get().refresh()
      const method = !status.installed
        ? null
        : hook
          ? (hook.uninstallMethod?.() ?? null)
          : isTauri
            ? await invoke<string | null>('tex_uninstall_method').catch(() => null)
            : null
      const removal = get().removal
      if (removal?.checking) set({ removal: { ...removal, method, checking: false } })
    })()
    return new Promise((resolve) => (turnOff = resolve))
  },

  answerTurnOff: async (proceed, remove) => {
    const removal = get().removal
    if (!removal || removal.busy) return
    const finish = (value: boolean) => {
      set({ removal: null })
      turnOff?.(value)
      turnOff = null
    }
    if (!proceed) return finish(false)
    // After a failed removal the button says "Turn off anyway": the plugin goes off, the TeX stays.
    if (!remove || removal.error || !removal.method) return finish(true)
    set({ removal: { ...removal, busy: true, error: null } })
    let stop = () => {}
    try {
      const hook = texTestHook()
      if (hook?.uninstall) set({ status: await hook.uninstall() })
      else {
        stop = await listen<InstallProgress>('tex-progress', () => {})
        set({ status: await invoke<TexStatus>('tex_uninstall') })
      }
      finish(true)
    } catch (err) {
      set({ removal: { ...removal, busy: false, error: String(err) } })
    } finally {
      stop()
    }
  },

  close: () => {
    // An install that's under way carries on; Settings › General shows how it's going.
    set({ offer: false })
    const status = get().status ?? NONE
    for (const resolve of closed) resolve(status)
    closed = []
  },
}))

/** What the install offer suggests on this system. */
export const INSTALL_CHOICES: { variant: InstallVariant; name: string; size: string; hint: string }[] = isMac
  ? [
      { variant: 'mactex', name: 'MacTeX', size: '6.9 GB', hint: 'All of TeX Live. Any document compiles.' },
      { variant: 'basictex', name: 'BasicTeX', size: '140 MB', hint: 'The common packages. Others need adding with tlmgr.' },
    ]
  : isWindows
    ? [{ variant: 'miktex', name: 'MiKTeX', size: '140 MB', hint: 'Installs packages as documents need them. Just for you, no administrator needed.' }]
    : isLinux
      ? [
          { variant: 'texlive', name: 'TeX Live', size: 'about 1 GB', hint: 'Installed with apt. What documents commonly need, with XeTeX and LuaTeX. Asks for your password.' },
          { variant: 'texlive-basic', name: 'TeX Live, basics', size: 'about 400 MB', hint: 'Installed with apt. Just the basic packages; add others with apt.' },
        ]
      : []

/**
 * Test builds can stand in for the app's TeX (window.__texFake), so the plugin can be driven in
 * a browser.
 */
type TexFake = Pick<TexService, 'status' | 'compile'> & { uninstallMethod?(): string | null; uninstall?(): Promise<TexStatus> }

export function texTestHook(): TexFake | undefined {
  if (!import.meta.env.DEV && !import.meta.env.VITE_TEST_HOOKS) return undefined
  return (window as { __texFake?: TexFake }).__texFake
}

/** Compiles one at a time per tab; asking again while one runs replaces the waiting request. */
export function texService(tabId: string, folder: () => string | null, job: () => string): TexService {
  let running: Promise<unknown> = Promise.resolve()
  // Compiles run after each pause in typing: count the first one per document.
  let counted = false
  return {
    status: () => useTex.getState().refresh(),
    install: async () => {
      const status = await useTex.getState().refresh()
      return status.installed ? status : useTex.getState().offerInstall()
    },
    compile: (source, { engine }) => {
      if (!counted) {
        counted = true
        track('feature_used', { feature: 'latex_compile', kind: 'latex' })
      }
      const next = running.then(async (): Promise<TexResult> => {
        const hook = texTestHook()
        if (hook) return hook.compile(source, { engine })
        const reply = await invoke<ArrayBuffer>('tex_compile', { key: tabId, source, dir: folder(), engine, job: job() })
        const bytes = new Uint8Array(reply)
        const data = new DataView(reply)
        const pdfLength = data.getUint32(0, true)
        const fileLength = data.getUint32(4 + pdfLength, true)
        const text = new TextDecoder()
        return {
          pdf: pdfLength ? bytes.slice(4, 4 + pdfLength) : null,
          file: text.decode(bytes.subarray(8 + pdfLength, 8 + pdfLength + fileLength)),
          log: text.decode(bytes.subarray(8 + pdfLength + fileLength)),
        }
      })
      running = next.catch(() => {})
      return next
    },
  }
}

export const forgetTexBuild = (tabId: string) => void (isTauri && invoke('tex_forget', { key: tabId }).catch(() => {}))
