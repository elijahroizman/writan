// Microsoft's core fonts on Linux: the tour offers them only where the system lacks them, nothing
// downloads before the license is accepted, failures say so, and installed fonts reach the page.
// The fake bridge serves Liberation Sans for every file: Microsoft's fonts can't be committed.
// node tests/stress/ui/core-fonts.mjs (on Linux; the step is Linux-only)
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { BASE, launch, check, done } from './harness.mjs'

const FACE = readFileSync(resolve(import.meta.dirname, '../../../src/engine/fonts/LiberationSans-Regular.ttf'))

/** Runs in the page: the tour shows, and the font commands answer from window.__core. */
function init(systemHas) {
  localStorage.removeItem('workspace.setupDone')
  const real = window.__TAURI_INTERNALS__.invoke
  const core = (window.__core = { installed: false, calls: [], fail: null })
  window.__TAURI_INTERNALS__.invoke = async (cmd, args, opts) => {
    switch (cmd) {
      case 'system_has_core_fonts':
        return systemHas
      case 'installed_fonts':
        return core.installed ? ['ms-core-fonts'] : []
      case 'install_font':
        core.calls.push(['install', args])
        await new Promise((r) => setTimeout(r, 300))
        if (!args.acceptedLicense) throw "Accept Microsoft's license to download these fonts."
        if (core.fail) throw core.fail
        core.installed = true
        return null
      case 'remove_font':
        core.calls.push(['remove', args])
        core.installed = false
        return null
      case 'font_file':
        return (await fetch('/__core-font')).arrayBuffer()
    }
    return real(cmd, args, opts)
  }
}

async function tour(systemHas) {
  const run = await launch({ viewport: { width: 1200, height: 820 }, init: `(${init})(${systemHas})` })
  await run.context.route('**/__core-font', (route) => route.fulfill({ body: FACE, contentType: 'font/ttf' }))
  await run.page.goto(BASE)
  await run.page.waitForSelector('.welcome')
  await run.page.getByRole('button', { name: /Show me around/ }).click()
  return run
}

const nextUntil = async (page, title) => {
  for (let i = 0; i < 12; i++) {
    const now = await page.locator('#welcome-title').textContent()
    if (now === title) return true
    if (!(await page.getByRole('button', { name: /^Next/ }).count())) return false
    await page.getByRole('button', { name: /^Next/ }).click()
  }
  return false
}

// A system with the fonts already: no step.
{
  const { browser, page, logs } = await tour(true)
  check('no fonts step where the system has the fonts', !(await nextUntil(page, 'Use Microsoft’s own fonts')))
  await done(browser, logs)
}

const { browser, page, logs } = await tour(false)
check('the tour offers the fonts on Linux', await nextUntil(page, 'Use Microsoft’s own fonts'))
const download = page.getByRole('button', { name: /Download the fonts/ })
check('the license is shown', (await page.locator('.core-fonts-license').textContent()).includes('END-USER LICENSE AGREEMENT'))
check('says Calibri and Cambria cannot be downloaded', (await page.locator('.tour-text').textContent()).includes('Calibri and Cambria'))
check('nothing downloads before the license is accepted', await download.isDisabled())
await page.screenshot({ path: '/tmp/writan-stress/core-fonts-tour.png' })

await page.evaluate(() => (window.__core.fail = 'Couldn’t download the font from SourceForge. Check your connection and try again.'))
await page.getByLabel('I accept Microsoft’s license agreement').check()
await download.click()
await page.waitForSelector('.core-fonts-offer .plugin-error')
check('a failed download says why', (await page.locator('.core-fonts-offer .plugin-error').textContent()).includes('SourceForge'))

await page.evaluate(() => (window.__core.fail = null))
await download.click()
await page.waitForSelector('.core-fonts-done')
const calls = await page.evaluate(() => window.__core.calls)
check('every download was asked with the license accepted', calls.every(([, args]) => args.acceptedLicense === true), JSON.stringify(calls))
const families = () => page.evaluate(() => [...new Set([...document.fonts].filter((f) => f.status === 'loaded').map((f) => f.family.replace(/"/g, '')))])
await page.waitForFunction(() => [...document.fonts].some((f) => f.family.replace(/"/g, '') === 'Webdings'))
const added = await families()
check('the fonts reach the page under their own names', ['Arial', 'Times New Roman', 'Courier New', 'Georgia', 'Verdana', 'Trebuchet MS'].every((f) => added.includes(f)), added.join(', '))
await page.screenshot({ path: '/tmp/writan-stress/core-fonts-installed.png' })

// Settings › Plugins: off removes them; on asks for the license again before anything downloads.
await page.getByRole('button', { name: 'Skip tour' }).click()
await page.waitForSelector('.welcome', { state: 'detached' })
await page.keyboard.press('Control+,')
await page.getByRole('button', { name: 'Plugins' }).click()
const row = page.locator('.plugin-row', { hasText: 'Microsoft’s core fonts' })
check('Settings lists the fonts', (await row.count()) === 1)
await row.getByRole('switch').click()
await page.waitForFunction(() => window.__core.calls.some(([c]) => c === 'remove'))
await page.waitForFunction(() => ![...document.fonts].some((f) => f.family.replace(/"/g, '') === 'Arial'))
check('turning them off takes them off the page', true)
await row.getByRole('switch').click()
const installs = await page.evaluate(() => window.__core.calls.filter(([c]) => c === 'install').length)
check('turning them on shows the license and downloads nothing yet', (await row.locator('.core-fonts-license').count()) === 1 && installs === 2)
await page.screenshot({ path: '/tmp/writan-stress/core-fonts-settings.png' })
await done(browser, logs)
