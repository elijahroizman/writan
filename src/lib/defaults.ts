// Making Writan the app that opens each file type (src-tauri/src/defaults.rs). macOS sets it
// straight away; Windows only lets the user choose, so it opens Settings › Default apps.

import type { DocKind } from './kinds'
import { KINDS } from './kinds'
import { isMac, isTauri, isWindows } from './platform'

export const canSetDefaults = isTauri && (isMac || isWindows)

const invoke = async <T>(cmd: string, args?: Record<string, unknown>) => (await import('@tauri-apps/api/core')).invoke<T>(cmd, args)

export interface DefaultExtensionStatus {
  ext: string
  isDefault: boolean | null
}

/**
 * Combine per-extension checks into a per-kind status. Unregistered secondary extensions
 * (e.g. `.xlsm` or `.tsv` when only `.xlsx` / `.csv` are registered) don't mark the whole
 * kind unknown when its registered extensions are known.
 */
export function resolveKindDefaults(kinds: DocKind[], list: DefaultExtensionStatus[]): Record<DocKind, boolean | null> {
  const byExt = new Map(list.map((s) => [s.ext.toLowerCase(), s.isDefault]))
  return Object.fromEntries(
    kinds.map((k) => {
      const exts = KINDS[k]?.extensions ?? []
      const each = exts.map((e) => byExt.get(e.toLowerCase()) ?? null)
      const known = each.filter((v): v is boolean => v !== null)
      return [k, known.length === 0 ? null : known.every(Boolean)]
    }),
  )
}

/** For each kind: whether Writan opens it now (null if the system can't say). */
export async function defaultStatus(kinds: DocKind[]): Promise<Record<DocKind, boolean | null>> {
  if (!canSetDefaults) return Object.fromEntries(kinds.map((k) => [k, null]))
  // A kind is Writan's when every one of its checkable extensions is.
  const exts = kinds.flatMap((k) => KINDS[k]?.extensions ?? [])
  const list = await invoke<DefaultExtensionStatus[]>('default_apps', { exts })
  if (!Array.isArray(list)) throw new Error('Unexpected response while checking default apps.')
  return resolveKindDefaults(kinds, list)
}

/** Resolves true when Windows Settings opened for the user to finish there. */
export async function makeDefault(kinds: DocKind[]): Promise<boolean> {
  const exts = kinds.flatMap((k) => KINDS[k]?.extensions ?? [])
  const result = await invoke<{ openedSettings: boolean }>('set_default_apps', { exts })
  return !!result?.openedSettings
}

/** Opens Windows Settings › Default apps for Writan without requiring prior detection to succeed. */
export async function openDefaultAppsSettings(): Promise<boolean> {
  return makeDefault([])
}
