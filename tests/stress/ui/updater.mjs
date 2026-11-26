import assert from 'node:assert/strict'
import { launch, start, done } from './harness.mjs'

const { browser, page, logs } = await launch({ init: () => {
  Object.defineProperty(navigator, 'platform', { value: 'MacIntel' })
} })

try {
  await page.clock.install()
  await start(page)
  await page.evaluate(() => {
    const bridge = window.__TAURI_INTERNALS__
    const invoke = bridge.invoke
    window.feedChecks = 0
    window.downloads = 0
    bridge.invoke = async (cmd, ...args) => {
      if (cmd === 'plugin:updater|check') {
        window.feedChecks++
        if (!window.feedAvailable) throw 'Could not fetch a valid release JSON'
        return { rid: 41, version: '9.0.0', body: 'Update notes' }
      }
      if (cmd === 'plugin:updater|download') {
        window.downloads++
        return 42
      }
      return invoke(cmd, ...args)
    }
  })
  // Let the launch check fail before opening the panel; opening it manually retries.
  await page.clock.fastForward(10_000)
  await page.waitForFunction(() => window.feedChecks === 1)
  await page.getByRole('button', { name: 'Check for updates', exact: true }).click()
  await page.getByText("Couldn't check for an update.", { exact: true }).waitFor()
  assert.equal(await page.evaluate(() => window.feedChecks), 2)

  // A retry with the feed still offline backs off; the next one succeeds without a click.
  await page.clock.fastForward(30_000)
  await page.waitForFunction(() => window.feedChecks === 3)
  await page.getByRole('button', { name: 'Try again' }).waitFor()
  await page.evaluate(() => { window.feedAvailable = true })
  await page.clock.fastForward(120_000)
  await page.getByRole('button', { name: 'Writan 9.0.0 is ready to install' }).waitFor()
  assert.equal(await page.evaluate(() => window.downloads), 1)
  await page.clock.fastForward(240_000)
  assert.equal(await page.evaluate(() => window.feedChecks), 4)
} finally {
  await done(browser, logs)
}
