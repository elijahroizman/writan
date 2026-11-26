// Runs the fixed performance corpus against the production build. Chromium is the default;
// `node scripts/perf.mjs --webkit` uses WebKit when its Playwright runtime is installed.
import { spawn, spawnSync } from 'node:child_process'
import { once } from 'node:events'
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../', import.meta.url))
const host = '127.0.0.1'
const port = 4173
const url = `http://${host}:${port}/`
const engine = process.argv.includes('--webkit') ? 'webkit' : 'chromium'

if (!existsSync(new URL('../dist/index.html', import.meta.url))) {
  throw new Error('Missing build. Run VITE_TEST_HOOKS=1 npm run build first.')
}
const generated = spawnSync('python3', ['tests/stress/perf/generate.py'], { cwd: root, stdio: 'inherit' })
if (generated.status) process.exit(generated.status)

const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', 'preview', '--host', host, '--port', String(port), '--strictPort'], { cwd: root, stdio: 'ignore' })
const stop = () => {
  if (!server.killed) server.kill('SIGKILL')
}
process.on('exit', stop)
try {
  const deadline = Date.now() + 30_000
  for (;;) {
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(1000) })
      await response.body?.cancel()
      if (response.ok) break
    } catch {}
    if (Date.now() > deadline) throw new Error('Vite preview did not become ready')
    await new Promise((r) => setTimeout(r, 100))
  }
  const scripts = ['startup', 'docx', 'xlsx']
  let failed = 0
  for (const type of scripts) {
    const child = spawn(process.execPath, ['tests/stress/perf/bench.mjs', `--type=${type}`], {
      cwd: root,
      env: { ...process.env, WRITAN_URL: url, ENGINE: engine },
      stdio: 'inherit',
    })
    const [code] = await once(child, 'exit')
    if (code) failed++
  }
  process.exitCode = failed ? 1 : 0
} finally {
  stop()
}
