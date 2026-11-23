// Many open tabs: Home and the tabs keep their size, and the tabs scroll instead of squeezing.
import { launch, start, check, done } from './harness.mjs'

const { browser, page, logs } = await launch({ tauri: false, viewport: { width: 1000, height: 700 } })
try {
  await start(page)
  for (let i = 0; i < 14; i++) await page.evaluate(() => window.__workspace.useWorkspace.getState().newDocument('docx'))
  await page.waitForFunction(() => window.__workspace.useWorkspace.getState().tabs.length === 14)
  await page.waitForTimeout(300)

  const home = page.getByRole('tab', { name: 'Home' })
  const homeTitle = home.locator('.tab-title')
  check('Home is not cut off', await homeTitle.evaluate((el) => el.scrollWidth <= el.clientWidth && el.getBoundingClientRect().width > 20))
  const titles = await page.locator('.tab:not(.tab-home) .tab-title').evaluateAll((els) => els.map((el) => el.getBoundingClientRect().width))
  check('document tabs keep readable titles', titles.every((w) => w > 60), JSON.stringify(titles.map(Math.round)))

  const strip = page.locator('.tabs-scroll')
  const scroll = () => strip.evaluate((el) => ({ left: el.scrollLeft, max: el.scrollWidth - el.clientWidth }))
  check('the tabs overflow and can scroll', (await scroll()).max > 0)
  check('the New button stays in view', await page.locator('.new-tab-btn').evaluate((el) => el.getBoundingClientRect().right <= window.innerWidth))

  // The newest tab is active, so it's scrolled into view.
  const inView = (locator) =>
    locator.evaluate((el) => {
      const r = el.getBoundingClientRect()
      const s = el.closest('.tabs-scroll').getBoundingClientRect()
      return r.left >= s.left - 1 && r.right <= s.right + 1
    })
  const tabs = page.locator('.tab:not(.tab-home)')
  check('the active last tab is in view', await inView(tabs.last()))

  await page.evaluate(() => {
    const ws = window.__workspace.useWorkspace.getState()
    ws.activate(ws.tabs[0].id)
  })
  await page.waitForTimeout(300)
  check('activating the first tab scrolls it into view', await inView(tabs.first()))
  await strip.evaluate((el) => (el.scrollLeft = el.scrollWidth / 2))
  check('Home stays in view while the tabs scroll', await home.evaluate((el) => {
    const r = el.getBoundingClientRect()
    return r.left >= 0 && r.right <= document.querySelector('.tabs-scroll').getBoundingClientRect().left + 1
  }))
  await strip.evaluate((el) => (el.scrollLeft = 0))

  // A mouse wheel scrolls the tabs sideways.
  const before = (await scroll()).left
  await strip.hover()
  await page.mouse.wheel(0, 400)
  await page.waitForTimeout(300)
  check('the mouse wheel scrolls the tabs', (await scroll()).left > before)

  // Dragging a tab past its neighbour still reorders them in the scrolled strip.
  const order = () => page.evaluate(() => window.__workspace.useWorkspace.getState().tabs.map((t) => t.id))
  const visible = await tabs.evaluateAll((els) => {
    const s = els[0].closest('.tabs-scroll').getBoundingClientRect()
    return els.findIndex((el) => el.getBoundingClientRect().left >= s.left)
  })
  const ids = await order()
  const a = await tabs.nth(visible).boundingBox()
  const b = await tabs.nth(visible + 1).boundingBox()
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2)
  await page.mouse.down()
  await page.mouse.move(a.x + a.width / 2 + 10, a.y + a.height / 2, { steps: 2 })
  await page.mouse.move(b.x + b.width * 0.75, b.y + b.height / 2, { steps: 5 })
  await page.mouse.up()
  const after = await order()
  check('dragging a tab past the next one swaps them', after[visible] === ids[visible + 1] && after[visible + 1] === ids[visible], `${visible} ${JSON.stringify(ids)} ${JSON.stringify(after)}`)

  await page.screenshot({ path: '/tmp/writan-stress/tab-overflow.png' })
} finally {
  await done(browser, logs)
}
