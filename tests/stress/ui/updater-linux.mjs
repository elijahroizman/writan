// The Linux updater (src/lib/updater.ts), where apt, dnf or pacman installs a newer release:
//
// - switching from nightly to stable while the first check waits on `linux_package` drops that
//   check's nightly and checks stable, instead of leaving the nightly ready on the stable channel;
// - a dismissed password prompt leaves the same version ready to try again;
// - a failed install checks again: when the nightly that was ready has gone (only the newest are
//   kept), the newer one becomes ready, and when the same version comes back the error shows.
import assert from 'node:assert/strict'
import { launch, start, done, check } from './harness.mjs'

const FIRST = '99.0.1-nightly.209901010000'
const SECOND = '99.0.1-nightly.209901020000'

const { browser, page, logs } = await launch({ init: () => {
  Object.defineProperty(navigator, 'platform', { value: 'Linux x86_64' })
  localStorage.setItem('workspace.updateChannel', 'nightly')
} })

try {
  await page.clock.install()
  await start(page)
  await page.evaluate((first) => {
    const bridge = window.__TAURI_INTERNALS__
    const invoke = bridge.invoke
    window.notesAsked = []
    window.upgrades = []
    window.newestNightly = first
    window.upgradeError = null
    bridge.invoke = async (cmd, args, ...rest) => {
      if (cmd === 'release_notes') {
        window.notesAsked.push(args.channel)
        const version = args.channel === 'nightly' ? window.newestNightly : '0.0.1'
        return { version, name: `Writan ${version}`, notes: 'Notes', url: 'https://github.com/elijahroizman/writan/releases', publishedAt: null, releases: [], truncated: false }
      }
      if (cmd === 'linux_package') {
        await new Promise((resolve) => { window.finishProbe = resolve })
        return { format: 'deb', manager: true, repo: true }
      }
      if (cmd === 'package_upgrade') {
        window.upgrades.push(args.version)
        if (window.upgradeError) throw window.upgradeError
        return null
      }
      return invoke(cmd, args, ...rest)
    }
  }, FIRST)
  const state = () => page.evaluate(() => {
    const { phase, version, error } = window.__updates.useUpdate.getState()
    return { phase, version, error }
  })

  // The launch check reads the nightly's notes, then waits on the package probe.
  await page.clock.fastForward(10_000)
  await page.waitForFunction(() => !!window.finishProbe)
  await page.evaluate(() => window.__updates.setUpdateChannel('stable'))
  await page.evaluate(() => window.finishProbe())
  await page.waitForFunction(() => window.notesAsked.includes('stable') && window.__updates.useUpdate.getState().phase === 'current')
  assert.deepEqual(await state(), { phase: 'current', version: '0.0.1', error: null })
  check('switching to stable during the package probe checks stable', true)

  await page.evaluate(() => window.__updates.setUpdateChannel('nightly'))
  await page.waitForFunction(() => window.__updates.useUpdate.getState().phase === 'ready')
  assert.equal((await state()).version, FIRST)

  // The password prompt dismissed: the same nightly stays ready, and nothing checks again.
  const asked = await page.evaluate(() => window.notesAsked.length)
  await page.evaluate(() => { window.upgradeError = 'Updating Writan was cancelled.' })
  await page.locator('.update-btn').click()
  await page.locator('.update-go').click()
  await page.waitForFunction(() => window.upgrades.length === 1)
  await page.waitForFunction(() => window.__updates.useUpdate.getState().error)
  assert.deepEqual(await state(), { phase: 'ready', version: FIRST, error: 'Updating Writan was cancelled.' })
  assert.equal(await page.evaluate(() => window.notesAsked.length), asked)
  await page.getByText('Updating Writan was cancelled.', { exact: true }).waitFor()
  check('a dismissed password prompt keeps the update ready', true)

  // The ready nightly was pruned: the install fails, the check finds the newer one, and that one installs next.
  await page.evaluate((second) => {
    window.newestNightly = second
    window.upgradeError = "Couldn't download the update. Check your connection and try again."
  }, SECOND)
  await page.locator('.update-go').click()
  await page.waitForFunction((second) => window.__updates.useUpdate.getState().version === second && window.__updates.useUpdate.getState().phase === 'ready', SECOND)
  assert.deepEqual(await state(), { phase: 'ready', version: SECOND, error: null })
  assert.deepEqual(await page.evaluate(() => window.upgrades), [FIRST, FIRST])
  check('a failed install of a pruned nightly moves on to the newest', true)

  // The same version comes back from the check: it stays ready, with the error.
  await page.evaluate(() => { window.upgradeError = "The nightly didn't install. Try again in a few minutes." })
  await page.locator('.update-go').click()
  await page.waitForFunction(() => window.upgrades.length === 3 && window.__updates.useUpdate.getState().error)
  assert.deepEqual(await state(), { phase: 'ready', version: SECOND, error: "The nightly didn't install. Try again in a few minutes." })
  await page.evaluate(() => { window.upgradeError = null })
  await page.locator('.update-go').click()
  await page.waitForFunction(() => window.upgrades.length === 4)
  assert.deepEqual(await page.evaluate(() => window.upgrades), [FIRST, FIRST, SECOND, SECOND])
  check('a failed install of a version that is still there can be tried again', true)
} finally {
  await done(browser, logs)
}
