// Offered when the LaTeX plugin needs TeX and the system has none.

import { useEffect, useState } from 'react'
import { formatSize } from '../state/plugins'
import { INSTALL_CHOICES, useTex } from '../state/tex'
import { Button, Checkbox, Modal, Spinner } from './ui'

export function TexInstall() {
  const installing = useTex((s) => s.installing)
  const error = useTex((s) => s.error)
  const { install, close } = useTex.getState()

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.preventDefault()
      e.stopPropagation()
      close()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [close])

  const stage =
    installing?.stage === 'download'
      ? installing.total
        ? `Downloading… ${formatSize(installing.received)} of ${formatSize(installing.total)}`
        : 'Downloading…'
      : installing?.stage === 'verify'
        ? 'Checking the installer’s signature…'
        : installing
          ? 'Installing… this can take several minutes.'
          : null

  return (
    <Modal placement="top" dim="light" layer="dialog" onClose={close}>
      <div className="tex-install" role="dialog" aria-modal="true" aria-labelledby="tex-install-title">
        <h2 id="tex-install-title">Install TeX for LaTeX documents?</h2>
        <p className="muted">
          Writan compiles LaTeX with the TeX on your computer, and it doesn’t have one yet. You can install it now, or later from Settings › General.
        </p>
        {INSTALL_CHOICES.length === 0 && <p className="muted">Install TeX Live with your system’s package manager, then reopen the document.</p>}
        {!installing && (
          <div className="tex-install-choices">
            {INSTALL_CHOICES.map((c, i) => (
              <button key={c.variant} type="button" className={`tex-install-choice ${i === 0 ? 'primary' : ''}`} onClick={() => void install(c.variant)}>
                <span className="settings-label">
                  {c.name} <span className="faint">· {c.size}</span>
                </span>
                <span className="settings-hint">{c.hint}</span>
              </button>
            ))}
          </div>
        )}
        {installing && (
          <div className="tex-install-progress">
            <Spinner size={16} />
            <span>{stage}</span>
          </div>
        )}
        {installing?.stage === 'download' && installing.total > 0 && (
          <span className="plugin-progress" role="progressbar" aria-valuemin={0} aria-valuemax={installing.total} aria-valuenow={installing.received}>
            <span style={{ width: `${Math.min(100, (100 * installing.received) / installing.total)}%` }} />
          </span>
        )}
        {error && <p className="plugin-error">{error}</p>}
        <div className="tex-install-actions">
          <Button onClick={close}>{installing ? 'Hide' : error ? 'Close' : 'Not now'}</Button>
        </div>
      </div>
    </Modal>
  )
}

/** Asked when LaTeX is turned off and Writan could also take TeX off the computer. */
export function TexRemove() {
  const removal = useTex((s) => s.removal)
  const { answerTurnOff } = useTex.getState()
  const [remove, setRemove] = useState(false)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.preventDefault()
      e.stopPropagation()
      void answerTurnOff(false, false)
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [answerTurnOff])

  if (!removal) return null
  return (
    <Modal placement="top" dim="light" layer="dialog" onClose={() => void answerTurnOff(false, false)}>
      <div className="tex-install" role="dialog" aria-modal="true" aria-labelledby="tex-remove-title">
        <h2 id="tex-remove-title">Turn off LaTeX?</h2>
        <p className="muted">
          LaTeX files stop opening in Writan. The TeX on this computer isn’t part of Writan, so it stays
          {removal.method ? ' unless you remove it too.' : '.'}
        </p>
        {removal.checking && (
          <div className="tex-install-progress settings-hint">
            <Spinner size={14} />
            <span>Checking how TeX was installed…</span>
          </div>
        )}
        {removal.method && (
          <>
            <Checkbox checked={remove} disabled={removal.busy || !!removal.error} onChange={setRemove} label="Also remove TeX from this computer" />
            <p className="settings-hint">Uninstalls it with {removal.method}. Other programs that use TeX lose it too, and putting it back means downloading it again.</p>
          </>
        )}
        {removal.busy && (
          <div className="tex-install-progress">
            <Spinner size={16} />
            <span>Removing TeX… this can take a few minutes.</span>
          </div>
        )}
        {removal.error && <p className="plugin-error">{removal.error}</p>}
        <div className="tex-install-actions">
          <Button disabled={removal.busy} onClick={() => void answerTurnOff(false, false)}>
            Cancel
          </Button>
          <Button variant="primary" disabled={removal.busy} onClick={() => void answerTurnOff(true, remove)}>
            {removal.error ? 'Turn off anyway' : remove && removal.method ? 'Turn off and remove TeX' : 'Turn off'}
          </Button>
        </div>
      </div>
    </Modal>
  )
}
