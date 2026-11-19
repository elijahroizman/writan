// Right-clicking a row or column header used to select through mousedown with the focus at the
// sheet's last cell, scrolling the grid to column AZ / the last row before the menu opened.
import { launch, start, check, done } from './harness.mjs'

const { browser, page, logs } = await launch({ tauri: false })
try {
  await start(page)
  await page.evaluate(() => window.__workspace.useWorkspace.getState().newDocument('xlsx'))
  await page.getByRole('textbox', { name: 'Cells', exact: true }).waitFor()
  const scroller = page.locator('.sg-scroller')
  const scrollPos = () => scroller.evaluate((el) => [el.scrollLeft, el.scrollTop])
  const range = () => page.evaluate(() => {
    const r = window.__sheets.range
    return [r.r0, r.c0, r.r1, r.c1]
  })
  const cols = () => page.evaluate(() => window.__sheets.geometryOf(0).info.cols)
  const rows = () => page.evaluate(() => window.__sheets.geometryOf(0).info.rows)

  await page.locator('.sg-rowhead').filter({ hasText: /^5$/ }).click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Delete rows', exact: true }).waitFor()
  let [left, top] = await scrollPos()
  check('right-click on a row header does not scroll', left === 0 && top === 0, `${left},${top}`)
  let r = await range()
  check('right-click on a row header selects the row', r[0] === 4 && r[1] === 0 && r[2] === 4 && r[3] === (await cols()) - 1, r.join(','))
  await page.keyboard.press('Escape')

  await page.locator('.sg-colhead').filter({ hasText: /^C$/ }).click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Delete columns', exact: true }).waitFor()
  ;[left, top] = await scrollPos()
  check('right-click on a column header does not scroll', left === 0 && top === 0, `${left},${top}`)
  r = await range()
  check('right-click on a column header selects the column', r[0] === 0 && r[1] === 2 && r[2] === (await rows()) - 1 && r[3] === 2, r.join(','))
  await page.keyboard.press('Escape')

  // A right-click on a header inside the selection keeps it.
  await page.evaluate(() => window.__sheets.run({ cmd: 'select', sheet: 0, anchor: [2, 0], focus: [6, 51] }))
  await page.locator('.sg-rowhead').filter({ hasText: /^4$/ }).click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Delete rows', exact: true }).waitFor()
  r = await range()
  check('right-click on a selected row header keeps the selection', r[0] === 2 && r[2] === 6, r.join(','))
  ;[left, top] = await scrollPos()
  check('the kept selection still does not scroll', left === 0 && top === 0, `${left},${top}`)
} finally { await done(browser, logs) }
