import { join, resolve } from 'node:path'
import { launch, start, open, check, done, TMP } from './harness.mjs'
const { browser, page, logs } = await launch()
try {
  await start(page)
  await open(page, resolve(import.meta.dirname, '../samples/S2-xlsx-hidden-first.xlsx'))
  await page.waitForFunction(() => window.__sheets?.sheets.length === 3)
  const state = await page.evaluate(() => {
    const e = window.__sheets
    return { sheets: e.sheets, state: e.state, value: e.cell?.value }
  })
  check('both hidden worksheets stay hidden in imported metadata', state.sheets[0].hidden && state.sheets[1].hidden, JSON.stringify(state.sheets))
  check('initial canvas selects the visible worksheet', state.state.sheet === 2, JSON.stringify(state.state))
  check('selection does not mark the workbook unsaved', !state.state.dirty)
  const nav = page.locator('.view[data-active=true] nav[aria-label=Sheets]')
  await nav.waitFor()
  const names = await nav.textContent()
  check('hidden sheet names are absent from navigation', !names.includes('Hidden first') && !names.includes('Very hidden') && names.includes('Visible'), names)
  await nav.getByRole('button', { name: 'Visible', exact: true }).click()
  check('visible sheet still responds', await page.evaluate(() => window.__sheets.state.sheet === 2))
  await page.screenshot({ path: join(TMP, `hidden-sheets-${process.env.ENGINE ?? 'chromium'}.png`) })
} finally { await done(browser, logs) }
