import { launch, start, check, done, dialogText } from './harness.mjs'

const { browser, page, logs } = await launch({ tauri: false })
try {
  await start(page)
  await page.evaluate(() => window.__workspace.useWorkspace.getState().newDocument('xlsx'))
  await page.getByRole('textbox', { name: 'Cells', exact: true }).waitFor()
  const run = (command) => page.evaluate((cmd) => window.__sheets.run({ sheet: 0, ...cmd }), command)
  const input = (row, col, text) => run({ cmd: 'input', sheet: 0, row, col, text })
  const cell = (row, col) => page.evaluate(([r, c]) => JSON.parse(window.__sheets.doc.cell(0, r, c)), [row, col])
  const readout = page.locator('.view[data-active=true] .canvas-bar .circular')
  const dismiss = () => page.locator('.dialog, [role=dialog], [role=alertdialog]').getByRole('button', { name: 'OK' }).click()

  check('no warning before any formula', (await dialogText(page)) === null && (await readout.count()) === 0)
  await input(0, 0, '=A1+1')
  await page.waitForFunction(() => document.querySelector('.dialog, [role=dialog], [role=alertdialog]'))
  check('a formula that refers to its own cell warns', /Circular reference/.test(await dialogText(page)), await dialogText(page))
  check('it shows 0, not one step round the loop', (await cell(0, 0)).display === '0', (await cell(0, 0)).display)
  check('the canvas bar names the cell', (await readout.textContent()) === 'Circular reference: A1', await readout.textContent())
  await dismiss()

  await input(1, 0, '=A1*2+3')
  check('a formula reading the loop sees 0', (await cell(1, 0)).display === '3', (await cell(1, 0)).display)
  check('an unrelated edit does not warn again', (await dialogText(page)) === null)

  await input(2, 0, '=B3')
  await input(2, 1, '=A3+1')
  await page.waitForFunction(() => document.querySelector('.dialog, [role=dialog], [role=alertdialog]'))
  check('a new loop through two cells warns again', /Circular reference/.test(await dialogText(page)))
  await dismiss()

  await input(0, 0, '5')
  await input(2, 0, '1')
  check('fixing the formulas clears the warning', (await readout.count()) === 0)
  check('and the dependent formula works again', (await cell(1, 0)).display === '13', (await cell(1, 0)).display)
} finally { await done(browser, logs) }
