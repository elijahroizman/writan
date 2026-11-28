// On the latest version, the update panel says so and leaves out the release notes
// (src/components/UpdateButton.tsx). A newer release still shows its notes.
import { readFileSync } from 'node:fs'
import { check, done, launch, start } from './harness.mjs'

const current = JSON.parse(readFileSync(new URL('../../../package.json', import.meta.url), 'utf8')).version

/** A Mac release build whose GitHub release notes say the latest version is `version`. Playwright
 * sends an init script as source, so the version goes in as text. */
const fakeRelease = (version) => `(${fake})(${JSON.stringify(version)})`
function fake(version) {
  Object.defineProperty(navigator, 'platform', { value: 'MacIntel' })
  const patch = setInterval(() => {
    const bridge = window.__TAURI_INTERNALS__
    if (!bridge) return
    clearInterval(patch)
    const invoke = bridge.invoke
    bridge.invoke = async (cmd, ...args) => {
      if (cmd === 'release_notes') {
        return { version, name: `Writan ${version}`, url: 'https://example.com/release', publishedAt: '2026-11-27T10:00:00Z', releases: [], truncated: false, notes: '## Fixes\n\n- Release note text' }
      }
      if (cmd === 'check_update' || cmd === 'plugin:updater|check') return null
      return invoke(cmd, ...args)
    }
  }, 1)
}

async function panel(latest) {
  const { browser, page, logs } = await launch({ init: fakeRelease(latest) })
  await start(page)
  await page.locator('.update-btn').click()
  await page.waitForFunction(() => !/Checking|Fetching/.test(document.querySelector('.update-panel')?.textContent ?? 'Checking'))
  // The notes render after markdown-it loads.
  if (latest !== current) await page.getByText('Release note text').waitFor({ timeout: 5000 }).catch(() => {})
  const text = await page.locator('.update-panel').innerText()
  return { browser, logs, text, notes: await page.locator('.update-panel .update-notes').count() }
}

{
  const { browser, logs, text, notes } = await panel(current)
  check('latest: says so', text.includes('You’re on the latest version') || text.includes("You're on the latest version"), text)
  check('latest: no release notes', notes === 0 && !text.includes('Release note text'), text)
  check('latest: notes are a link away', text.includes('What’s new in this version'), text)
  await done(browser, logs)
}

// The latest release is older than this copy, as for a nightly waiting for the next stable
// release: still the latest, but that older release's notes aren't this version's.
{
  const { browser, logs, text, notes } = await panel('0.0.1')
  check('older release: says latest', text.includes("You're on the latest version"), text)
  check('older release: no notes and no link to them', notes === 0 && !text.includes('What’s new in this version') && !text.includes('Release note text'), text)
  await done(browser, logs)
}

{
  const { browser, logs, text } = await panel('99.0.0')
  check('newer release: notes still show', text.includes('Release note text') && !text.includes('latest version'), text)
  await done(browser, logs)
}
