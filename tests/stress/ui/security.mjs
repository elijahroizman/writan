// Documents must never run script or steer the app's web view; links open only http(s) and mailto.
// node tests/stress/ui/security.mjs   (ENGINE=webkit for WebKit)
import { readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { launch, start, open, tabs, check, done, TMP } from './harness.mjs'

const S = resolve(import.meta.dirname, '../samples')
const { browser, page, logs } = await launch({
  // Any script a document manages to run calls one of these.
  init: () => {
    window.__xss = []
    const hit = (what) => (...a) => window.__xss.push(`${what}(${a.join(',')})`)
    window.alert = hit('alert')
    window.confirm = hit('confirm')
    window.prompt = hit('prompt')
  },
})
await start(page)
const startUrl = page.url()
const mod = process.platform === 'darwin' ? 'Meta' : 'Control'
const xss = () => page.evaluate(() => window.__xss)
// Refused links show the app's own message; close each one.
let refused = 0
const dismiss = async () => {
  await page.waitForTimeout(150)
  while (await page.locator('.dialog').count()) {
    refused++
    await page.locator('.dialog button').last().click()
    await page.waitForTimeout(100)
  }
}
const opened = async () => {
  await dismiss()
  return page.evaluate(() => window.__fake.opened.splice(0))
}
const allowed = (u) => /^(https?:|mailto:)/i.test(u)

async function activeView() {
  return page.locator('.view[data-active=true]')
}

// ── Word
await open(page, join(S, 'S1-docx-evil-links.docx'))
await page.waitForSelector('.view[data-active=true] .ln[data-link]', { timeout: 20000 })
const links = await page.locator('.view[data-active=true] .ln[data-link]').all()
for (const l of links) {
  await l.click({ modifiers: [mod] }).catch(() => {})
  await dismiss()
}
await page.waitForTimeout(500)
let o = await opened()
check('docx: only http(s)/mailto links reach the opener', o.every(allowed), JSON.stringify(o))
check('docx: the https link still opens', o.includes('https://example.com/ok'))

// ── Word with an SVG picture
await open(page, join(S, 'S1-docx-svg-image.docx'))
await page.waitForTimeout(2500)

// ── File named with HTML
// (Injected into fake bridge: Windows NTFS forbids < and > in real file names.)
const evilName = join(TMP, 'S1-name-<img src=x onerror=alert(1)>.docx')
const evilB64 = readFileSync(join(S, 'S1-docx-evil-links.docx')).toString('base64')
await page.evaluate(([p, b64]) => {
  window.__fake.files.set(p, Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)))
}, [evilName, evilB64])
await open(page, evilName)
await page.waitForTimeout(1500)
check('html in a file name shows as text', (await page.locator('.tab img[src=x]').count()) === 0)

// ── PowerPoint: present and click each linked box
await open(page, join(S, 'S1-pptx-evil-links.pptx'))
await page.waitForFunction(() => window.__slides, null, { timeout: 20000 })
await page.waitForTimeout(1500)
await page.evaluate(() => { const w = window.__workspace; w.getEditor(w.useWorkspace.getState().activeId).actions['pptx.present-start']() })
await page.waitForTimeout(800)
const presenting = await page.locator('.present').count()
if (presenting) {
  const box = await page.locator('.present').boundingBox()
  const els = await page.evaluate(() => {
    const e = window.__slides
    return { w: e.width, h: e.height, els: JSON.parse(e.doc.elements(0)).map((x) => ({ x: x.x, y: x.y, w: x.width, h: x.height, url: x.link?.url })) }
  })
  const scale = Math.min(box.width / els.w, box.height / els.h)
  const left = (box.width - els.w * scale) / 2
  const top = (box.height - els.h * scale) / 2
  for (const el of els.els.filter((e) => e.url)) {
    await page.mouse.click(box.x + left + (el.x + el.w / 2) * scale, box.y + top + (el.y + el.h / 2) * scale)
    await dismiss()
  }
  await page.keyboard.press('Escape')
  o = await opened()
  check('pptx present: only http(s)/mailto links reach the opener', o.every(allowed), JSON.stringify(o))
} else check('pptx present mode opened', false)

// ── Excel and CSV with HTML in cells, sheet names and comments
await open(page, join(S, 'S1-xlsx-html-cells.xlsx'))
await page.waitForTimeout(2500)
check('xlsx: no injected <img>', (await page.locator('img[src=x]').count()) === 0)
await open(page, join(S, 'S1-csv-html-cells.csv'))
await page.waitForTimeout(1500)

// ── PDF links, bookmarks and JavaScript actions
await open(page, join(S, 'S1-pdf-evil-links.pdf'))
await page.waitForTimeout(4000)
const pdfLinks = await page.locator('.view[data-active=true] [data-annot] a, .view[data-active=true] .pdf-link, .view[data-active=true] a[href]').all()
for (const l of pdfLinks) {
  await l.click().catch(() => {})
  await dismiss()
}
await page.waitForTimeout(300)
o = await opened()
check('pdf: only http(s)/mailto links reach the opener', o.every(allowed), JSON.stringify(o))

// ── Markdown
await open(page, join(S, 'S1-markdown-xss.md'))
await page.waitForSelector('.view[data-active=true] .md-body', { timeout: 15000 })
await page.waitForTimeout(800)
const body = page.locator('.view[data-active=true] .md-body')
const html = await body.innerHTML()
check('markdown: no script/iframe/object/embed/base/meta in the preview', !/<(script|iframe|object|embed|base|meta)\b/i.test(html))
const onAttrs = await body.evaluate((el) => [...el.querySelectorAll('*')].flatMap((n) => [...n.attributes].filter((a) => /^on/i.test(a.name)).map((a) => `${n.tagName}.${a.name}`)))
check('markdown: no on* attributes', onAttrs.length === 0, onAttrs.join(' '))
check('markdown: no javascript:/vbscript:/data: hrefs', !/href="\s*(javascript|vbscript|data):/i.test(html), (html.match(/href="[^"]*"/g) ?? []).join(' '))
for (const a of await body.locator('a').all()) {
  await a.click({ timeout: 1000 }).catch(() => {})
  await dismiss()
}
await page.waitForTimeout(500)
o = await opened()
check('markdown: only http(s)/mailto links reach the opener', o.every(allowed), JSON.stringify(o))

const t = await tabs(page)
check('markdown links did not open stray tabs for files outside', !t.some((x) => /passwd/.test(x.path ?? '')), t.map((x) => x.path).join(', '))
check('refused links said so', refused >= 16, String(refused))
check('no script ran', (await xss()).length === 0, JSON.stringify(await xss()))
check('the app never navigated', page.url() === startUrl, page.url())
check('no unhandled errors', !logs.some((l) => l.startsWith('[pageerror]')))
await done(browser, logs)
