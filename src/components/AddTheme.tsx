// Add a theme: search Open VSX's community color themes, or bring in a theme file (Writan's,
// T3 Code's or VS Code's). The Open VSX search follows T3 Code's (see lib/openVsx.ts).

import { Download, ExternalLink, FileUp, Palette, Search, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import type { OpenVsxSort, OpenVsxTheme } from '../lib/openVsx'
import { iconUrl, importExtension, searchThemes } from '../lib/openVsx'
import { openExternal } from '../lib/platform'
import type { ThemeDef } from '../lib/themes'
import { parseThemeFile } from '../lib/themes'
import { useAppearance } from '../state/appearance'
import { Button, IconButton, Modal, SelectField, Spinner, TextField } from './ui'

const POPULAR = ['Dracula', 'Catppuccin', 'Nord', 'Tokyo Night', 'Solarized']
const SORTS: { value: OpenVsxSort; label: string }[] = [
  { value: 'downloadCount', label: 'Most downloaded' },
  { value: 'rating', label: 'Best rated' },
  { value: 'timestamp', label: 'Newest' },
  { value: 'relevance', label: 'Best match' },
]

const newId = () => `custom-${Math.random().toString(36).slice(2, 8)}`

/** Adds themes and uses the first. Says what happened. */
function addThemes(themes: ThemeDef[]): string {
  const { save, use, custom } = useAppearance.getState()
  let updated = 0
  for (const t of themes) {
    if (custom.some((c) => c.id === t.id)) updated++
    save(t)
  }
  use(themes[0].id)
  const names = themes.map((t) => `“${t.name}”`).join(', ')
  return `${updated === themes.length ? 'Updated' : 'Added'} ${names}${themes.length > 1 ? `. Using ${themes[0].name}.` : ', and using it.'}`
}

export function AddTheme({ onClose }: { onClose: () => void }) {
  const [query, setQuery] = useState('')
  const [sort, setSort] = useState<OpenVsxSort>('downloadCount')
  const [results, setResults] = useState<OpenVsxTheme[] | null>(null)
  const [searching, setSearching] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [adding, setAdding] = useState<string | null>(null)
  const [paste, setPaste] = useState(false)
  const [json, setJson] = useState('')
  const file = useRef<HTMLInputElement>(null)

  // Searches a moment after typing stops.
  useEffect(() => {
    const q = query.trim()
    if (!q) {
      setResults(null)
      setError(null)
      setSearching(false)
      return
    }
    const abort = new AbortController()
    const timer = window.setTimeout(() => {
      setSearching(true)
      setError(null)
      searchThemes(q, sort, abort.signal)
        .then((found) => !abort.signal.aborted && setResults(found))
        .catch((err) => !abort.signal.aborted && setError((err as Error).message))
        .finally(() => !abort.signal.aborted && setSearching(false))
    }, 350)
    return () => {
      abort.abort()
      window.clearTimeout(timer)
    }
  }, [query, sort])

  const install = async (ext: OpenVsxTheme) => {
    setAdding(ext.id)
    setError(null)
    setNote(null)
    try {
      setNote(addThemes(await importExtension(ext)))
    } catch (err) {
      setError(`${ext.name}: ${(err as Error).message}`)
    } finally {
      setAdding(null)
    }
  }

  const fromText = (texts: { name: string; text: string }[]) => {
    setError(null)
    setNote(null)
    const themes: ThemeDef[] = []
    const problems: string[] = []
    for (const { name, text } of texts) {
      try {
        themes.push({ ...parseThemeFile(text, name.replace(/\.jsonc?$/i, '')), id: newId(), custom: true })
      } catch (err) {
        problems.push(`${name}: ${(err as Error).message}`)
      }
    }
    if (themes.length) setNote(addThemes(themes))
    if (problems.length) setError(problems.join('\n'))
    return themes.length > 0
  }

  // Over Settings, not inside its scrolling page.
  return createPortal(
    <Modal onClose={onClose} closeOnEscape layer="picker">
      <div className="add-theme" role="dialog" aria-modal="true" aria-labelledby="add-theme-title">
        <header className="add-theme-head">
          <h2 id="add-theme-title">Add a theme</h2>
          <IconButton label="Close" onClick={onClose}>
            <X size={16} />
          </IconButton>
        </header>

        <div className="add-theme-body">
          <section>
            <div className="add-theme-label">
              <span className="settings-label">Community themes</span>
              <span className="settings-hint">Openly licensed VS Code color themes from Open VSX. Writan takes their window colors.</span>
            </div>
            <div className="add-theme-search">
              <TextField icon={<Search size={14} className="faint" />} value={query} autoFocus placeholder="Search themes, like Dracula or Nord" onChange={(e) => setQuery(e.currentTarget.value)} />
              <SelectField value={sort} options={SORTS} onChange={(v) => setSort(v as OpenVsxSort)} />
            </div>
            {!query.trim() && (
              <div className="add-theme-popular">
                <span className="faint">Try</span>
                {POPULAR.map((p) => (
                  <button key={p} type="button" className="pill" onClick={() => setQuery(p)}>
                    {p}
                  </button>
                ))}
              </div>
            )}
            {note && (
              <p className="add-theme-note" role="status">
                {note}
              </p>
            )}
            {error && (
              <p className="add-theme-error" role="alert">
                {error}
              </p>
            )}
            {searching && !results && (
              <div className="add-theme-empty">
                <Spinner size={16} /> Searching Open VSX…
              </div>
            )}
            {results && !results.length && !searching && <div className="add-theme-empty">No openly licensed themes match “{query.trim()}”.</div>}
            {results && results.length > 0 && (
              <ul className={`add-theme-results ${searching ? 'stale' : ''}`}>
                {results.map((r) => (
                  <ThemeResult key={r.id} theme={r} adding={adding === r.id} disabled={adding !== null} onAdd={() => void install(r)} />
                ))}
              </ul>
            )}
          </section>

          <section className="add-theme-file">
            <div className="add-theme-label">
              <span className="settings-label">From a file</span>
              <span className="settings-hint">A theme exported from Writan or T3 Code, or a VS Code color theme (.json).</span>
            </div>
            <div className="add-theme-file-actions">
              <Button onClick={() => file.current?.click()}>
                <FileUp size={14} /> Choose files…
              </Button>
              <Button variant="quiet" onClick={() => setPaste(!paste)}>
                {paste ? 'Hide' : 'Paste JSON'}
              </Button>
            </div>
            <input
              ref={file}
              type="file"
              accept=".json,.jsonc,application/json"
              multiple
              hidden
              onChange={async (e) => {
                const files = [...(e.currentTarget.files ?? [])]
                e.currentTarget.value = ''
                fromText(await Promise.all(files.map(async (f) => ({ name: f.name, text: f.size > 1024 * 1024 ? '' : await f.text() }))))
              }}
            />
            {paste && (
              <>
                <textarea className="theme-css" spellCheck={false} placeholder={'{\n  "name": "Dusk",\n  "accent": "#7aa2f7",\n  "light": "#dfe3f5",\n  "dark": "#1a1b26"\n}'} value={json} onChange={(e) => setJson(e.currentTarget.value)} />
                <div>
                  <Button variant="primary" disabled={!json.trim()} onClick={() => fromText([{ name: 'Pasted theme', text: json }]) && setJson('')}>
                    Add theme
                  </Button>
                </div>
              </>
            )}
          </section>
        </div>
      </div>
    </Modal>,
    document.body,
  )
}

function ThemeResult({ theme, adding, disabled, onAdd }: { theme: OpenVsxTheme; adding: boolean; disabled: boolean; onAdd: () => void }) {
  const [icon, setIcon] = useState<string | null>(null)
  useEffect(() => {
    let url: string | null = null
    let live = true
    void iconUrl(theme).then((u) => {
      url = u
      if (live) setIcon(u)
      else if (u) URL.revokeObjectURL(u)
    })
    return () => {
      live = false
      if (url) URL.revokeObjectURL(url)
    }
  }, [theme])
  const downloads = new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }).format(theme.downloads)
  return (
    <li className="add-theme-result">
      <span className="add-theme-icon">{icon ? <img src={icon} alt="" draggable={false} /> : <Palette size={18} />}</span>
      <span className="add-theme-text">
        <span className="add-theme-name">{theme.name}</span>
        <span className="faint">
          {theme.publisher} · {downloads} downloads · {theme.license}
        </span>
        {theme.description && <span className="add-theme-desc">{theme.description}</span>}
      </span>
      <span className="add-theme-actions">
        {theme.sourceUrl && (
          <IconButton label="See its source" size="sm" onClick={() => void openExternal(theme.sourceUrl!)}>
            <ExternalLink size={13} />
          </IconButton>
        )}
        <Button disabled={disabled} onClick={onAdd}>
          {adding ? <Spinner size={12} /> : <Download size={13} />} {adding ? 'Adding…' : 'Add'}
        </Button>
      </span>
    </li>
  )
}
