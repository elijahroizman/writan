import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { existsSync } from 'node:fs'
import { createServer } from 'node:net'
import { availableParallelism } from 'node:os'
import { fileURLToPath } from 'node:url'
import { setTimeout as delay } from 'node:timers/promises'

const root = fileURLToPath(new URL('../', import.meta.url))
const host = '127.0.0.1'
const port = 4173
const url = `http://${host}:${port}/`
const timeout = 180_000
// Each test opens its own browser, so several run at once. BROWSER_TEST_JOBS=1 runs them in turn.
const jobs = Math.max(1, Number(process.env.BROWSER_TEST_JOBS) || Math.min(4, availableParallelism() - 1))
const scripts = [
  'tests/stress/ui/circular-reference.mjs',
  'tests/stress/ui/css-fonts.mjs',
  'tests/stress/ui/datasafety.mjs',
  'tests/stress/ui/hidden-sheets.mjs',
  'tests/stress/ui/excel-visuals.mjs',
  'tests/stress/ui/latex-math.mjs',
  'tests/stress/ui/linked-excel.mjs',
  'tests/stress/ui/nasty.mjs',
  'tests/stress/ui/new-presentation.mjs',
  'tests/stress/ui/offline-ocr.mjs',
  'tests/stress/ui/pdf-vector-export.mjs',
  'tests/stress/ui/reload.mjs',
  'tests/stress/ui/row-header-menu.mjs',
  'tests/stress/ui/security.mjs',
  'tests/stress/ui/session.mjs',
  'tests/stress/ui/sheet-rename.mjs',
  'tests/stress/ui/slides-animations.mjs',
  'tests/stress/ui/slides-clipboard.mjs',
  'tests/stress/ui/slides-keyboard.mjs',
  'tests/stress/ui/slides-video.mjs',
  'tests/stress/ui/tab-overflow.mjs',
  'tests/stress/ui/themes.mjs',
  'tests/stress/ui/tracked-changes.mjs',
  'tests/stress/ui/updater.mjs',
  'tests/stress/ui/ux-regressions.mjs',
  'tests/stress/ui/version-history.mjs',
  'tests/stress/native/pdf-page-select.mjs',
  'tests/stress/native/pdf-flatten.mjs',
  'tests/stress/native/save-conflict.mjs',
]
// ui/harness.mjs is shared setup, not a test.
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

// With `output`, the child's output is collected there instead of printed, so tests running at
// the same time don't interleave their lines.
function launch(args, env = {}, output) {
  const child = spawn(process.execPath, args, {
    cwd: root,
    env: { ...process.env, ...env },
    stdio: output ? ['ignore', 'pipe', 'pipe'] : 'inherit',
    detached,
  })
  children.add(child)
  if (output) {
    child.stdout.on('data', (chunk) => output.push(chunk))
    child.stderr.on('data', (chunk) => output.push(chunk))
  }
  const finished = new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('exit', (code, signal) => resolve({ code, signal }))
  })
  // The last output can arrive after the exit.
  const closed = once(child, 'close').catch(() => {})
  return { child, finished, closed }
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
  const stopped = server.finished.then(() => { throw new Error('Vite preview stopped during browser tests') })
  stopped.catch(() => {})
  async function run(script) {
    const start = performance.now()
    const args = script.includes('/native/') ? [script, url, 'chromium'] : [script]
    const output = []
    const test = launch(args, { WRITAN_URL: url, ENGINE: 'chromium' }, jobs > 1 ? output : undefined)
    let timedOut = false
    const timer = setTimeout(() => {
      timedOut = true
      kill(test.child)
    }, timeout)
    let result
    try {
      result = await Promise.race([test.finished, stopped])
    } finally {
      clearTimeout(timer)
      kill(test.child)
      children.delete(test.child)
      await Promise.race([test.closed, delay(2000)])
      if (output.length) process.stdout.write(Buffer.concat(output))
    }
    const ok = result.code === 0 && !timedOut
    if (ok) passed++
    else failed++
    const detail = timedOut ? 'timed out after 180s' : `exit ${result.code ?? result.signal}`
    console.log(`${ok ? 'PASS' : 'FAIL'} ${script} (${((performance.now() - start) / 1000).toFixed(2)}s, ${detail})`)
  }
  const queue = [...scripts]
  await Promise.all(Array.from({ length: Math.min(jobs, queue.length) }, async () => {
    while (queue.length && !interrupted) await run(queue.shift())
  }))
} catch (error) {
  failed++
  console.error(`FAIL browser runner: ${error.message}`)
} finally {
  for (const child of children) kill(child)
  if (!interrupted) process.exitCode = failed ? 1 : 0
  console.log(`Browser tests: ${passed} passed, ${failed} failed (${((performance.now() - started) / 1000).toFixed(2)}s)`)
}
