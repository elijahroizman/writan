import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import { startAppIcon } from './lib/app-icon'
import { startAppearance } from './state/appearance'
import { getEditor, useWorkspace } from './state/workspace'
import { useCollab } from './collab/session'
// Inter, the UI font where the system has no San Francisco (Windows, Linux). Only declares the
// faces; the files load only where the font stack reaches Inter, so never on a Mac.
import '@fontsource-variable/inter/opsz.css'
import './styles/app.css'

startAppearance()
startAppIcon()

// Development helper: http://localhost:1420/?open=/dev-samples/Handbook.pdf opens files without a dialog.
// Also available in test builds (VITE_TEST_HOOKS=1 npm run build) for automated UI checks.
if (import.meta.env.DEV || import.meta.env.VITE_TEST_HOOKS) {
  const paths = new URLSearchParams(location.search).getAll('open')
  if (paths.length) setTimeout(() => void useWorkspace.getState().openPaths(paths), 300)
  Object.assign(window, { __workspace: { useWorkspace, getEditor }, __collab: { useCollab } })
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
