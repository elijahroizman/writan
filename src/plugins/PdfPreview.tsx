// A read-only PDF view a plugin can put in its own element (context.createPdfView), drawn by
// the same PDFium engine and viewer as PDF tabs. Showing a new PDF keeps the place and zoom, so
// a live preview (LaTeX) doesn't jump while you type.

import { forwardRef, useCallback, useImperativeHandle, useRef, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { PdfEngine } from '../editors/pdf/engine/client'
import type { PageInfo } from '../editors/pdf/engine/protocol'
import { DocStore } from '../editors/pdf/engine/store'
import type { PageSlot, ViewerHandle } from '../editors/pdf/viewer/Viewer'
import { Viewer } from '../editors/pdf/viewer/Viewer'
import { TextLayer } from '../editors/pdf/viewer/layers'
import '../editors/pdf/pdf.css'
import type { PdfView } from './api'

interface PreviewHandle {
  show(bytes: Uint8Array): Promise<void>
  zoom(direction: 'in' | 'out' | 'reset'): void
  destroy(): void
}

const Preview = forwardRef<PreviewHandle>(function Preview(_, ref) {
  const [store, setStore] = useState<DocStore | null>(null)
  const [pages, setPages] = useState<PageInfo[]>([])
  const storeRef = useRef<DocStore | null>(null)
  const starting = useRef<Promise<DocStore> | null>(null)
  const viewer = useRef<ViewerHandle>(null)
  const destroyed = useRef(false)

  useImperativeHandle(ref, () => ({
    async show(bytes) {
      if (destroyed.current) return
      starting.current ??= PdfEngine.create().then((engine) => {
        const s = new DocStore(engine)
        s.subscribe(() => setPages(s.pages))
        storeRef.current = s
        return s
      })
      const s = await starting.current
      if (destroyed.current) return
      const result = await s.engine.call('open', [bytes.slice().buffer, ''])
      if (destroyed.current) return
      s.replaced(result.pages)
      setStore(s)
    },
    zoom(direction) {
      if (direction === 'reset') viewer.current?.setZoom('auto')
      else viewer.current?.step(direction === 'in' ? 1 : -1)
    },
    destroy() {
      destroyed.current = true
      // The store may still be starting; destroy it once it exists.
      void starting.current?.then((s) => s.destroy())
    },
  }))

  const noop = useCallback(() => {}, [])
  // Keyed by the page's revision: a new PDF can reuse a page's key, and its text must be read again.
  const renderPage = useCallback(({ index, page }: PageSlot) => storeRef.current && <TextLayer key={storeRef.current.rev(page.key)} store={storeRef.current} index={index} page={page} />, [])

  return (
    <div className="pdf-stage tool-select pdf-preview">
      {store && <Viewer ref={viewer} store={store} pages={pages} awake onPageChange={noop} onZoomChange={noop} renderPage={renderPage} />}
    </div>
  )
})

export function createPdfView(element: HTMLElement): PdfView {
  const root = createRoot(element)
  let handle: PreviewHandle | null = null
  const ready = new Promise<PreviewHandle>((resolve) => {
    root.render(
      <Preview
        ref={(h) => {
          handle = h
          if (h) resolve(h)
        }}
      />,
    )
  })
  let closed = false
  return {
    show: async (bytes) => {
      if (!closed) await (await ready).show(bytes)
    },
    zoom: (direction) => handle?.zoom(direction),
    destroy: () => {
      if (closed) return
      closed = true
      handle?.destroy()
      // Unmounting while the plugin's own React renders would warn; let that finish first.
      queueMicrotask(() => root.unmount())
    },
  }
}
