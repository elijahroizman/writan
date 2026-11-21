// "Save flattened copy" on a two-page PDF with a faked Tauri backend: a page with nothing to
// flatten doesn't fail the export, highlights end up in the copy's page content (also ones saved
// by other apps without the Print flag or a drawing of their own), and cancelling the export
// leaves the original alone. Runs against a VITE_TEST_HOOKS build.
//   node tests/stress/native/pdf-flatten.mjs http://localhost:4173/ [chromium|webkit]
import assert from 'node:assert/strict'
import { inflateSync } from 'node:zlib'
import * as pw from 'playwright'

const url = process.argv[2] ?? 'http://localhost:4173/'
const kind = process.argv[3] ?? 'chromium'

/** Two pages of text; `annots` are dictionaries put on page 1, with `extra` objects numbered from 8. */
function twoPagePdf({ annots = [], extra = [] } = {}) {
  const page = (contents, list = []) => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 7 0 R >> >> /Contents ${contents} 0 R${list.length ? ` /Annots [${list.join(' ')}]` : ''} >>`
  const text = (t) => {
    const s = `BT /F1 24 Tf 72 700 Td (${t}) Tj ET`
    return `<< /Length ${s.length} >>\nstream\n${s}\nendstream`
  }
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>',
    page(4, annots.map((_, i) => `${8 + extra.length + i} 0 R`)),
    text('Page one has some words'),
    page(6),
    text('Page two has other words'),
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
    ...extra,
    ...annots,
  ]
  let out = '%PDF-1.7\n'
  const offsets = objects.map((o, i) => {
    const at = out.length
    out += `${i + 1} 0 obj\n${o}\nendobj\n`
    return at
  })
  const xref = out.length
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((o) => `${String(o).padStart(10, '0')} 00000 n \n`).join('')}`
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`
  return Buffer.from(out, 'latin1')
}

/** How many XObjects the pages draw: flattening draws a page's annotations through one, which draws each one. */
function drawnXObjects(pdf) {
  const text = pdf.toString('latin1')
  let count = 0
  for (const m of text.matchAll(/stream\r?\n([\s\S]*?)endstream/g)) {
    const raw = Buffer.from(m[1], 'latin1')
    let body
    try {
      body = inflateSync(raw).toString('latin1')
    } catch {
      body = m[1]
    }
    count += (body.match(/ Do\b/g) ?? []).length
  }
  return count
}

const original = twoPagePdf()

/** Highlights over page 1's line as other apps save them: F is the flags (4 is Print, 2 Hidden, 32 NoView). */
function savedHighlights() {
  const drawing = '0 0 1 RG 1 1 1 rg 0 0 300 30 re f'
  const highlight = (flags, ap) => `<< /Type /Annot /Subtype /Highlight /Rect [70 690 370 720] /QuadPoints [70 720 370 720 70 690 370 690] /C [1 1 0] /F ${flags}${ap ? ' /AP << /N 8 0 R >>' : ''} >>`
  return twoPagePdf({
    extra: [`<< /Type /XObject /Subtype /Form /BBox [70 690 370 720] /Length ${drawing.length} >>\nstream\n${drawing}\nendstream`],
    annots: [
      highlight(0, true), // no Print flag: shown, so it belongs in the copy
      highlight(0, false), // no Print flag and no drawing: the editor draws one
      highlight(4, false), // prints, but has no drawing to bake in
      highlight(2, true), // hidden
      highlight(32, true), // not shown on screen and not printed
    ],
  })
}
const browser = await pw[kind].launch()

async function run({ highlight, answer, undoRedo = false, pdf = original }) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
  page.setDefaultTimeout(20000)
  const errors = []
  page.on('pageerror', (e) => errors.push(e.message))
  await page.addInitScript((b64) => {
    localStorage.setItem('workspace.setupDone', '1')
    localStorage.setItem('workspace.settings', JSON.stringify({ version: 2, autosave: { enabled: false, delay: 1, kinds: {} }, history: false }))
    const bin = atob(b64)
    const files = new Map([['/docs/two.pdf', { bytes: Uint8Array.from(bin, (c) => c.charCodeAt(0)), mtime: 1000 }]])
    let cb = 0
    let opened = false
    window.__fake = { files, writes: [], saveDialogs: 0, saveAnswer: null }
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
      transformCallback: (fn) => { const id = ++cb; window[`_${id}`] = fn; return id },
      unregisterCallback: () => {},
      convertFileSrc: (p) => p,
      invoke: async (cmd, args, opts) => {
        switch (cmd) {
          case 'path_info': {
            const f = files.get(args.path)
            return { exists: !!f, isDir: false, size: f?.bytes.length ?? 0, modifiedMs: f?.mtime ?? 0 }
          }
          case 'read_file': {
            const f = files.get(args.path)
            if (!f) throw `missing ${args.path}`
            return f.bytes.slice().buffer
          }
          case 'write_file': {
            const path = decodeURIComponent(opts.headers['x-path'])
            files.set(path, { bytes: new Uint8Array(args), mtime: (files.get(path)?.mtime ?? 0) + 1000 })
            window.__fake.writes.push(path)
            return null
          }
          case 'take_pending_opens': {
            if (opened) return []
            opened = true
            return ['/docs/two.pdf']
          }
          case 'plugin:dialog|save':
            window.__fake.saveDialogs++
            return window.__fake.saveAnswer
          case 'plugin:event|listen':
            return ++cb
          case 'installed_plugins': case 'history_list': case 'list_dir': case 'walk_dir':
            return []
          default:
            return null
        }
      },
    }
  }, pdf.toString('base64'))
  await page.goto(url)
  await page.waitForFunction(() => {
    const s = window.__workspace?.useWorkspace.getState()
    if (!s || s.tabs[0]?.kind !== 'pdf') return false
    if (!s.rightOpen.pdf) s.toggleRight('pdf')
    return !!window.__workspace.getEditor(s.activeId)
  })
  await page.getByRole('button', { name: 'Highlight', exact: true }).first().click()
  for (const index of highlight) {
    await page.evaluate((i) => document.querySelectorAll('.text-layer')[i]?.scrollIntoView(), index)
    await page.waitForFunction((i) => document.querySelectorAll('.text-layer')[i]?.querySelector('span[data-run]'), index)
    const before = await page.evaluate(() => window.__workspace.useWorkspace.getState().tabs[0].dirty)
    await page.evaluate((i) => {
      const range = document.createRange()
      range.selectNodeContents(document.querySelectorAll('.text-layer')[i].querySelector('span[data-run]'))
      getSelection().removeAllRanges()
      getSelection().addRange(range)
      document.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }))
    }, index)
    if (!before) await page.waitForFunction(() => window.__workspace.useWorkspace.getState().tabs[0].dirty)
    await page.waitForFunction(() => !getSelection().toString())
  }
  if (undoRedo) {
    await page.keyboard.press('ControlOrMeta+z')
    await page.waitForTimeout(500)
    await page.keyboard.press('ControlOrMeta+Shift+z')
    await page.waitForTimeout(500)
  }
  await page.evaluate((a) => (window.__fake.saveAnswer = a), answer)
  await page.getByRole('button', { name: /Save flattened copy/ }).first().click()
  await page.waitForFunction(() => window.__fake.saveDialogs > 0 || document.querySelector('.dialog'))
  await page.waitForTimeout(500)
  const result = await page.evaluate(() => ({
    dialog: document.querySelector('.dialog')?.textContent ?? null,
    writes: window.__fake.writes,
    files: Object.fromEntries([...window.__fake.files].map(([p, f]) => [p, btoa(String.fromCharCode(...f.bytes))])),
  }))
  await page.close()
  assert.deepEqual(errors, [])
  return { ...result, file: (p) => (result.files[p] ? Buffer.from(result.files[p], 'base64') : null) }
}

try {
  {
    const r = await run({ highlight: [1], answer: '/docs/flat.pdf' })
    assert.equal(r.dialog, null, `flattening with an untouched first page failed: ${r.dialog}`)
    const flat = r.file('/docs/flat.pdf')
    assert.ok(flat, 'flattened copy was not written')
    assert.doesNotMatch(flat.toString('latin1'), /\/Annots/, 'flattened copy still has annotations')
    assert.equal(drawnXObjects(flat), drawnXObjects(original) + 2, 'page 2 highlight is missing from the flattened copy')
    assert.deepEqual(r.writes, ['/docs/flat.pdf'])
  }
  {
    const r = await run({ highlight: [0, 1], answer: '/docs/flat.pdf', undoRedo: true })
    assert.equal(r.dialog, null, `flattening failed: ${r.dialog}`)
    const flat = r.file('/docs/flat.pdf')
    assert.ok(flat, 'flattened copy was not written')
    assert.equal(drawnXObjects(flat), drawnXObjects(original) + 4, 'highlights, one undone and redone, are missing from the flattened copy')
  }
  {
    const r = await run({ highlight: [], answer: '/docs/flat.pdf', pdf: savedHighlights() })
    assert.equal(r.dialog, null, `flattening failed: ${r.dialog}`)
    const flat = r.file('/docs/flat.pdf')
    assert.ok(flat, 'flattened copy was not written')
    assert.doesNotMatch(flat.toString('latin1'), /\/Annots/, 'flattened copy still has annotations')
    assert.equal(drawnXObjects(flat), drawnXObjects(original) + 1 + 3, 'shown highlights without the Print flag or a drawing are missing from the flattened copy, or hidden ones are in it')
  }
  {
    const r = await run({ highlight: [0, 1], answer: null })
    assert.equal(r.dialog, null, `flattening failed: ${r.dialog}`)
    assert.deepEqual(r.writes, [], 'cancelled export wrote a file')
    assert.ok(r.file('/docs/two.pdf').equals(original), 'cancelled export changed the original')
  }
  console.log(`pdf-flatten: ${kind} passed untouched page, highlights, saved highlights, and cancel`)
} finally {
  await browser.close()
}
