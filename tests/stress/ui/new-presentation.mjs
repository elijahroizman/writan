// New presentation makes a plain deck right away: a title slide and a content slide, no
// template gallery. The Templates button, the "from a template" items in the + menu, New… and
// the command palette are gone.
import { launch } from './harness.mjs'
import { join } from 'node:path'

const { browser, page, logs } = await launch()
const errors = []
const check = (name, ok, detail = '') => {
  console.log(`${ok ? 'ok' : 'FAIL'} ${name}${ok ? '' : ` — ${detail}`}`)
  if (!ok) errors.push(name)
}
const shot = async (name) => {
  if (process.env.SLIDE_THEME_SCREENSHOTS) await page.screenshot({ path: join(process.env.SLIDE_THEME_SCREENSHOTS, `${name}.png`) })
}

try {
  await page.goto(process.env.WRITAN_URL ?? 'http://localhost:1420/')
  await page.waitForSelector('.home', { timeout: 30000 })

  check('no Templates button on Home', (await page.getByRole('button', { name: 'Templates', exact: true }).count()) === 0)

  // Home's New row makes the deck directly.
  await page.getByRole('button', { name: 'New presentation', exact: true }).first().click()
  await page.waitForFunction(() => window.__slides?.slides?.length === 2, null, { timeout: 30000 })
  const texts = await page.evaluate(() => [0, 1].map((i) => JSON.parse(window.__slides.doc.elements(i)).map((e) => e.text).join('\n')))
  check('a new presentation has a title slide and a content slide', texts[0].includes('Presentation title') && texts[1].includes('First point'), JSON.stringify(texts))
  const plain = await page.evaluate(() => ({
    theme: window.__slides.theme, layoutCount: window.__slides.layouts.length, title: window.__slides.elements[0],
  }))
  check('the new deck uses the one-layout plain design', plain.layoutCount === 1 && !plain.title.placeholder, JSON.stringify(plain))
  await shot('01-default')

  const theme = page.getByRole('button', { name: 'Theme', exact: true })
  const themeItems = page.getByRole('menu').getByRole('menuitem')
  await theme.click()
  const options = await themeItems.allTextContents()
  check('the in-deck Design control offers Default and all nine themes', options.join(', ') === 'Default, Harbor, Summit, Grove, Copper, Graphite, Meridian, Standard, Gallery, Dune', JSON.stringify(options))
  await shot('02-theme-menu')
  await themeItems.getByText('Harbor', { exact: true }).click()
  await page.waitForFunction(() => window.__slides.theme.name === 'Harbor')
  const harbor = await page.evaluate(() => {
    const deck = window.__slides
    return {
      theme: deck.theme,
      title: deck.elements[0],
      layouts: deck.slides.map((s) => deck.layouts.find((l) => l.path === s.layout)?.name),
      texts: deck.slides.map((_, i) => JSON.parse(deck.doc.elements(i)).map((e) => e.text).join('\n')),
    }
  })
  check('Harbor replaces the theme and preserves text on matching layouts', harbor.theme.colors.accent1 !== plain.theme.colors.accent1 &&
    Boolean(harbor.title.placeholder) && harbor.title.x !== plain.title.x &&
    harbor.layouts.join(', ') === 'Title Slide, Title and Content' && JSON.stringify(harbor.texts) === JSON.stringify(texts), JSON.stringify(harbor))
  await shot('03-harbor')

  await theme.click()
  await themeItems.getByText('Graphite', { exact: true }).click()
  await page.waitForFunction(() => window.__slides.theme.name === 'Graphite')
  await shot('04-graphite')
  await theme.click()
  await themeItems.getByText('Default', { exact: true }).click()
  const restored = await page.evaluate(() => ({ theme: window.__slides.theme, layouts: window.__slides.layouts.length, title: window.__slides.elements[0] }))
  check('Default restores the plain title and theme', restored.theme.name === plain.theme.name && restored.layouts === 1 &&
    restored.title.x === plain.title.x && restored.title.text === plain.title.text, JSON.stringify(restored))

  // The + menu has no template entry.
  await page.getByRole('tab', { name: 'Home' }).click()
  await page.locator('button[aria-label="New"]').click()
  const menu = page.locator('[role="menu"]')
  await menu.waitFor()
  check('the + menu has no template item', (await menu.getByText(/template/i).count()) === 0)
  check('the + menu still lists New presentation', (await menu.getByText('New presentation').count()) === 1)
  await page.keyboard.press('Escape')

  // ⌘N lists kinds only.
  await page.keyboard.press('ControlOrMeta+n')
  const picker = page.getByRole('dialog', { name: 'New file' })
  await picker.waitFor()
  check('New… has no template item', (await picker.getByText(/template/i).count()) === 0)
  await page.keyboard.press('Escape')

  // ⌘K: searching "template" finds nothing.
  await page.keyboard.press('ControlOrMeta+k')
  const palette = page.getByRole('dialog', { name: 'Command palette' })
  await palette.waitFor()
  await page.keyboard.type('template')
  // Two frames is enough for the list to recompute on the keystroke.
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))))
  check('the palette has no template command', (await palette.getByText(/from a template/i).count()) === 0)
  await page.keyboard.press('Escape')
} catch (e) {
  check('new presentation flow', false, e.message)
}

for (const l of logs) console.log(' ', l)
await browser.close()
process.exit(errors.length ? 1 : 0)
