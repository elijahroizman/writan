// Page selection follows the page you go to: after rotating page 2 and moving it up, the moved
// page is current, clicking a thumbnail makes its page current, and "Extract page as PDF…" writes
// the page you clicked. Runs against a VITE_TEST_HOOKS build with a faked Tauri backend.
//   node tests/stress/native/pdf-page-select.mjs http://localhost:4173/ [chromium|webkit]
import assert from 'node:assert/strict'
import { inflateSync } from 'node:zlib'
import * as pw from 'playwright'

const url = process.argv[2] ?? 'http://localhost:4173/'
const kind = process.argv[3] ?? 'chromium'

function threePagePdf() {
  const labels = ['Page one', 'Page two', 'Page three']
  const font = 3 + labels.length * 2
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    `<< /Type /Pages /Kids [${labels.map((_, i) => `${3 + i * 2} 0 R`).join(' ')}] /Count ${labels.length} >>`,
  ]
  for (const [i, label] of labels.entries()) {
    const s = `BT /F1 24 Tf 72 700 Td (${label}) Tj ET`
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 ${font} 0 R >> >> /Contents ${4 + i * 2} 0 R >>`)
    objects.push(`<< /Length ${s.length} >>\nstream\n${s}\nendstream`)
  }
  objects.push('<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>')
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

/** The text of a PDF's streams, inflated where they're compressed. */
function streams(pdf) {
  return [...pdf.matchAll(/stream\r?\n([\s\S]*?)endstream/g)]
    .map((m) => {
      try {
        return inflateSync(Buffer.from(m[1], 'latin1')).toString('latin1')
      } catch {
        return m[1]
      }
    })
    .join('\n')
}

const browser = await pw[kind].launch()
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
  page.setDefaultTimeout(20000)
  const errors = []
  page.on('pageerror', (e) => errors.push(e.message))
  await page.addInitScript((b64) => {
    localStorage.setItem('workspace.setupDone', '1')
    localStorage.setItem('workspace.settings', JSON.stringify({ version: 2, autosave: { enabled: false, delay: 1, kinds: {} }, history: false }))
    const bin = atob(b64)
    const files = new Map([['/docs/three.pdf', { bytes: Uint8Array.from(bin, (c) => c.charCodeAt(0)), mtime: 1000 }]])
    let cb = 0
    let opened = false
    window.__fake = { files, writes: [], saveAnswer: null }
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
            return ['/docs/three.pdf']
          }
          case 'plugin:dialog|save':
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
  }, threePagePdf().toString('base64'))
  await page.goto(url)
  await page.waitForFunction(() => {
    const s = window.__workspace?.useWorkspace.getState()
    if (!s || s.tabs[0]?.kind !== 'pdf') return false
    if (!s.leftOpen.pdf) s.toggleLeft('pdf')
    if (!s.rightOpen.pdf) s.toggleRight('pdf')
    return !!window.__workspace.getEditor(s.activeId)
  })

  const thumb = (n) => page.locator(`.thumb[data-page="${n}"]`)
  const expectCurrent = async (n, step, total = 3) => {
    try {
      await page.getByText(`Page ${n} of ${total}`, { exact: true }).waitFor({ timeout: 5000 })
    } catch {
      const shown = await page.evaluate(() => [...document.querySelectorAll('*')].find((e) => /^Page \d+ of \d+$/.test(e.textContent ?? '') && !e.children.length)?.textContent)
      assert.fail(`${step}: expected page ${n} to be current, the inspector shows "${shown}"`)
    }
    assert.match((await thumb(n).getAttribute('class')) ?? '', /\bactive\b/, `${step}: thumbnail ${n} isn't highlighted`)
  }

  await thumb(2).waitFor()
  await thumb(2).click()
  await expectCurrent(2, 'clicking thumbnail 2')
  await page.getByRole('button', { name: 'Rotate right' }).click()
  await page.waitForFunction(() => window.__workspace.useWorkspace.getState().tabs[0].dirty)
  await page.getByRole('button', { name: 'Move up' }).click()
  await expectCurrent(1, 'moving the rotated page up')
  await thumb(2).click()
  await expectCurrent(2, 'clicking thumbnail 2 after the move')
  await thumb(1).click()
  await expectCurrent(1, 'clicking thumbnail 1 after the move')

  await page.evaluate(() => (window.__fake.saveAnswer = '/docs/extracted.pdf'))
  await page.getByRole('button', { name: /Extract page as PDF/ }).click()
  await page.waitForFunction(() => window.__fake.writes.includes('/docs/extracted.pdf'))
  const extracted = Buffer.from(await page.evaluate(() => btoa(String.fromCharCode(...window.__fake.files.get('/docs/extracted.pdf').bytes))), 'base64').toString('latin1')
  assert.match(extracted, /\/Rotate\s+90\b/, 'the extracted page is not the rotated page that was clicked')
  assert.match(streams(extracted), /\(Page two\)/, 'the extracted page is not "Page two"')

  const pageNumber = page.getByRole('textbox', { name: 'Page number' })
  await pageNumber.fill('1.5')
  await pageNumber.press('Enter')
  await expectCurrent(1, 'entering a fractional page number')
  assert.equal(await pageNumber.inputValue(), '1', 'the page number field kept the rejected value')

  await thumb(3).click()
  await expectCurrent(3, 'clicking thumbnail 3')
  await thumb(3).click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Delete page' }).click()
  await expectCurrent(2, 'deleting the last page', 2)

  assert.deepEqual(errors, [])
  console.log(`pdf-page-select: ${kind} passed move, thumbnail clicks, extract, page number, and delete`)
} finally {
  await browser.close()
}
