import { useEffect, useRef, useState } from 'react'
import type { EditorProps } from '../App'
import { Button, CenterState } from '../components/ui'
import type { ViewMode } from '../components/ViewSwitch'
import { initialView, nextView, ViewSwitch } from '../components/ViewSwitch'
import { FindBar, Workbench } from '../components/Workbench'
import { recognizeImage, saveOcrCopy } from '../lib/convert'
import { KINDS } from '../lib/kinds'
import { getPackMeta } from '../lib/ocr/catalog'
import { useOcrPacks } from '../lib/ocr/packs'
import { isTauri, openExternal, saveBytesAs, shortcut, showError } from '../lib/platform'
import { installSettled, pluginBaseUrl } from '../state/plugins'
import { useSettings } from '../state/settings'
import { forgetTexBuild, texService, texTestHook } from '../state/tex'
import { registerEditor, useWorkspace } from '../state/workspace'
import type { ImageOcrService, PluginEditor as Instance, PdfView, PluginModule, SearchResult } from './api'
import { PLUGIN_API_VERSION } from './api'
import { pluginForKind } from './catalog'

const modules = new Map<string, Promise<PluginModule>>()

function imageOcrService(title: () => string): ImageOcrService {
  return {
    selectedLanguages: () => useOcrPacks.getState().selectedLangs.map((code) => getPackMeta(code)?.name ?? code),
    async recognize({ bytes, mime }, { signal, onProgress }) {
      const result = await recognizeImage(bytes, mime, signal, onProgress)
      const [{ ocrResultsToPlainText, ocrResultsToMarkdown, ocrResultsToDocx }, { pageText, searchPageText }] = await Promise.all([
        import('../lib/ocr/toDocx'),
        import('../editors/pdf/ocr'),
      ])
      signal?.throwIfAborted()
      const pages = [result.page]
      const { widthPx: width, heightPx: height } = result.page
      // Found the way a scanned PDF page is: the same folding of case, quotes and dashes.
      const words = pageText(result.page, { width, height })
      return {
        text: ocrResultsToPlainText(pages),
        markdown: ocrResultsToMarkdown(pages),
        width,
        height,
        find: (query) => searchPageText(words, query),
        saveDocx: async () => {
          const name = title()
          return saveOcrCopy(`${name} (OCR).docx`, await ocrResultsToDocx(pages, name))
        },
        saveSearchablePdf: async (buildSignal) => {
          const pdf = await result.buildSearchablePdf(buildSignal)
          buildSignal?.throwIfAborted()
          return saveOcrCopy(`${title()} (OCR).pdf`, pdf, buildSignal)
        },
      }
    },
  }
}

/** A plugin's module, loaded once per launch. */
function loadModule(kind: string): Promise<PluginModule> {
  const plugin = pluginForKind(kind)
  let module = modules.get(plugin.id)
  if (!module) {
    // An update downloading at launch replaces the files; load the new ones once it's done.
    module = installSettled(plugin.id)
      .then(() => import(/* @vite-ignore */ `${pluginBaseUrl(plugin)}index.js`))
      .then((m: PluginModule) => {
      if (m.apiVersion !== PLUGIN_API_VERSION) throw new Error(`This version of the ${plugin.name} plugin doesn't fit this version of Writan.`)
      return m
    })
    // A failed load (not downloaded yet, offline) can be tried again.
    module.catch(() => modules.delete(plugin.id))
    modules.set(plugin.id, module)
  }
  return module
}

/** Puts the plugin's sidebar element into Writan's sidebar while that is open. */
function SidebarSlot({ element }: { element: HTMLElement }) {
  const slot = useRef<HTMLDivElement>(null)
  useEffect(() => {
    slot.current!.appendChild(element)
    return () => element.remove()
  }, [element])
  return <div ref={slot} />
}

/** context.createPdfView: the PDF engine and viewer load the first time a plugin asks for one. */
function lazyPdfView(element: HTMLElement): PdfView {
  const view = import('./PdfPreview').then((m) => m.createPdfView(element))
  return {
    show: async (bytes) => (await view).show(bytes),
    zoom: (direction) => void view.then((v) => v.zoom(direction)),
    destroy: () => void view.then((v) => v.destroy()),
  }
}

/** The tab's file now (Save As can move it), unless it only lives in the browser's memory. */
function currentPath(tabId: string): string | null {
  const path = useWorkspace.getState().tabs.find((t) => t.id === tabId)?.path ?? null
  return path && !path.startsWith('browser:') ? path : null
}

const folderOf = (path: string) => path.slice(0, Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\')))

/** Hosts a plugin bundle's editor (Images, and any optional plugin): it renders into a plain element and talks through plugins/api.ts. */
export default function PluginEditor({ tab, active }: EditorProps) {
  const host = useRef<HTMLDivElement>(null)
  const instance = useRef<Instance | null>(null)
  /** `start`: the plugin didn't load. `open`: it did, but couldn't open this file. */
  const [error, setError] = useState<{ stage: 'start' | 'open'; message: string } | null>(null)
  const [ready, setReady] = useState(false)
  const [view, setView] = useState<ViewMode>(() => initialView(!!tab.bytes))
  const [findOpen, setFindOpen] = useState(false)
  const [found, setFound] = useState<SearchResult | null>(null)
  const findQuery = useRef('')
  const theme = useWorkspace((s) => s.theme)
  const plugin = pluginForKind(tab.kind)
  // Kinds with a sidebar get an element in it that outlives the sidebar being hidden.
  const [sidebar] = useState(() => (KINDS[tab.kind].panels.left ? document.createElement('div') : null))

  useEffect(() => {
    let cancelled = false
    let mounted: Instance | null = null
    let opened: string | null = null
    const tex = plugin.services?.includes('tex') && (isTauri || texTestHook())
      ? texService(
          tab.id,
          () => {
            const path = currentPath(tab.id)
            return path && folderOf(path)
          },
          () => useWorkspace.getState().tabs.find((t) => t.id === tab.id)?.title ?? tab.title,
        )
      : undefined
    void (async () => {
      let stage: 'start' | 'open' = 'start'
      try {
        const module = await loadModule(tab.kind)
        if (cancelled || !host.current) return
        stage = 'open'
        mounted = await module.mount(host.current, {
          bytes: tab.bytes,
          title: tab.title,
          theme: useWorkspace.getState().theme,
          onChange: () => useWorkspace.getState().markDirty(tab.id, true),
          openExternal: (url) => void openExternal(url),
          sidebar: sidebar ?? undefined,
          searchChanged: (result) => setFound(result),
          path: currentPath(tab.id),
          previewDelay: () => useSettings.getState().previewDelay,
          openDocument: ({ bytes, name }) => {
            opened = useWorkspace.getState().openUnsaved(bytes, name, opened)
          },
          updateDocument: ({ bytes }) => !!opened && useWorkspace.getState().refreshUnsaved(opened, bytes),
          saveFile: ({ bytes, name }) =>
            saveBytesAs(name, bytes).catch(async (err) => {
              await showError(err instanceof Error ? err.message : String(err))
              return false
            }),
          createPdfView: lazyPdfView,
          shortcut,
          imageOcr: plugin.id === 'images'
            ? imageOcrService(() => useWorkspace.getState().tabs.find((t) => t.id === tab.id)?.title ?? tab.title)
            : undefined,
          tex,
          print: async (content) => {
            const root = document.getElementById('print-root')!
            const page = document.createElement('div')
            page.className = 'print-page'
            page.appendChild(content)
            root.replaceChildren(page)
            window.print()
          },
        })
        if (cancelled) return mounted.destroy()
        instance.current = mounted
        mounted.setView?.(view)
        setReady(true)
      } catch (err) {
        if (!cancelled) setError({ stage, message: err instanceof Error ? err.message : String(err) })
      }
    })()
    return () => {
      cancelled = true
      mounted?.destroy()
      instance.current = null
      if (tex) forgetTexBuild(tab.id)
    }
    // The tab remounts with a new loadId (see EditorSlot) when its bytes change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    if (ready) instance.current?.setTheme?.(theme)
  }, [theme, ready])

  useEffect(() => {
    if (ready) instance.current?.setActive?.(active)
  }, [active, ready])

  useEffect(() => {
    if (ready) instance.current?.setView?.(view)
  }, [view, ready])

  useEffect(() => {
    if (!ready) return
    const it = instance.current!
    return registerEditor(tab.id, {
      getBytes: () => it.getBytes(),
      markSaved: it.markSaved && (() => it.markSaved!()),
      find: it.search
        ? () => {
            setFindOpen(true)
            // The bar comes back with its last query: show its matches again (and let a picture start reading).
            setFound(it.search!(findQuery.current, 0))
          }
        : it.find && (() => it.find!()),
      zoom: it.zoom && ((direction) => it.zoom!(direction)),
      print: it.print && (() => it.print!()),
      actions: it.recognizeText ? { 'ocr-document': () => it.recognizeText!() } : undefined,
      viewMode: it.setView && ((command) => setView((v) => nextView(v, command))),
    })
  }, [tab.id, ready])

  const search = (step: 1 | -1 | 0) => {
    const it = instance.current
    if (it?.search) setFound(it.search(findQuery.current, step))
  }

  if (error?.stage === 'open') {
    return (
      <CenterState title={`Couldn’t open “${tab.title}”`}>
        <p className="muted">{error.message}</p>
      </CenterState>
    )
  }
  if (error) {
    return (
      <CenterState title={`Couldn’t start ${plugin.name}`}>
        <p className="muted">{error.message}</p>
        <Button onClick={() => useSettings.getState().setOpen(true, 'general')}>Open Settings</Button>
      </CenterState>
    )
  }
  const editor = (
    <div className="plugin-editor">
      {/* The plugin owns this element and everything in it. */}
      <div className="plugin-editor-host" ref={host} />
      {!ready && (
        <div className="plugin-editor-loading">
          <CenterState spinner />
        </div>
      )}
      <FindBar
        open={findOpen}
        onClose={() => {
          setFindOpen(false)
          // The bar keeps the query for next time; the highlights go.
          instance.current?.search?.('', 0)
          setFound(null)
        }}
        onQuery={(query) => {
          findQuery.current = query
          search(0)
        }}
        onNext={() => search(1)}
        onPrev={() => search(-1)}
        count={found && !found.pending ? found.count : null}
        current={found && !found.pending ? Math.max(0, found.current) : null}
      />
      {ready && instance.current?.views && (
        <div className="canvas-bar">
          <ViewSwitch mode={view} setMode={setView} labels={instance.current.views} />
        </div>
      )}
    </div>
  )
  if (!sidebar) return editor
  return (
    <Workbench tab={tab} left={<SidebarSlot element={sidebar} />}>
      {editor}
    </Workbench>
  )
}
