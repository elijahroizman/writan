import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import { startAppIcon } from './lib/app-icon'
import { startAppearance } from './state/appearance'
import { getEditor, useWorkspace } from './state/workspace'
import { useChat, useCollab } from './collab/session'
import { useAi } from './state/ai'
import { isTauri, isWindows } from './lib/platform'
import { stopWindowZoom } from './lib/pinch'
// Inter, the UI font where the system has no San Francisco (Windows, Linux). Only declares the
// faces; the files load only where the font stack reaches Inter, so never on a Mac.
import '@fontsource-variable/inter/opsz.css'
import './styles/app.css'

startAppearance()
startAppIcon()
if (isTauri && isWindows) stopWindowZoom()

// Development helper: http://localhost:1420/?open=/dev-samples/Handbook.pdf opens files without a dialog.
// Also available in test builds (VITE_TEST_HOOKS=1 npm run build) for automated UI checks.
if (import.meta.env.DEV || import.meta.env.VITE_TEST_HOOKS) {
  const paths = new URLSearchParams(location.search).getAll('open')
  if (paths.length) setTimeout(() => void useWorkspace.getState().openPaths(paths), 300)
  Object.assign(window, {
    __workspace: { useWorkspace, getEditor },
    __collab: { useCollab, useChat },
    __ai: { useAi },
    __loadPdfOcrTestApi: async () => {
      const [pdfOcr, packs, toDocx, exporter, pdfClient, ocrClient, preprocess, catalog, convert, pdfEngine] =
        await Promise.all([
          import('./lib/ocr/pdfOcr'),
          import('./lib/ocr/packs'),
          import('./lib/ocr/toDocx'),
          import('./lib/pdf/exporter'),
          import('./lib/pdf/client'),
          import('./lib/ocr/client'),
          import('./lib/ocr/preprocess'),
          import('./lib/ocr/catalog'),
          import('./lib/convert'),
          import('./editors/pdf/engine/client'),
        ])
      return {
        pdfOcr,
        packs,
        toDocx,
        exporter,
        pdfClient,
        ocrClient,
        preprocess,
        catalog,
        convert,
        PdfEngine: pdfEngine.PdfEngine,
      }
    },
  })
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
