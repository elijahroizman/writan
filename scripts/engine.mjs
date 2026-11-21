// Builds the document engine (crates/office-wasm) for WebAssembly into src/engine/wasm, which
// the Word, PowerPoint and Excel editors load. Skips the build when nothing in crates/ is newer
// than the last one. Needs the wasm32-unknown-unknown target and wasm-bindgen (the CLI version
// must match the crate's: see crates/office-wasm/Cargo.toml).

import { execFileSync } from 'node:child_process'
import { existsSync, readdirSync, statSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const crates = join(root, 'crates')
const out = join(root, 'src/engine/wasm')
const built = join(out, 'office_wasm_bg.wasm')

function newest(dir) {
  let t = 0
  for (const name of readdirSync(dir)) {
    if (name === 'target' || name.startsWith('.')) continue
    const p = join(dir, name)
    const s = statSync(p)
    t = Math.max(t, s.isDirectory() ? newest(p) : s.mtimeMs)
  }
  return t
}

if (!process.argv.includes('--force') && existsSync(built) && statSync(built).mtimeMs >= newest(crates)) {
  process.exit(0)
}
const env = { ...process.env, CARGO_TARGET_DIR: process.env.CARGO_TARGET_DIR ?? join(crates, 'target') }
execFileSync('cargo', ['build', '--release', '--target', 'wasm32-unknown-unknown', '-p', 'office-wasm'], { cwd: crates, stdio: 'inherit', env })
const wasm = join(env.CARGO_TARGET_DIR, 'wasm32-unknown-unknown/release/office_wasm.wasm')
// Without the name section (function names for debuggers, about 1.1 MB of the 8 MB), the app
// has less to load and compile on each launch.
execFileSync('wasm-bindgen', ['--target', 'web', '--remove-name-section', '--out-dir', out, wasm], { stdio: 'inherit' })
console.log('Built the document engine into src/engine/wasm')
