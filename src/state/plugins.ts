// Which optional plugins are on, and installing them. Built-in plugins are always on.

import { create } from 'zustand'
import type { PluginInfo } from '../plugins/catalog'
import { PLUGINS, isBuiltIn, pluginById, pluginForKind } from '../plugins/catalog'
import { isTauri, isWindows } from '../lib/platform'
import { texTestHook, useTex } from './tex'
import { track } from './analytics'
import { pluginOf } from '../lib/analytics/events'

const KEY = 'workspace.plugins'
const RELEASES = 'https://github.com/elijahroizman/writan/releases/download'

export interface InstallProgress {
  received: number
  total: number
}

interface PluginsState {
  /** Optional plugins the user turned on. */
  enabled: Record<string, boolean>
  /** Downloaded plugins on disk: id → version. */
  installed: Record<string, string>
  /** Downloads in flight. */
  progress: Record<string, InstallProgress>
  errors: Record<string, string>
  /** Turn a plugin on, downloading it first if it needs that. Resolves false if that failed. */
  enable(id: string): Promise<boolean>
  disable(id: string): Promise<void>
}

function load(): Record<string, boolean> {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(KEY) || '{}')
    if (typeof parsed !== 'object' || parsed === null) return {}
    return Object.fromEntries(Object.entries(parsed).filter(([id, on]) => typeof on === 'boolean' && pluginById(id)))
  } catch {
    return {}
  }
}

const invoke = async <T>(cmd: string, args?: Record<string, unknown>) => (await import('@tauri-apps/api/core')).invoke<T>(cmd, args)

/** The archive a download plugin's version lives in: attached to the app release it shipped with. */
export function archiveUrl(plugin: PluginInfo) {
  if (plugin.source.type !== 'download') throw new Error(`${plugin.name} isn't downloaded`)
  return `${RELEASES}/plugin-${plugin.id}-${plugin.source.version}/${plugin.id}-${plugin.source.version}.zip`
}

/**
 * Downloaded plugins are served by the app's `wsplugin:` scheme. In a browser or a dev build
 * (`npm run plugins` builds them into public/plugins), they come from the page's own server.
 */
export function pluginBaseUrl(plugin: PluginInfo) {
  if (plugin.source.type !== 'download' || !isTauri || import.meta.env.DEV) return `${import.meta.env.BASE_URL}plugins/${plugin.id}/`
  return isWindows ? `http://wsplugin.localhost/${plugin.id}/` : `wsplugin://localhost/${plugin.id}/`
}

/** Downloaded plugins only need fetching in the app proper. */
const downloads = (plugin: PluginInfo) => plugin.source.type === 'download' && isTauri && !import.meta.env.DEV

export const usePlugins = create<PluginsState>((set, get) => {
  const save = () => localStorage.setItem(KEY, JSON.stringify(get().enabled))
  const setError = (id: string, message: string | null) => {
    const errors = { ...get().errors }
    if (message) errors[id] = message
    else delete errors[id]
    set({ errors })
  }

  return {
    enabled: load(),
    installed: {},
    progress: {},
    errors: {},

    enable: async (id) => {
      const plugin = pluginById(id)
      if (!plugin) return false
      if (plugin.source.type === 'download' && downloads(plugin) && get().installed[id] !== plugin.source.version) {
        if (get().progress[id]) return false
        if (!(await install(plugin))) return false
      }
      if (!get().enabled[id]) track('plugin_enabled', { plugin: pluginOf(id) })
      set({ enabled: { ...get().enabled, [id]: true } })
      save()
      if (plugin.services?.includes('tex') && (isTauri || texTestHook())) void offerTex()
      return true
    },

    disable: async (id) => {
      const plugin = pluginById(id)
      if (!plugin || isBuiltIn(plugin)) return
      if (get().enabled[id]) track('plugin_disabled', { plugin: pluginOf(id) })
      set({ enabled: { ...get().enabled, [id]: false } })
      save()
      if (downloads(plugin) && get().installed[id]) {
        // Downloaded files go too; turning it back on downloads it again.
        try {
          await invoke('remove_plugin', { id })
          const installed = { ...get().installed }
          delete installed[id]
          set({ installed })
        } catch (err) {
          setError(id, String(err))
        }
      }
    },
  }

  async function install(plugin: PluginInfo): Promise<boolean> {
    if (plugin.source.type !== 'download') return true
    const { id } = plugin
    setError(id, null)
    set({ progress: { ...get().progress, [id]: { received: 0, total: plugin.source.size } } })
    try {
      await invoke('install_plugin', { id, version: plugin.source.version, url: archiveUrl(plugin) })
      set({ installed: { ...get().installed, [id]: plugin.source.version } })
      return true
    } catch (err) {
      setError(id, String(err))
      return false
    } finally {
      const progress = { ...get().progress }
      delete progress[id]
      set({ progress })
    }
  }
})

/** The LaTeX plugin compiles with the system's TeX: offer to install it if there's none. */
async function offerTex() {
  const status = await useTex.getState().refresh()
  if (!status.installed) await useTex.getState().offerInstall()
}

/** Resolves once no download of this plugin is in flight (at once if none is). */
export function installSettled(id: string): Promise<void> {
  if (!usePlugins.getState().progress[id]) return Promise.resolve()
  return new Promise((resolve) => {
    const stop = usePlugins.subscribe((s) => {
      if (s.progress[id]) return
      stop()
      resolve()
    })
  })
}

/** Whether files of this kind open: its plugin is built in, or turned on. */
export function kindEnabled(kind: string, enabled = usePlugins.getState().enabled): boolean {
  const plugin = pluginForKind(kind)
  return !!plugin && (isBuiltIn(plugin) || !!enabled[plugin.id])
}

export function pluginEnabled(plugin: PluginInfo, enabled = usePlugins.getState().enabled): boolean {
  return isBuiltIn(plugin) || !!enabled[plugin.id]
}

/** Kinds whose plugin is on, in catalog order. */
export function enabledKinds(enabled = usePlugins.getState().enabled) {
  return PLUGINS.filter((p) => pluginEnabled(p, enabled)).flatMap((p) => p.kinds)
}

/**
 * At launch: learn what's on disk, delete anything this build doesn't offer as a download (a
 * plugin that was removed), and bring downloaded plugins up to the version this build wants
 * (an app update can come with a new plugin version). Until it arrives, a plugin that is still
 * on disk keeps working.
 */
export async function startPlugins() {
  if (!isTauri) return
  const { listen } = await import('@tauri-apps/api/event')
  void listen<{ id: string; received: number; total: number }>('plugin-progress', ({ payload }) => {
    const { progress } = usePlugins.getState()
    if (!progress[payload.id]) return
    usePlugins.setState({ progress: { ...progress, [payload.id]: { received: payload.received, total: payload.total || progress[payload.id].total } } })
  })
  try {
    const list = await invoke<{ id: string; version: string }[]>('installed_plugins')
    const downloadIds = new Set(PLUGINS.filter((p) => p.source.type === 'download').map((p) => p.id))
    const installed: Record<string, string> = {}
    for (const plugin of list) {
      if (downloadIds.has(plugin.id)) {
        installed[plugin.id] = plugin.version
        continue
      }
      // Files from a plugin this build no longer downloads. A failure is tried again next launch.
      try {
        await invoke('remove_plugin', { id: plugin.id })
      } catch {
        /* leave it off `installed`; installed_plugins will list it again */
      }
    }
    usePlugins.setState({ installed })
  } catch {
    return
  }
  const { enabled, installed, enable } = usePlugins.getState()
  for (const plugin of PLUGINS) {
    if (plugin.source.type === 'download' && enabled[plugin.id] && downloads(plugin) && installed[plugin.id] !== plugin.source.version) {
      void enable(plugin.id)
    }
  }
}

export function formatSize(bytes: number) {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`
  return bytes >= 1e6 ? `${Math.round(bytes / 1e6)} MB` : `${Math.max(1, Math.round(bytes / 1e3))} KB`
}
