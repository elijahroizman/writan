import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { existsSync } from 'node:fs'
import { createServer } from 'node:net'
import { fileURLToPath } from 'node:url'
import { setTimeout as delay } from 'node:timers/promises'

const root = fileURLToPath(new URL('../', import.meta.url))
const host = '127.0.0.1'
const port = 4173
const url = `http://${host}:${port}/`
const timeout = 180_000
const scripts = [
  'tests/stress/ui/css-fonts.mjs',
  'tests/stress/ui/datasafety.mjs',
  'tests/stress/ui/hidden-sheets.mjs',
  'tests/stress/ui/nasty.mjs',
  'tests/stress/ui/security.mjs',
  'tests/stress/ui/slides-clipboard.mjs',
  'tests/stress/native/pdf-page-select.mjs',
  'tests/stress/native/pdf-flatten.mjs',
  'tests/stress/native/save-conflict.mjs',
]
// ui/harness.mjs is shared setup, not a test.
// ui/session.mjs observes unsaved-edit loss without asserting recovery.
// native/pdf-unchanged.mjs requires an external signed PDF, not a committed fixture.

const children = new Set()
const detached = process.platform !== 'win32'
let interrupted = false
function kill(child) {
  if (!child.pid) return
  try {
    if (detached) process.kill(-child.pid, 'SIGKILL')
    else child.kill('SIGKILL')
  } catch (error) {
    if (error.code !== 'ESRCH') throw error
  }
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    interrupted = true
    process.exitCode = signal === 'SIGINT' ? 130 : 143
    for (const child of children) kill(child)
  })
}

function launch(args, env = {}) {
  const child = spawn(process.execPath, args, {
    cwd: root,
    env: { ...process.env, ...env },
    stdio: 'inherit',
    detached,
  })
  children.add(child)
  const finished = new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('exit', (code, signal) => resolve({ code, signal }))
  })
  return { child, finished }
}

async function checkPort() {
  const probe = createServer()
  try {
    const listening = once(probe, 'listening')
    probe.listen(port, host)
    await listening
  } catch (error) {
    throw new Error(`Preview port ${port} is unavailable; stop the existing server first.`, { cause: error })
  } finally {
    if (probe.listening) await new Promise((resolve) => probe.close(resolve))
  }
}

async function ready(server) {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline && !interrupted) {
    if (server.exitCode !== null || server.signalCode !== null) throw new Error('Vite preview exited before it was ready')
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(1000) })
      await response.body?.cancel()
      if (response.ok) return
    } catch {}
    await delay(100)
  }
  throw new Error('Vite preview did not become ready within 30 seconds')
}

const started = performance.now()
let passed = 0
let failed = 0
try {
  if (!existsSync(new URL('../dist/index.html', import.meta.url))) {
    throw new Error('Missing build. Run VITE_TEST_HOOKS=1 npm run build first.')
  }
  await checkPort()
  const server = launch(['node_modules/vite/bin/vite.js', 'preview', '--host', host, '--port', String(port), '--strictPort'])
  await ready(server.child)
  for (const script of scripts) {
    if (interrupted) break
    const start = performance.now()
    const args = script.includes('/native/') ? [script, url, 'chromium'] : [script]
    const test = launch(args, { WRITAN_URL: url, ENGINE: 'chromium' })
    let timedOut = false
    const timer = setTimeout(() => {
      timedOut = true
      kill(test.child)
    }, timeout)
    let result
    try {
      result = await Promise.race([
        test.finished,
        server.finished.then(() => { throw new Error('Vite preview stopped during browser tests') }),
      ])
    } finally {
      clearTimeout(timer)
      kill(test.child)
      children.delete(test.child)
    }
    const ok = result.code === 0 && !timedOut
    if (ok) passed++
    else failed++
    const detail = timedOut ? 'timed out after 180s' : `exit ${result.code ?? result.signal}`
    console.log(`${ok ? 'PASS' : 'FAIL'} ${script} (${((performance.now() - start) / 1000).toFixed(2)}s, ${detail})`)
  }
} catch (error) {
  failed++
  console.error(`FAIL browser runner: ${error.message}`)
} finally {
  for (const child of children) kill(child)
  if (!interrupted) process.exitCode = failed ? 1 : 0
  console.log(`Browser tests: ${passed} passed, ${failed} failed (${((performance.now() - started) / 1000).toFixed(2)}s)`)
}
