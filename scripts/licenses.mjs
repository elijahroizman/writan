// Collects the license of every open-source package that ships in Writan: the npm packages in
// the app and in each plugin, and the Rust crates in the native app. Writes public/licenses.json,
// which Settings › General › Open-source licenses shows. Runs as part of `npm run build`.

import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))

/** npm packages each plugin bundles (plugins/<id>/vite.config.ts builds them). */
const PLUGIN_PACKAGES = {
  LaTeX: ['@codemirror/commands', '@codemirror/legacy-modes', '@codemirror/language', '@codemirror/search', '@codemirror/view'],
}

const TEMPLATES = {
  MIT: (holder) => `MIT License

Copyright (c) ${holder}

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.`,
  ISC: (holder) => `ISC License

Copyright (c) ${holder}

Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted, provided that the above copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.`,
}

function holderOf(meta) {
  const author = typeof meta.author === 'string' ? meta.author.replace(/\s*[<(].*$/, '') : meta.author?.name
  return author || `the ${meta.name} authors`
}

const LICENSE_FILE = /^(licen[cs]e|copying|notice|third[-_]party[-_]notices)([.-].*)?$/i
const texts = new Map()

/** Store each distinct text once; packages point at it by id. */
function textId(text) {
  const id = createHash('sha1').update(text).digest('hex').slice(0, 12)
  texts.set(id, text)
  return id
}

function licenseFiles(dir) {
  const files = []
  let names = []
  try {
    names = readdirSync(dir)
  } catch {
    return files
  }
  for (const name of names.sort()) {
    const path = join(dir, name)
    if (LICENSE_FILE.test(name) && statSync(path).isFile()) files.push({ file: name, text: textId(readFileSync(path, 'utf8').trim()) })
    // Bundled fonts and the like keep their licenses in a folder (docx-editor's licenses/).
    else if (/^licen[cs]es$/i.test(name) && statSync(path).isDirectory()) {
      for (const inner of readdirSync(path).sort()) {
        const innerPath = join(path, inner)
        if (statSync(innerPath).isFile()) files.push({ file: `${name}/${inner}`, text: textId(readFileSync(innerPath, 'utf8').trim()) })
      }
    }
  }
  return files
}

/** Where Node would find `name` from `fromDir`. */
function resolvePackage(name, fromDir) {
  for (let dir = fromDir; ; dir = dirname(dir)) {
    const candidate = join(dir, 'node_modules', name, 'package.json')
    if (existsSync(candidate)) return dirname(candidate)
    if (dirname(dir) === dir) return null
  }
}

function npmPackages(names, group) {
  const seen = new Map()
  const walk = (name, fromDir) => {
    const dir = resolvePackage(name, fromDir)
    if (!dir || seen.has(dir)) return
    const meta = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'))
    const license = typeof meta.license === 'string' ? meta.license : (meta.license?.type ?? (meta.licenses ?? []).map((l) => l.type).join(' OR ')) || 'See files'
    let files = licenseFiles(dir)
    // Some packages publish no license file; the license still asks for its notice.
    if (!files.length && TEMPLATES[license]) files = [{ file: `${license} (from package.json)`, text: textId(TEMPLATES[license](holderOf(meta))) }]
    seen.set(dir, {
      group,
      name: meta.name,
      version: meta.version,
      license,
      homepage: typeof meta.homepage === 'string' ? meta.homepage : typeof meta.repository === 'string' ? meta.repository : meta.repository?.url?.replace(/^git\+/, '').replace(/\.git$/, ''),
      files,
    })
    for (const dep of Object.keys({ ...meta.dependencies, ...meta.optionalDependencies })) walk(dep, dir)
  }
  for (const name of names) walk(name, root)
  // The same version can be installed in two places; list it once.
  const unique = new Map([...seen.values()].map((p) => [`${p.name}@${p.version}`, p]))
  return [...unique.values()]
}

function cratePackages(manifestRel, group, extraArgs = []) {
  let metadata
  try {
    metadata = JSON.parse(
      execFileSync('cargo', ['metadata', '--format-version', '1', ...extraArgs, '--manifest-path', join(root, manifestRel)], {
        encoding: 'utf8',
        maxBuffer: 256 * 1024 * 1024,
        stdio: ['ignore', 'pipe', 'inherit'],
      }),
    )
  } catch (err) {
    // A list without the crates would claim to be complete; don't write one.
    throw new Error(`licenses: cargo metadata failed for ${manifestRel}, so the Rust crates can't be listed: ${err.message}`)
  }
  const ours = new Set(metadata.workspace_members)
  const inGraph = new Set(metadata.resolve.nodes.map((n) => n.id))
  return metadata.packages
    .filter((p) => inGraph.has(p.id) && !ours.has(p.id))
    .map((p) => {
      const license = p.license ?? (p.license_file ? 'See files' : 'Unknown')
      let files = licenseFiles(dirname(p.manifest_path))
      // A crate can name its license file anything (license-file in Cargo.toml).
      if (p.license_file) {
        const declared = join(dirname(p.manifest_path), p.license_file)
        if (existsSync(declared) && !files.some((f) => join(dirname(p.manifest_path), f.file) === declared)) {
          files.push({ file: p.license_file, text: textId(readFileSync(declared, 'utf8').trim()) })
        }
      }
      // Dual-licensed crates that publish no file: take the MIT option, with its notice.
      if (!files.length && /\bMIT\b/.test(license)) {
        const holder = p.authors?.length ? p.authors.map((a) => a.replace(/\s*<.*$/, '')).join(', ') : `the ${p.name} authors`
        files = [{ file: 'MIT (from Cargo.toml)', text: textId(TEMPLATES.MIT(holder)) }]
      }
      return { group, name: p.name, version: p.version, license, homepage: p.homepage ?? p.repository ?? `https://crates.io/crates/${p.name}`, files }
    })
}

/** Fonts shipped directly with the Rust engine's renderer. */
function bundledFonts() {
  const fonts = join(root, 'src', 'engine', 'fonts')
  return [
    ['Carlito', 'OFL-Carlito.txt', 'https://github.com/googlefonts/carlito'],
    ['Caladea', 'OFL-Caladea.txt', 'https://github.com/huertatipografica/Caladea'],
    ['Liberation Sans, Serif and Mono', 'LICENSE-Liberation.txt', 'https://github.com/liberationfonts/liberation-fonts'],
    ['Noto Sans, Noto Sans Arabic and Noto Sans Math', 'pdf/OFL-Noto.txt', 'https://github.com/notofonts/notofonts.github.io'],
  ].map(([name, file, homepage]) => ({ group: 'App', name, version: 'bundled', license: 'OFL-1.1', homepage, files: [{ file, text: textId(readFileSync(join(fonts, file), 'utf8').trim()) }] }))
}

/** Code adapted from other projects rather than installed as a package. */
function adaptedCode() {
  return [
    // Themes: the editor, custom CSS's safety checks, the Open VSX search and the agent tools.
    ['T3 Code (themes)', 'T3-Code-LICENSE.txt', 'https://github.com/pingdotgg/t3code'],
  ].map(([name, file, homepage]) => ({ group: 'App', name, version: 'adapted', license: 'MIT', homepage, files: [{ file, text: textId(readFileSync(join(root, 'assets', 'third-party', file), 'utf8').trim()) }] }))
}

/** Data files shipped with the app: the English text recognition model. */
function bundledData() {
  const file = 'tessdata_fast-LICENSE.txt'
  return [{ group: 'App', name: 'Tesseract English language data (tessdata_fast)', version: 'bundled', license: 'Apache-2.0', homepage: 'https://github.com/tesseract-ocr/tessdata_fast', files: [{ file, text: textId(readFileSync(join(root, 'assets', 'third-party', file), 'utf8').trim()) }] }]
}

const byName = (a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version)
const packages = [
  // libheif-js decodes HEIC in the Images viewer, which ships inside the app.
  ...npmPackages([...Object.keys(pkg.dependencies), 'libheif-js'], 'App').sort(byName),
  ...Object.entries(PLUGIN_PACKAGES).flatMap(([plugin, names]) => npmPackages(names, `${plugin} plugin`).sort(byName)),
  ...cratePackages(join('src-tauri', 'Cargo.toml'), 'Native app (Rust)').sort(byName),
  ...cratePackages(join('crates', 'Cargo.toml'), 'Document engine (Rust/WASM)', ['--filter-platform', 'wasm32-unknown-unknown']).sort(byName),
  ...bundledFonts(),
  ...adaptedCode(),
  ...bundledData(),
]
writeFileSync(join(root, 'public', 'licenses.json'), JSON.stringify({ packages, texts: Object.fromEntries(texts) }))
console.log(`licenses: ${packages.length} packages, ${texts.size} license texts`)
