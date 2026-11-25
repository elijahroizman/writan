// Builds the document and PDF engines for WebAssembly. The editors load the PDF module on
// demand. The wasm-bindgen CLI version must match the pins in the two wrappers' Cargo.toml.

import { execFileSync } from 'node:child_process'
import { existsSync, readdirSync, statSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(import.meta.url)
const root = join(dirname(script), '..')
const crates = join(root, 'crates')
const env = { ...process.env, CARGO_TARGET_DIR: resolve(crates, process.env.CARGO_TARGET_DIR ?? 'target') }
const shared = Math.max(...[script, join(crates, 'Cargo.toml'), join(crates, 'Cargo.lock')].map(path => statSync(path).mtimeMs))

function newest(path) {
  const info = statSync(path)
  if (!info.isDirectory()) return info.mtimeMs
  return Math.max(info.mtimeMs, ...readdirSync(path).filter(name => name !== 'target' && !name.startsWith('.'))
    .map(name => newest(join(path, name))))
}

function build(pkg, output, sources) {
  const name = pkg.replaceAll('-', '_')
  const out = join(root, output)
  const built = [name + '_bg.wasm', name + '.js', name + '.d.ts'].map(file => join(out, file))
  const changed = Math.max(shared, ...sources.map(source => newest(join(crates, source))))
  if (!process.argv.includes('--force') && built.every(file => existsSync(file) && statSync(file).mtimeMs >= changed)) return

  execFileSync('cargo', ['build', '--release', '--target', 'wasm32-unknown-unknown', '-p', pkg], { cwd: crates, stdio: 'inherit', env })
  const wasm = join(env.CARGO_TARGET_DIR, 'wasm32-unknown-unknown', 'release', name + '.wasm')
  // Strip debugger-only function names to reduce the amount of code the app loads.
  execFileSync('wasm-bindgen', ['--target', 'web', '--remove-name-section', '--out-dir', out, wasm], { stdio: 'inherit' })
  console.log(`Built ${pkg} into ${output}`)
}

build('office-wasm', 'src/engine/wasm', ['office-core', 'office-engine', 'office-sheets', 'office-slides', 'office-wasm', 'office-word'])
build('office-pdf-wasm', 'src/lib/pdf/wasm', ['office-pdf', 'office-pdf-wasm'])
