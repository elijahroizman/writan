import assert from 'node:assert/strict'
import { launch, start, open, done } from './harness.mjs'

const { browser, page, logs } = await launch()
try {
  await start(page)
  await page.evaluate(() => window.__workspace.useWorkspace.getState().newDocument('xlsx'))
  const nav = page.getByRole('navigation', { name: 'Sheets' })
  const original = nav.getByRole('button', { name: 'Sheet1', exact: true })
  await nav.getByRole('button', { name: 'Add sheet' }).click()

  await original.click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Rename…', exact: true }).click()
  const field = nav.getByRole('textbox', { name: 'Sheet name' })
  await field.waitFor()
  assert.equal(await field.inputValue(), 'Sheet1')
  assert.equal(await field.evaluate((el) => el === document.activeElement), true)
  await page.keyboard.type('Quarterly Plan')
  await field.press('Enter')
  const renamed = nav.getByRole('button', { name: 'Quarterly Plan', exact: true })
  await renamed.waitFor()
  assert.equal(await renamed.evaluate((el) => el === document.activeElement), true)
  assert.equal(await nav.locator('.list-item.active').textContent(), 'Sheet2')

  await renamed.dblclick()
  await field.fill('Discard this name')
  await field.press('Escape')
  await renamed.waitFor()
  assert.equal(await renamed.evaluate((el) => el === document.activeElement), true)

  const path = '/tmp/writan-stress/renamed-sheet.xlsx'
  await page.evaluate(async (path) => {
    const workspace = window.__workspace.useWorkspace.getState()
    if (!(await workspace.save(undefined, { to: path, quiet: true }))) throw new Error('Workbook was not saved')
    if (!(await workspace.closeTab())) throw new Error('Workbook was not closed')
  }, path)
  await open(page, path)
  await renamed.waitFor()
  assert.equal(await original.count(), 0)
} finally { await done(browser, logs) }
