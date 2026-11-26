// Themes from the themes folder (the ones agents publish): the folder button beside the name shows
// the theme's file in Finder or File Explorer, and Delete removes the file and stops using it.
import { launch, start, check, done, shot, TMP } from './harness.mjs'

const DIR = '/home/probe/.config/writan/themes'
const init = (dir) => {
  const themes = new Map([['dusk', JSON.stringify({ name: 'Dusk', accent: '#ff7744', light: '#fff4ee', dark: '#201410' })]])
  const real = window.__TAURI_INTERNALS__.invoke
  window.__themesFolder = { themes, deleted: [], revealed: [] }
  window.__TAURI_INTERNALS__.invoke = async (cmd, args, opts) => {
    if (cmd === 'themes_read') return { dir, themes: [...themes].map(([id, text]) => ({ id, text, background: null })) }
    if (cmd === 'theme_delete') {
      window.__themesFolder.deleted.push(args.id)
      themes.delete(args.id)
      return null
    }
    if (cmd === 'plugin:opener|reveal_item_in_dir') {
      window.__themesFolder.revealed.push(...args.paths)
      return null
    }
    return real(cmd, args, opts)
  }
  localStorage.setItem('workspace.appearance', JSON.stringify({ light: 'dusk', dark: 'dusk', custom: [] }))
}
const { browser, page, logs } = await launch({ viewport: { width: 1280, height: 900 } })
await page.addInitScript(init, DIR)
const folder = () => page.evaluate(() => ({ deleted: window.__themesFolder.deleted, revealed: window.__themesFolder.revealed }))

try {
  await start(page)
  await page.keyboard.press('Control+Comma')
  await page.getByRole('button', { name: /Appearance/ }).first().click()
  await page.waitForSelector('.theme-section')
  const card = page.locator('.theme-card', { has: page.locator('.theme-name', { hasText: 'Dusk' }) })
  await card.waitFor()
  check('the folder theme is listed and in use', (await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--accent').trim())) === '#ff7744')
  await shot(page, `${TMP}/themes-folder-card.png`, '.theme-grid')

  // The folder button beside the name shows the theme's file.
  const reveal = card.getByRole('button', { name: /From the themes folder/ })
  check('a folder theme has a folder button', (await reveal.count()) === 1)
  await reveal.click()
  await page.waitForFunction(() => window.__themesFolder.revealed.length, null, { timeout: 3000 }).catch(() => {})
  check('the folder button shows the theme’s file', (await folder()).revealed.includes(`${DIR}/dusk.json`), JSON.stringify((await folder()).revealed))

  // So does its ⋯ menu, which can now delete it.
  await card.getByRole('button', { name: 'More for Dusk' }).click()
  const items = await page.locator('[role=menu] [role=menuitem]').allTextContents()
  check('its menu has Show in folder and Delete', items.some((t) => /Show in/.test(t)) && items.some((t) => /Delete/.test(t)), items.join(' | '))
  await page.locator('[role=menu]').getByRole('menuitem', { name: 'Delete' }).click()
  await page.waitForSelector('.dialog')
  check('Delete asks first', /Delete the theme “Dusk”/.test((await page.locator('.dialog').textContent()) ?? ''))
  await page.locator('.dialog').getByRole('button', { name: 'Delete' }).click()
  await card.waitFor({ state: 'detached' })
  check('Delete removes the theme’s file', (await folder()).deleted.join() === 'dusk', JSON.stringify((await folder()).deleted))
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('workspace.appearance')))
  check('and stops using it', stored.light !== 'dusk' && stored.dark !== 'dusk', JSON.stringify(stored))
} catch (err) {
  check('run finished', false, String(err?.stack ?? err))
} finally {
  await done(browser, logs)
}
