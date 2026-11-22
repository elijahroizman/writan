import { useCallback, useEffect, useRef, useState } from 'react'
import { SheetsEngine, type ChartView, type RangeCellSnapshot, type RangeSnapshot } from '../engine/sheets/engine'
import { readFile } from '../lib/platform'
import { renderChartToPng } from './chartImage'

export const EXCEL_LINK_PREFIX = 'writan:excel-link:'
export const EXCEL_LINKS_PART = 'writan/excel-links.json'
export const EXCEL_LINK_CLIPBOARD = 'application/x-writan-excel-link'

export interface ExcelLink {
  id: string
  kind: 'range' | 'chart'
  workbookName: string
  workbookPath: string | null
  workbookTabId?: string
  sheet: string
  range?: string
  chartIndex?: number
  chartTitle?: string
  autoUpdate: boolean
  lastUpdatedAt: number
}

export interface ExcelClipboardLink {
  workbookTabId: string
  workbookName: string
  workbookPath: string | null
  sheet: string
  range: string
}

export interface OpenWorkbook {
  tabId: string
  name: string
  path: string | null
  engine: SheetsEngine
}

export function linkTagFor(id: string): string {
  return `${EXCEL_LINK_PREFIX}${id}`
}

export function linkIdFromTag(tag: string | null | undefined): string | null {
  if (!tag || !tag.startsWith(EXCEL_LINK_PREFIX)) return null
  return tag.slice(EXCEL_LINK_PREFIX.length)
}

export function colName(col: number): string {
  let n = col + 1
  let out = ''
  while (n > 0) {
    n -= 1
    out = String.fromCharCode(65 + (n % 26)) + out
    n = Math.floor(n / 26)
  }
  return out
}

export function selectionToA1(anchor: [number, number], focus: [number, number]): string {
  const r0 = Math.min(anchor[0], focus[0]) + 1
  const c0 = Math.min(anchor[1], focus[1])
  const r1 = Math.max(anchor[0], focus[0]) + 1
  const c1 = Math.max(anchor[1], focus[1])
  const a = `${colName(c0)}${r0}`
  const b = `${colName(c1)}${r1}`
  return a === b ? a : `${a}:${b}`
}

// ---- Live open-workbook registry ----

interface RegistryEntry {
  workbook: OpenWorkbook
  revision: number
  unsub: () => void
}

const registry = new Map<string, RegistryEntry>()
const workbookListeners = new Set<(changedTabId?: string) => void>()

function notifyWorkbookListeners(changedTabId?: string) {
  for (const fn of workbookListeners) fn(changedTabId)
}

export function registerOpenWorkbook(workbook: OpenWorkbook): () => void {
  const prev = registry.get(workbook.tabId)
  if (prev) prev.unsub()
  const entry: RegistryEntry = {
    workbook,
    revision: workbook.engine.revision,
    unsub: () => {},
  }
  entry.unsub = workbook.engine.subscribe(() => {
    if (workbook.engine.revision !== entry.revision) {
      entry.revision = workbook.engine.revision
      notifyWorkbookListeners(workbook.tabId)
    }
  })
  registry.set(workbook.tabId, entry)
  notifyWorkbookListeners(workbook.tabId)
  return () => {
    const cur = registry.get(workbook.tabId)
    if (cur === entry) {
      cur.unsub()
      registry.delete(workbook.tabId)
      notifyWorkbookListeners()
    }
  }
}

export function updateOpenWorkbookMeta(tabId: string, name: string, path: string | null) {
  const entry = registry.get(tabId)
  if (!entry) return
  entry.workbook = { ...entry.workbook, name, path }
  notifyWorkbookListeners()
}

export function listOpenWorkbooks(): OpenWorkbook[] {
  return [...registry.values()].map((e) => e.workbook)
}

export function subscribeOpenWorkbooks(fn: (changedTabId?: string) => void): () => void {
  workbookListeners.add(fn)
  return () => {
    workbookListeners.delete(fn)
  }
}

export function findOpenWorkbook(link: Pick<ExcelLink, 'workbookPath' | 'workbookTabId' | 'workbookName'>): OpenWorkbook | null {
  const all = listOpenWorkbooks()
  if (link.workbookPath) {
    const byPath = all.find((w) => w.path === link.workbookPath)
    if (byPath) return byPath
    if (link.workbookTabId) {
      const byTab = all.find((w) => w.tabId === link.workbookTabId && w.path === link.workbookPath)
      if (byTab) return byTab
    }
    return null
  }
  if (link.workbookTabId) {
    const byTab = all.find((w) => w.tabId === link.workbookTabId)
    if (byTab) return byTab
  }
  return all.find((w) => !w.path && w.name === link.workbookName) ?? all.find((w) => w.name === link.workbookName) ?? null
}

// ---- Snapshot extraction (live workbook or disk fallback) ----

export interface ResolvedLinkData {
  kind: 'range' | 'chart'
  range?: RangeSnapshot
  chart?: { bytes: Uint8Array; width: number; height: number; title: string }
  resolvedSheet: string
  resolvedRange?: string
}

async function withWorkbookEngine<T>(
  link: Pick<ExcelLink, 'workbookPath' | 'workbookTabId' | 'workbookName'>,
  fn: (engine: SheetsEngine) => Promise<T>,
): Promise<T | null> {
  const open = findOpenWorkbook(link)
  if (open) return fn(open.engine)
  if (!link.workbookPath) return null
  try {
    const bytes = await readFile(link.workbookPath)
    const engine = await SheetsEngine.open(bytes, link.workbookName)
    try {
      return await fn(engine)
    } finally {
      engine.dispose()
    }
  } catch {
    return null
  }
}

function findChartOnSheet(engine: SheetsEngine, sheetIdx: number, chartIndex?: number, chartTitle?: string): ChartView | null {
  const charts = engine.chartsOf(sheetIdx)
  if (!charts.length) return null
  if (chartTitle) {
    if (chartIndex != null && charts[chartIndex]?.title === chartTitle) return charts[chartIndex]
    return charts.find((c) => c.title === chartTitle) ?? null
  }
  if (chartIndex != null) return charts[chartIndex] ?? null
  return charts[0] ?? null
}

export async function resolveExcelLink(link: ExcelLink): Promise<ResolvedLinkData | null> {
  return withWorkbookEngine(link, async (engine) => {
    const sheetIdx = engine.sheets.findIndex((s) => s.name === link.sheet)
    if (sheetIdx < 0) return null
    const resolvedSheet = engine.sheets[sheetIdx].name
    if (link.kind === 'range') {
      const snap = engine.rangeSnapshot(sheetIdx, link.range || 'A1')
      if (!snap || !snap.rows.length) return null
      return { kind: 'range', range: snap, resolvedSheet: snap.sheet, resolvedRange: snap.range }
    }
    const chart = findChartOnSheet(engine, sheetIdx, link.chartIndex, link.chartTitle)
    if (!chart) return null
    const rendered = await renderChartToPng(chart)
    return {
      kind: 'chart',
      chart: { ...rendered, title: chart.title ?? link.chartTitle ?? 'Linked Chart' },
      resolvedSheet,
    }
  })
}

// ---- OOXML zip persistence (`writan/excel-links.json`) ----

export async function readExcelLinks(bytes: Uint8Array | null): Promise<ExcelLink[]> {
  if (!bytes || bytes.length < 4) return []
  try {
    const { default: JSZip } = await import('jszip')
    const zip = await JSZip.loadAsync(bytes)
    const file = zip.file(EXCEL_LINKS_PART)
    if (!file) return []
    const parsed = JSON.parse(await file.async('string')) as { version?: number; links?: ExcelLink[] }
    if (!Array.isArray(parsed?.links)) return []
    return parsed.links.filter((l) => l && typeof l.id === 'string' && (l.kind === 'range' || l.kind === 'chart'))
  } catch {
    return []
  }
}

export async function writeExcelLinks(bytes: Uint8Array, links: ExcelLink[]): Promise<Uint8Array> {
  const { default: JSZip } = await import('jszip')
  const zip = await JSZip.loadAsync(bytes)
  const existing = zip.file(EXCEL_LINKS_PART)
  if (!links.length) {
    if (!existing) return bytes
    zip.remove(EXCEL_LINKS_PART)
    return zip.generateAsync({ type: 'uint8array', compression: 'DEFLATE' })
  }
  // Strip runtime-only workbookTabId before persisting? Keep workbookPath & workbookName;
  // tabId is harmless and ignored on fresh sessions where tab ids differ.
  zip.file(EXCEL_LINKS_PART, JSON.stringify({ version: 1, links }, null, 2))
  // Ensure [Content_Types].xml declares .json so Microsoft Office opens the package without repair prompts.
  const ctFile = zip.file('[Content_Types].xml')
  if (ctFile) {
    const ct = await ctFile.async('string')
    if (!/Extension\s*=\s*"json"/i.test(ct)) {
      const updated = ct.replace(
        /(<Types\b[^>]*>)/,
        '$1<Default Extension="json" ContentType="application/json"/>',
      )
      zip.file('[Content_Types].xml', updated)
    }
  }
  return zip.generateAsync({ type: 'uint8array', compression: 'DEFLATE' })
}

// ---- Shared React hook for Word and Slides editors ----

export interface LinkedDocumentTarget {
  insertLinkedTable(tag: string, rows: RangeCellSnapshot[][], name: string): void
  updateLinkedTable(tag: string, rows: RangeCellSnapshot[][]): boolean
  insertLinkedImage(tag: string, bytes: Uint8Array, contentType: string, width: number, height: number, name: string): void
  updateLinkedImage(tag: string, bytes: Uint8Array, contentType: string): boolean
  clearLinkTag(tag: string): boolean
  linkedTags(): string[]
}

export interface ExcelLinksController {
  links: ExcelLink[]
  openWorkbooks: OpenWorkbook[]
  updating: boolean
  insertRange(source: { workbook: OpenWorkbook; sheet: string; range: string; autoUpdate?: boolean }): Promise<ExcelLink | null>
  insertChart(source: { workbook: OpenWorkbook; sheet: string; chartIndex: number; autoUpdate?: boolean }): Promise<ExcelLink | null>
  insertFromClipboard(clip: ExcelClipboardLink): Promise<ExcelLink | null>
  updateLink(id: string): Promise<boolean>
  updateAll(): Promise<number>
  unlink(id: string): void
  setAutoUpdate(id: string, autoUpdate: boolean): void
  getByTag(tag: string | null | undefined): ExcelLink | null
  writeToBytes(bytes: Uint8Array): Promise<Uint8Array>
}

function makeLinkId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
}

export function useExcelLinks(
  target: LinkedDocumentTarget | null,
  initialBytes: Uint8Array | null,
  onMutated?: () => void,
): ExcelLinksController {
  const [links, setLinks] = useState<ExcelLink[]>([])
  const [openWorkbooks, setOpenWorkbooks] = useState<OpenWorkbook[]>(() => listOpenWorkbooks())
  const [updating, setUpdating] = useState(false)
  const linksRef = useRef<ExcelLink[]>([])
  const targetRef = useRef<LinkedDocumentTarget | null>(target)
  const onMutatedRef = useRef(onMutated)
  linksRef.current = links
  targetRef.current = target
  onMutatedRef.current = onMutated

  // Load persisted links from the opened file bytes.
  useEffect(() => {
    let dead = false
    readExcelLinks(initialBytes).then((loaded) => {
      if (dead) return
      setLinks(loaded)
    })
    return () => {
      dead = true
    }
  }, [initialBytes])

  const applyResolved = useCallback((link: ExcelLink, data: ResolvedLinkData): boolean => {
    const t = targetRef.current
    if (!t) return false
    const tag = linkTagFor(link.id)
    if (data.kind === 'range' && data.range) {
      return t.updateLinkedTable(tag, data.range.rows)
    }
    if (data.kind === 'chart' && data.chart) {
      return t.updateLinkedImage(tag, data.chart.bytes, 'image/png')
    }
    return false
  }, [])

  const updateLink = useCallback(
    async (id: string): Promise<boolean> => {
      const link = linksRef.current.find((l) => l.id === id)
      if (!link || !targetRef.current) return false
      setUpdating(true)
      try {
        const resolved = await resolveExcelLink(link)
        if (!resolved) return false
        const ok = applyResolved(link, resolved)
        if (ok) {
          const now = Date.now()
          setLinks((prev) =>
            prev.map((l) =>
              l.id === id
                ? { ...l, lastUpdatedAt: now, sheet: resolved.resolvedSheet, range: resolved.resolvedRange ?? l.range }
                : l,
            ),
          )
          onMutatedRef.current?.()
        }
        return ok
      } finally {
        setUpdating(false)
      }
    },
    [applyResolved],
  )

  const updateAll = useCallback(async (): Promise<number> => {
    if (!targetRef.current || !linksRef.current.length) return 0
    setUpdating(true)
    let count = 0
    try {
      const presentTags = new Set(targetRef.current.linkedTags())
      const activeLinks = linksRef.current.filter((l) => presentTags.has(linkTagFor(l.id)))
      const updates = new Map<string, { now: number; sheet: string; range?: string }>()
      for (const link of activeLinks) {
        const resolved = await resolveExcelLink(link)
        if (resolved && applyResolved(link, resolved)) {
          count++
          updates.set(link.id, { now: Date.now(), sheet: resolved.resolvedSheet, range: resolved.resolvedRange })
        }
      }
      if (updates.size > 0 || activeLinks.length !== linksRef.current.length) {
        setLinks((prev) =>
          prev
            .filter((l) => presentTags.has(linkTagFor(l.id)))
            .map((l) => {
              const u = updates.get(l.id)
              return u ? { ...l, lastUpdatedAt: u.now, sheet: u.sheet, range: u.range ?? l.range } : l
            }),
        )
        if (count > 0) onMutatedRef.current?.()
      }
      return count
    } finally {
      setUpdating(false)
    }
  }, [applyResolved])

  // Subscribe to open workbook changes and auto-update matching links (debounced 120ms).
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null
    const pendingTabs = new Set<string>()
    const flush = async () => {
      timer = null
      const tabs = [...pendingTabs]
      pendingTabs.clear()
      const t = targetRef.current
      if (!t || !linksRef.current.length) return
      const matching = linksRef.current.filter((l) => {
        if (!l.autoUpdate) return false
        const open = findOpenWorkbook(l)
        return open != null && (tabs.length === 0 || tabs.includes(open.tabId))
      })
      if (!matching.length) return
      let changed = false
      const updated = new Map<string, number>()
      for (const link of matching) {
        const resolved = await resolveExcelLink(link)
        if (resolved && applyResolved(link, resolved)) {
          changed = true
          updated.set(link.id, Date.now())
        }
      }
      if (changed) {
        setLinks((prev) => prev.map((l) => (updated.has(l.id) ? { ...l, lastUpdatedAt: updated.get(l.id)! } : l)))
        onMutatedRef.current?.()
      }
    }
    return subscribeOpenWorkbooks((changedTabId) => {
      setOpenWorkbooks(listOpenWorkbooks())
      if (changedTabId) {
        pendingTabs.add(changedTabId)
        if (timer) clearTimeout(timer)
        timer = setTimeout(() => {
          void flush()
        }, 120)
      }
    })
  }, [applyResolved])

  const insertRange = useCallback(
    async ({
      workbook,
      sheet,
      range,
      autoUpdate = true,
    }: {
      workbook: OpenWorkbook
      sheet: string
      range: string
      autoUpdate?: boolean
    }): Promise<ExcelLink | null> => {
      const t = targetRef.current
      if (!t) return null
      const sheetIdx = Math.max(
        0,
        workbook.engine.sheets.findIndex((s) => s.name === sheet),
      )
      const snap = workbook.engine.rangeSnapshot(sheetIdx, range)
      if (!snap || !snap.rows.length) return null
      const id = makeLinkId()
      const link: ExcelLink = {
        id,
        kind: 'range',
        workbookName: workbook.name,
        workbookPath: workbook.path,
        workbookTabId: workbook.tabId,
        sheet: snap.sheet,
        range: snap.range,
        autoUpdate,
        lastUpdatedAt: Date.now(),
      }
      t.insertLinkedTable(linkTagFor(id), snap.rows, `${workbook.name} · ${snap.sheet}!${snap.range}`)
      setLinks((prev) => [...prev, link])
      onMutatedRef.current?.()
      return link
    },
    [],
  )

  const insertChart = useCallback(
    async ({
      workbook,
      sheet,
      chartIndex,
      autoUpdate = true,
    }: {
      workbook: OpenWorkbook
      sheet: string
      chartIndex: number
      autoUpdate?: boolean
    }): Promise<ExcelLink | null> => {
      const t = targetRef.current
      if (!t) return null
      const sheetIdx = Math.max(
        0,
        workbook.engine.sheets.findIndex((s) => s.name === sheet),
      )
      const chart = workbook.engine.chartsOf(sheetIdx)[chartIndex]
      if (!chart) return null
      const rendered = await renderChartToPng(chart)
      const id = makeLinkId()
      const title = chart.title || `Chart ${chartIndex + 1}`
      const link: ExcelLink = {
        id,
        kind: 'chart',
        workbookName: workbook.name,
        workbookPath: workbook.path,
        workbookTabId: workbook.tabId,
        sheet: workbook.engine.sheets[sheetIdx]?.name ?? sheet,
        chartIndex,
        chartTitle: title,
        autoUpdate,
        lastUpdatedAt: Date.now(),
      }
      t.insertLinkedImage(
        linkTagFor(id),
        rendered.bytes,
        'image/png',
        rendered.width,
        rendered.height,
        `${workbook.name} · ${title}`,
      )
      setLinks((prev) => [...prev, link])
      onMutatedRef.current?.()
      return link
    },
    [],
  )

  const insertFromClipboard = useCallback(
    async (clip: ExcelClipboardLink): Promise<ExcelLink | null> => {
      const wb = findOpenWorkbook(clip)
      if (!wb) return null
      return insertRange({ workbook: wb, sheet: clip.sheet, range: clip.range, autoUpdate: true })
    },
    [insertRange],
  )

  const unlink = useCallback((id: string) => {
    targetRef.current?.clearLinkTag(linkTagFor(id))
    setLinks((prev) => prev.filter((l) => l.id !== id))
    onMutatedRef.current?.()
  }, [])

  const setAutoUpdate = useCallback((id: string, autoUpdate: boolean) => {
    setLinks((prev) => prev.map((l) => (l.id === id ? { ...l, autoUpdate } : l)))
    onMutatedRef.current?.()
  }, [])

  const getByTag = useCallback((tag: string | null | undefined): ExcelLink | null => {
    const id = linkIdFromTag(tag)
    if (!id) return null
    return linksRef.current.find((l) => l.id === id) ?? null
  }, [])

  const writeToBytes = useCallback(async (bytes: Uint8Array): Promise<Uint8Array> => {
    const t = targetRef.current
    const current = linksRef.current
    if (!t) return writeExcelLinks(bytes, current)
    const liveTags = new Set(t.linkedTags())
    const kept = current.filter((l) => liveTags.has(linkTagFor(l.id)))
    return writeExcelLinks(bytes, kept)
  }, [])

  return {
    links,
    openWorkbooks,
    updating,
    insertRange,
    insertChart,
    insertFromClipboard,
    updateLink,
    updateAll,
    unlink,
    setAutoUpdate,
    getByTag,
    writeToBytes,
  }
}
