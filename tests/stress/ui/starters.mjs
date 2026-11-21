// New presentation makes a deck from the default starter without asking. "From a template" opens
// the gallery: tiles draw each deck's title slide, search filters them, the preview shows the
// selected deck's slides, and the keyboard creates one or makes it the default.
// SHOTS=dir saves screenshots.
import { launch, start, check, done } from './harness.mjs'

const shots = process.env.SHOTS
const { browser, page, logs } = await launch({ tauri: false, scrollbars: true })
const ws = (f) => page.evaluate(f)
const tabs = () => ws(() => window.__workspace.useWorkspace.getState().tabs.length)
const home = () => ws(() => window.__workspace.useWorkspace.getState().activate('home'))
async function openedDeck(before) {
  await page.waitForFunction((n) => window.__workspace.useWorkspace.getState().tabs.length === n + 1, before)
  await page.waitForFunction(() => window.__slides?.slides?.length > 0)
  return ws(() => {
    const s = window.__workspace.useWorkspace.getState()
    const t = s.tabs.find((t) => t.id === s.activeId)
    return { slides: window.__slides.slides.length, heading: window.__slides.theme.heading, title: t.title, path: t.path, dirty: t.dirty }
  })
}

try {
  await start(page)
  const gallery = page.getByRole('dialog', { name: 'Choose a template' })

  // The plain way: no gallery, the default deck.
  let before = await tabs()
  await page.getByRole('button', { name: 'New presentation', exact: true }).first().click()
  let deck = await openedDeck(before)
  check('New presentation opens without a gallery', (await gallery.count()) === 0)
  check('it starts from Plain', deck.slides === 6 && deck.heading === 'Arial', JSON.stringify(deck))
  check('it opens as an untitled, unchanged presentation', deck.title.startsWith('Untitled Presentation') && deck.path === null && !deck.dirty, JSON.stringify(deck))

  await home()
  if (shots) await page.locator('.home-new').screenshot({ path: `${shots}/home-new.png` })
  await page.getByRole('button', { name: 'Templates', exact: true }).click()
  await gallery.waitFor()
  const options = gallery.getByRole('option')
  check('the gallery lists the starter decks', (await options.count()) === 6, String(await options.count()))
  await page.waitForFunction(() => document.querySelectorAll('.starter .slide-surface').length === 6)
  check('every starter draws its title slide', (await gallery.locator('.starter .slide-surface').count()) === 6)
  check('Plain is selected first', (await gallery.getByRole('option', { name: 'Plain', exact: true, selected: true }).count()) === 1)
  check('Plain is marked as the default', (await gallery.locator('.starter.on .starter-default').count()) === 1)
  check('Blank is told apart from Plain', (await gallery.locator('#starter-blank .starter-blank-mark').count()) === 1)
  check('the search box has focus', await ws(() => document.activeElement?.getAttribute('aria-label') === 'Search templates'))
  await page.waitForFunction(() => document.querySelectorAll('.starter-strip .slide-surface').length === 5)
  check('the preview shows the rest of the deck', (await gallery.locator('.starter-strip .slide-surface').count()) === 5)
  if (shots) {
    await gallery.screenshot({ path: `${shots}/gallery.png` })
    await page.emulateMedia({ colorScheme: 'dark' })
    await ws(() => window.__workspace.useWorkspace.getState().setThemeMode('dark'))
    await page.screenshot({ path: `${shots}/gallery-dark.png` })
    await ws(() => window.__workspace.useWorkspace.getState().setThemeMode('light'))
  }
  // The smallest window (720 × 460, tauri.conf.json) and a short, wide one keep Create on screen.
  for (const size of [{ width: 720, height: 460 }, { width: 980, height: 460 }]) {
    await page.setViewportSize(size)
    await page.waitForTimeout(100)
    const create = gallery.getByRole('button', { name: 'Create' })
    await create.scrollIntoViewIfNeeded()
    const box = await create.boundingBox()
    // The drawn slide ends inside the gallery's content box, scrollbar and padding excluded.
    const fit = await ws(() => {
      const picker = document.querySelector('.starter-picker')
      const r = picker.getBoundingClientRect()
      const right = r.left + picker.clientLeft + picker.clientWidth - parseFloat(getComputedStyle(picker).paddingRight)
      const slide = document.querySelector('.starter-preview-main .slide-preview').getBoundingClientRect()
      return { slide: slide.right, right }
    })
    check(`at ${size.width}×${size.height} Create is reachable and the preview fits`, box && box.y + box.height <= size.height && fit.slide <= fit.right + 0.5, JSON.stringify({ box, fit }))
    if (shots) await page.screenshot({ path: `${shots}/gallery-${size.width}x${size.height}.png` })
  }
  await page.setViewportSize({ width: 1400, height: 900 })

  // Focus goes round the search box and the buttons without leaving the gallery.
  const focusInGallery = () => ws(() => !!document.activeElement?.closest('.starter-picker'))
  let inside = true
  for (const key of ['Shift+Tab', 'Shift+Tab', 'Shift+Tab', 'Tab', 'Tab', 'Tab', 'Tab']) {
    await page.keyboard.press(key)
    inside &&= await focusInGallery()
  }
  check('Tab and Shift+Tab stay in the gallery', inside)
  await gallery.getByRole('combobox', { name: 'Search templates' }).focus()

  // Search: a tag finds the dark deck; Escape clears the search before it closes anything.
  await page.keyboard.type('dark')
  check('search filters by what a deck is like', (await options.count()) === 1 && (await gallery.getByRole('option', { name: 'Graphite', selected: true }).count()) === 1)
  if (shots) await gallery.screenshot({ path: `${shots}/gallery-search.png` })
  await page.keyboard.press('ArrowLeft')
  const caret = await ws(() => document.activeElement.selectionStart)
  check('with a query, left moves the caret, not the selection', caret === 3 && (await gallery.getByRole('option', { name: 'Graphite', selected: true }).count()) === 1, String(caret))
  await page.keyboard.press('End')
  await page.keyboard.type('zzz')
  check('no match says so', (await options.count()) === 0 && (await gallery.getByText('No templates match').count()) === 1)
  await page.keyboard.press('Escape')
  check('Escape clears the search first', (await gallery.count()) === 1 && (await options.count()) === 6)

  // Harbor is second: one step right, make it the default, then return.
  await page.keyboard.press('ArrowRight')
  check('arrows move the selection', (await gallery.getByRole('option', { name: 'Harbor', selected: true }).count()) === 1)
  await gallery.getByRole('button', { name: 'Make default' }).click()
  check('Make default moves the badge', (await gallery.locator('#starter-harbor .starter-default').count()) === 1 && (await gallery.getByText('starts from Harbor').count()) === 1)
  before = await tabs()
  await page.keyboard.press('Enter')
  await gallery.waitFor({ state: 'detached' })
  deck = await openedDeck(before)
  check('return creates the selected deck', deck.slides === 6 && deck.heading === 'Calibri Light', JSON.stringify(deck))

  // Double-clicking a tile creates it too.
  await home()
  await page.getByRole('button', { name: 'Templates', exact: true }).click()
  await gallery.waitFor()
  check('the gallery starts on the default', (await gallery.getByRole('option', { name: 'Harbor', exact: true, selected: true }).count()) === 1)
  before = await tabs()
  await gallery.getByRole('option', { name: 'Blank' }).dblclick()
  await gallery.waitFor({ state: 'detached' })
  deck = await openedDeck(before)
  check('double-click creates that deck', deck.slides === 1, JSON.stringify(deck))

  // New presentation now uses Harbor, still without asking.
  await home()
  before = await tabs()
  await page.getByRole('button', { name: 'New presentation', exact: true }).first().click()
  deck = await openedDeck(before)
  check('New presentation uses the new default', deck.heading === 'Calibri Light' && (await gallery.count()) === 0, JSON.stringify(deck))

  // ⌘N offers both.
  await home()
  await page.keyboard.press('ControlOrMeta+n')
  const picker = page.getByRole('dialog', { name: 'New file' })
  await picker.waitFor()
  check('New… lists the template gallery', (await picker.getByRole('option', { name: /New presentation from a template/ }).count()) === 1)
  await picker.getByRole('option', { name: /New presentation from a template/ }).click()
  await gallery.waitFor()
  before = await tabs()
  await page.keyboard.press('Escape')
  await gallery.waitFor({ state: 'detached' })
  check('Escape closes the gallery without a new tab', (await tabs()) === before)
} catch (e) {
  check('starter gallery flow', false, e.message)
} finally {
  await done(browser, logs)
}
