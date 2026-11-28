#!/usr/bin/env node
// Inspect actual engine display lists for the report's representative files.
import { chromium, webkit } from 'playwright'
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { join } from 'node:path'
const root = import.meta.dirname
const names = ['au-pbo-PBO---Build-your-own-budget---2026-27-Budget.xlsx', 'pa-budgetprep-issue-form.xlsx', 'lo-conditional_fmt_checkpriority.xlsx', 'ucsd-ThesisTemplate_Master_v2.3.docx', 'au-budget-bp1_bs-4.docx', 'lo-footnote.docx', 'lo-math-accents.docx', 'lo-smartart.docx', 'lo-table-rtl.docx', 'lo-cjklist30.docx', 'lo-cjklist31.docx', 'lo-smartart-accent-process.pptx', 'lo-chart_pt_color_bg1.pptx']
const out = []
for (const [kind, type] of Object.entries({ chromium, webkit })) {
  const browser = await type.launch({ headless: true })
  for (const name of names) {
    const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } })
    const page = await context.newPage()
    await page.addInitScript(() => localStorage.setItem('workspace.setupDone', '2'))
    const row = { name, browser: kind }
    try {
      await page.goto('http://127.0.0.1:1443/?open=' + encodeURIComponent('/dev-samples/' + name))
      await page.waitForFunction(name => !!(name.endsWith('.docx') ? window.__docxEditor : name.endsWith('.pptx') ? window.__slides : window.__sheets), name, { timeout: 45000 })
      row.engine = await page.evaluate(name => {
        const counts = {}, text = []
        function visit(items) {
          for (const item of items) {
            counts[item.t] = (counts[item.t] || 0) + 1
            if (item.t === 'text') text.push(item.text)
            if (item.items) visit(item.items)
          }
        }
        if (name.endsWith('.docx')) {
          const e = window.__docxEditor
          for (let i = 0; i < e.sizes.length; i++) visit(e.paint(i).items)
          return { pageCount: e.sizes.length, counts, text: text.join(' ') }
        }
        if (name.endsWith('.pptx')) {
          const e = window.__slides
          for (let i = 0; i < e.slides.length; i++) visit(JSON.parse(e.doc.paint(i)).items)
          return { slides: e.slides.length, counts, text: text.join(' ') }
        }
        const e = window.__sheets
        if (name.startsWith('au-pbo')) return { sheets: e.sheets, activeSheet: e.state.sheet, cellI38: JSON.parse(e.doc.cell(4, 37, 8)), dirty: e.state.dirty }
        return { sheets: e.sheets, activeSheet: e.state.sheet, cellA1: JSON.parse(e.doc.cell(0, 0, 0)), cellA3: JSON.parse(e.doc.cell(0, 2, 0)) }
      }, name)
      row.status = 'ok'
    } catch (e) { row.status = 'failed'; row.error = String(e) }
    out.push(row)
    console.log(kind, name, row.status)
    await context.close()
  }
  await browser.close()
}
await writeFile(join(root, 'results/verification.json'), JSON.stringify(out, null, 2) + '\n')
