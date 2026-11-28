#!/usr/bin/env node
// Run on bbb against a forced VITE_TEST_HOOKS build. Pair with corpus/browser.mjs for save/screenshot checks.
import assert from 'node:assert/strict'
import { chromium, webkit } from 'playwright'

const base = process.env.CORPUS_URL || 'http://127.0.0.1:1453'
for (const [kind, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch({ headless: true })
  try {
    for (const [name, sequence] of [['lo-cjklist30.docx', '甲乙丙丁戊己庚辛壬癸'], ['lo-cjklist31.docx', '子丑寅卯辰巳午未申酉戌亥']]) {
      const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } })
      await page.addInitScript(() => localStorage.setItem('workspace.setupDone', '2'))
      await page.goto(`${base}/?open=${encodeURIComponent('/dev-samples/' + name)}`)
      await page.waitForFunction(() => !!window.__docxEditor, null, { timeout: 45000 })
      const expected = [...sequence].map(c => c + '.')
      for (let n = expected.length + 1; n <= 20; n++) expected.push(n + '.')
      const actual = await page.evaluate(expected => {
        const editor = window.__docxEditor, texts = []
        function visit(items) {
          for (const item of items) {
            if (item.t === 'text' && expected.includes(item.text)) texts.push(item.text)
            if (item.items) visit(item.items)
          }
        }
        for (let i = 0; i < editor.sizes.length; i++) visit(editor.paint(i).items)
        return texts
      }, expected)
      assert.deepEqual(actual, expected)
      console.log(JSON.stringify({ browser: kind, name, url: page.url(), title: await page.title(), markers: actual, status: 'pass' }))
      await page.close()
    }
  } finally {
    await browser.close()
  }
}
