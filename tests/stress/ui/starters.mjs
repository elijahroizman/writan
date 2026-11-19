// New presentation opens the starter gallery: every deck draws its title slide, the keyboard
// picks one, and the deck opens with its slides and theme. SHOTS=dir saves screenshots.
import { launch, start, check, done } from './harness.mjs'

const shots = process.env.SHOTS
const { browser, page, logs } = await launch({ tauri: false })
try {
  await start(page)
  await page.getByRole('button', { name: 'New presentation' }).first().click()
  const gallery = page.getByRole('dialog', { name: 'New presentation' })
  await gallery.waitFor()
  const options = gallery.getByRole('option')
  check('the gallery lists the starter decks', (await options.count()) === 6, String(await options.count()))
  await page.waitForFunction(() => document.querySelectorAll('.starter .slide-surface').length === 6)
  check('every starter draws its title slide', (await gallery.locator('.starter .slide-surface').count()) === 6)
  check('Plain is selected first', (await gallery.getByRole('option', { name: 'Plain', exact: true, selected: true }).count()) === 1)
  if (shots) await gallery.screenshot({ path: `${shots}/gallery.png` })

  // Return picks the highlighted tile, so focus stays on the list.
  const listFocused = () => page.evaluate(() => document.activeElement?.getAttribute('role') === 'listbox')
  await page.keyboard.press('Tab')
  const afterTab = await listFocused()
  await page.keyboard.press('Shift+Tab')
  check('Tab and Shift+Tab keep focus on the list', afterTab && (await listFocused()))

  // Harbor is second: one step right, then return.
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('Enter')
  await gallery.waitFor({ state: 'detached' })
  await page.waitForFunction(() => window.__slides?.slides?.length === 6)
  const deck = await page.evaluate(() => ({ slides: window.__slides.slides.length, theme: window.__slides.theme }))
  check('the picked deck opens with its six slides', deck.slides === 6, JSON.stringify(deck.slides))
  check('the deck has the starter theme', deck.theme.heading === 'Calibri Light' && deck.theme.body === 'Calibri', JSON.stringify(deck.theme))
  const tab = await page.evaluate(() => {
    const ws = window.__workspace.useWorkspace.getState()
    const t = ws.tabs.find((t) => t.id === ws.activeId)
    return { title: t.title, path: t.path, dirty: t.dirty }
  })
  check('it opens as an untitled, unchanged presentation', tab.title.startsWith('Untitled Presentation') && tab.path === null && !tab.dirty, JSON.stringify(tab))

  // The gallery remembers the last pick.
  const tabs = () => page.evaluate(() => window.__workspace.useWorkspace.getState().tabs.length)
  const before = await tabs()
  await page.evaluate(() => window.__workspace.useWorkspace.getState().activate('home'))
  await page.getByRole('button', { name: 'New presentation' }).first().click()
  await gallery.waitFor()
  check('the gallery starts on the deck used last', (await gallery.getByRole('option', { name: 'Harbor', exact: true, selected: true }).count()) === 1)
  await page.keyboard.press('Escape')
  await gallery.waitFor({ state: 'detached' })
  check('Escape closes the gallery without a new tab', (await tabs()) === before)
} catch (e) {
  check('starter gallery flow', false, e.message)
} finally {
  await done(browser, logs)
}
