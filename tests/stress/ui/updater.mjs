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
        if (window.feedChecks === 1) await new Promise((resolve) => { window.finishLaunchCheck = resolve })
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
  // Open the panel during the launch check, then leave it untouched as the feed recovers.
  await page.clock.fastForward(10_000)
  await page.getByRole('button', { name: 'Checking…', exact: true }).click()
  await page.waitForFunction(() => !!window.finishLaunchCheck)
  await page.evaluate(() => window.finishLaunchCheck())
  await page.getByText("Couldn't check for an update.", { exact: true }).waitFor()

  await page.clock.fastForward(5_000)
  await page.waitForFunction(() => window.feedChecks === 2)
  await page.getByRole('button', { name: 'Try again' }).waitFor()
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
