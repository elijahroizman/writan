#!/usr/bin/env node
// Scroll every Word page so virtualization does not hide text from the audit.
import { chromium, webkit } from 'playwright'
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { resolve, join } from 'node:path'
const root = import.meta.dirname
const out = resolve(process.env.CORPUS_RESULTS || join(root, 'results'))
const base = process.env.CORPUS_URL || 'http://127.0.0.1:1443'
const names = (await readFile(join(root, 'manifest.tsv'), 'utf8')).trim().split('\n').slice(1).map(l => l.split('\t')[0]).filter(n => n.endsWith('.docx') && (!process.env.CORPUS_FILTER || new RegExp(process.env.CORPUS_FILTER).test(n)))
const kind = process.env.CORPUS_BROWSER || 'chromium'
const browser = await ({ chromium, webkit })[kind].launch({ headless: true })
const results = []
for (const name of names) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } })
  const page = await context.newPage()
  await page.addInitScript(() => { localStorage.setItem('workspace.setupDone', '2'); localStorage.setItem('workspace.themeMode', 'light') })
  const result = { name, browser: kind, pages: [] }
  try {
    await page.goto(`${base}/?open=${encodeURIComponent('/dev-samples/' + name)}`)
    await page.waitForSelector('.docx-page', { timeout: 30000 })
    await page.evaluate(() => document.fonts.ready)
    await page.waitForTimeout(150)
    const count = await page.locator('.docx-page').count()
    result.pageCount = count
    const dir = join(out, `pages-${kind}`, name)
    await mkdir(dir, { recursive: true })
    for (let i = 0; i < count; i++) {
      const el = page.locator('.docx-page').nth(i)
      await el.scrollIntoViewIfNeeded()
      await page.waitForTimeout(40)
      const text = await el.innerText()
      result.pages.push({ index: i + 1, text })
      if ([0, Math.floor(count / 2), count - 1].includes(i)) await el.screenshot({ path: join(dir, `page-${i + 1}.png`) })
    }
    result.status = 'ok'
  } catch (e) { result.status = 'failed'; result.error = String(e) }
  results.push(result)
  console.log(name, result.status, result.pageCount)
  await context.close()
}
await browser.close()
await writeFile(join(out, `pages-${kind}.json`), JSON.stringify(results, null, 2) + '\n')
