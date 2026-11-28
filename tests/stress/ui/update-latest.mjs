// On the latest version, the update panel says so and leaves out the release notes
// (src/components/UpdateButton.tsx). A newer release still shows its notes. On the nightly
// channel the panel names a nightly the way its GitHub release title does, in UTC.
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { TMP, check, done, launch, shot, start } from './harness.mjs'

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

// The nightly channel, reading GitHub's releases list the way the browser build does
// (releaseNotesFromList in src/lib/updater.ts). A nightly's tag, v99.1.0-nightly-2099-01-02-1015,
// reads back as the version 99.1.0-nightly.209901021015, which orders it, and the panel names it
// "Nightly v99.1.0 2099-01-02 10:15 UTC", the same as its release title. In UTC, so a clock in
// Auckland (13 hours ahead) shows the same.
{
  const release = (tag, name, body, prerelease) => ({ tag_name: tag, name, body, html_url: `https://github.com/elijahroizman/writan/releases/tag/${tag}`, published_at: '2099-01-02T10:20:00Z', draft: false, prerelease })
  const list = [
    release('nightly', 'Nightly update feed', 'The update feed.', true),
    release('apt', 'APT repository', 'Packages', true),
    release('v99.1.0-nightly-2099-01-01-0930', 'Writan Nightly v99.1.0 2099-01-01 09:30 UTC', '- First nightly note', true),
    release('v99.0.0', 'Writan v99.0.0', '- Stable note', false),
    release('v99.1.0-nightly-2099-01-02-1015', 'Writan Nightly v99.1.0 2099-01-02 10:15 UTC', '- Second nightly note', true),
    // Not quite a nightly's tag: skipped, like the other pre-releases that aren't versions.
    release('v99.1.0-nightly-2099-01-03-10', 'Broken', '- Broken note', true),
  ]
  const { browser, context, page, logs } = await launch({
    tauri: false,
    timezoneId: 'Pacific/Auckland',
    viewport: { width: 1400, height: 1000 },
    init: () => localStorage.setItem('workspace.updateChannel', 'nightly'),
  })
  await context.route(/^https:\/\/api\.github\.com\/repos\/elijahroizman\/writan\/releases/, (route) => route.fulfill({ json: list }))
  await start(page)
  await page.locator('.update-btn').click()
  await page.getByText('Second nightly note').waitFor({ timeout: 10000 })
  const heading = await page.locator('#update-title').innerText()
  check('nightly: the heading is the release title', heading === 'Writan Nightly v99.1.0 2099-01-02 10:15 UTC', heading)
  const changes = await page.locator('.update-release h3').allInnerTexts()
  check(
    'nightly: missed versions in order, named like their releases',
    JSON.stringify(changes) === JSON.stringify(['Changes in Nightly v99.1.0 2099-01-02 10:15 UTC', 'Changes in Nightly v99.1.0 2099-01-01 09:30 UTC', 'Changes in 99.0.0']),
    JSON.stringify(changes),
  )
  const text = await page.locator('.update-panel').innerText()
  check('nightly: a tag that isn\'t a nightly\'s is left out', !text.includes('Broken note'), text)
  await shot(page, join(TMP, 'update-panel-nightly.png'), '.update-panel')
  await done(browser, logs)
}
