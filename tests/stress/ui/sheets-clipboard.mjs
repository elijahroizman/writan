import { launch, start, check, done } from './harness.mjs'

const { browser, page, logs } = await launch({ tauri: false })
try {
  await start(page)
  await page.evaluate(() => window.__workspace.useWorkspace.getState().newDocument('xlsx'))
  const grid = page.getByRole('textbox', { name: 'Cells', exact: true })
  await grid.waitFor()
  await grid.focus()
  const run = (command) => page.evaluate((cmd) => window.__sheets.run({ sheet: 0, ...cmd }), command)
  const input = (row, col, text) => run({ cmd: 'input', row, col, text })
  const select = (row, col) => run({ cmd: 'select', anchor: [row, col], focus: [row, col] })
  const cell = (row, col) => page.evaluate(([r, c]) => JSON.parse(window.__sheets.doc.cell(0, r, c)), [row, col])
  const key = (k) => page.keyboard.press(`ControlOrMeta+${k}`)

  await input(1, 1, '125')
  await input(3, 1, '75')
  await input(1, 2, '30')
  await input(4, 1, '=SUM(B2:B4)')
  await select(4, 1)
  await key('c')
  await select(4, 2)
  await key('v')
  const copied = await cell(4, 2)
  check('copy shifts relative references and recalculates against the destination', copied.input === '=SUM(C2:C4)' && copied.display === '30', copied.input)
  check('copy leaves the source formula intact', (await cell(4, 1)).input === '=SUM(B2:B4)')

  await select(4, 1)
  const modifier = process.platform === 'darwin' ? 'Meta' : 'Control'
  await page.keyboard.down(modifier)
  await page.keyboard.down('x')
  const repeatedCut = await grid.evaluate((el) => {
    const data = new DataTransfer()
    el.dispatchEvent(new ClipboardEvent('cut', { bubbles: true, cancelable: true, clipboardData: data }))
    return [data.getData('text/plain'), data.getData('application/x-writan-sheets')]
  })
  await page.keyboard.up('x')
  await page.keyboard.up(modifier)
  check('a second cut event from one keypress keeps the clipboard payload', repeatedCut[0] === '200\n' && repeatedCut[1].includes('SUM(B2:B4)'))
  await select(4, 3)
  await key('v')
  const moved = await cell(4, 3)
  check('cut clears the source', (await cell(4, 1)).input === '')
  check('cut pastes the original formula without changing its references', moved.input === '=SUM(B2:B4)' && moved.display === '200', moved.input)
  await input(1, 1, '225')
  check('the moved formula still recalculates', (await cell(4, 3)).display === '300')
} finally {
  await done(browser, logs)
}
