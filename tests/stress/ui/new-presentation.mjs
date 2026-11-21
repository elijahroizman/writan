// New presentation makes a plain deck right away: a title slide and a content slide, no
// template gallery. The Templates button, the "from a template" items in the + menu, New… and
// the command palette are gone.
import { launch } from './harness.mjs'

const { browser, page, logs } = await launch()
const errors = []
const check = (name, ok, detail = '') => {
  console.log(`${ok ? 'ok' : 'FAIL'} ${name}${ok ? '' : ` — ${detail}`}`)
  if (!ok) errors.push(name)
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
