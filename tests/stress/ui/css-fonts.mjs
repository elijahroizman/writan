// Font names from a document go into a style sheet; none may restyle the app around it.
// node tests/stress/ui/css-fonts.mjs
import { join, resolve } from 'node:path'
import { launch, start, open, check, done, TMP } from './harness.mjs'

const S = resolve(import.meta.dirname, '../samples')
const { browser, page, logs } = await launch()
await start(page)
for (const f of ['S2-docx-css-font-names.docx', 'S2-pptx-css-font-names.pptx', 'S2-xlsx-css-font-names.xlsx']) {
  await open(page, join(S, f))
  let ready = false
  for (let attempt = 0; attempt < 200; attempt++) {
    ready = await page.evaluate(async () => {
      const w = window.__workspace
      const s = w.useWorkspace.getState()
      try { return (await w.getEditor(s.activeId)?.getBytes())?.length > 0 } catch { return false }
    })
    if (ready) break
    await page.waitForTimeout(100)
  }
  check(`${f}: editor becomes ready`, ready)
  check(`${f}: document still saves`, await page.evaluate(async () => {
    const s = window.__workspace.useWorkspace.getState()
    return s.save(s.activeId, { quiet: true })
  }))
  await page.screenshot({ path: join(TMP, `css-${f}.png`) })
  const look = await page.evaluate(() => ({
    app: getComputedStyle(document.querySelector('.app') ?? document.body).display,
    bodyBg: getComputedStyle(document.body).backgroundColor,
    tabsVisible: !!document.querySelector('.tab')?.getClientRects().length,
    hidden: [...document.querySelectorAll('body *')].filter((e) => getComputedStyle(e).display === 'none').length,
  }))
  check(`${f}: the app still shows`, look.app !== 'none' && look.tabsVisible && look.bodyBg !== 'rgb(255, 0, 0)', JSON.stringify(look))
  const rules = await page.evaluate(() => [...document.styleSheets].flatMap((s) => { try { return [...s.cssRules] } catch { return [] } }).filter((r) => r.selectorText && /^(\*|body|\.app)$/.test(r.selectorText.trim())).map((r) => r.cssText))
  check(`${f}: no rules for the whole app came from the document`, !rules.some((r) => /display: none|background: red/.test(r)), rules.join(' | ').slice(0, 300))
}
await page.screenshot({ path: join(TMP, `css-final-${process.env.ENGINE ?? 'chromium'}.png`) })
check('font documents do not trigger core borrow errors', !logs.some((l) => /recursive use|while it was borrowed|unsafe aliasing/.test(l)))
check('no script ran', !logs.some((l) => l.includes('native-dialog')))
await done(browser, logs)
