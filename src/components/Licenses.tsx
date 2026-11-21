import { ChevronRight, Search } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { openExternal } from '../lib/platform'
import { LinkButton, Spinner, TextField } from './ui'

interface LicensedPackage {
  group: string
  name: string
  version: string
  license: string
  homepage?: string
  files: { file: string; text: string }[]
}

interface LicenseData {
  packages: LicensedPackage[]
  /** License texts by id; packages refer to them. */
  texts: Record<string, string>
}

/** Every open-source package in the app, its plugins and the native shell (scripts/licenses.mjs). */
export default function LicensesPage() {
  const [data, setData] = useState<LicenseData | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [open, setOpen] = useState<string | null>(null)
  // Hundreds of packages: each group starts folded, and a search opens them all.
  const [unfolded, setUnfolded] = useState<Set<string>>(new Set())

  useEffect(() => {
    fetch(`${import.meta.env.BASE_URL}licenses.json`)
      .then((res) => (res.ok ? (res.json() as Promise<LicenseData>) : Promise.reject(new Error('missing'))))
      .then(setData, () => setError('The license list is missing from this build. Run npm run licenses.'))
  }, [])

  const groups = useMemo(() => {
    if (!data) return []
    const q = query.trim().toLowerCase()
    const shown = data.packages.filter((p) => !q || p.name.toLowerCase().includes(q) || p.license.toLowerCase().includes(q))
    return [...new Set(shown.map((p) => p.group))].map((group) => ({ group, packages: shown.filter((p) => p.group === group) }))
  }, [data, query])

  if (error) return <p className="settings-hint">{error}</p>
  if (!data) return <Spinner />

  return (
    <>
      <p className="settings-intro">
        Writan is free to use thanks to these open-source projects. Each is used under its license, shown below.
      </p>
      <p className="settings-intro">
        The theme picker in Settings › Appearance is heavily inspired by the one in{' '}
        <LinkButton onClick={() => void openExternal('https://github.com/pingdotgg/t3code')}>T3 Code</LinkButton>
        .
      </p>
      <TextField
        className="shortcut-search"
        icon={<Search size={13} className="faint" />}
        value={query}
        placeholder="Search packages or licenses"
        onChange={(e) => setQuery(e.currentTarget.value)}
      />
      {groups.map(({ group, packages }) => {
        const folded = !query.trim() && !unfolded.has(group)
        const toggle = () =>
          setUnfolded((prev) => {
            const next = new Set(prev)
            if (!next.delete(group)) next.add(group)
            return next
          })
        return (
          <section key={group} className="shortcut-group">
            <h4>
              <button type="button" className="license-group" aria-expanded={!folded} onClick={toggle}>
                <ChevronRight size={13} className={`license-chevron ${folded ? '' : 'open'}`} />
                {group} · {packages.length}
              </button>
            </h4>
            {!folded && packages.map((p) => {
              const id = `${p.group}/${p.name}@${p.version}`
              const expanded = open === id
              return (
                <div key={id} className="license-row">
                  <button type="button" className="license-head" aria-expanded={expanded} onClick={() => setOpen(expanded ? null : id)}>
                    <ChevronRight size={13} className={`license-chevron ${expanded ? 'open' : ''}`} />
                    <span className="license-name">
                      {p.name} <span className="faint">{p.version}</span>
                    </span>
                    <span className="faint license-spdx">{p.license}</span>
                  </button>
                  {expanded && (
                    <div className="license-body">
                      {p.homepage && (
                        <LinkButton onClick={() => void openExternal(p.homepage!.replace(/^git:\/\//, 'https://'))}>
                          {p.homepage.replace(/^(git|https?):\/\//, '')}
                        </LinkButton>
                      )}
                      {p.files.length ? (
                        p.files.map((f) => (
                          <div key={f.file}>
                            <div className="settings-hint">{f.file}</div>
                            <pre className="license-text">{data.texts[f.text]}</pre>
                          </div>
                        ))
                      ) : (
                        <p className="settings-hint">This package ships no license file. It is licensed under {p.license}; see its homepage.</p>
                      )}
                    </div>
                  )}
                </div>
              )
            })}
          </section>
        )
      })}
    </>
  )
}
