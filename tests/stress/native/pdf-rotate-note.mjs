// A text note added on a rotated page: the saved FreeText annotation must paint its text
// along the box the user saw, not wrapped into slivers and clipped. The appearance stream
// must draw the note in one run (rotated into place via its /Matrix), not one Tj per wrap.
// Runs against a VITE_TEST_HOOKS build.
//   node tests/stress/native/pdf-rotate-note.mjs http://localhost:4173/ [chromium|webkit]
import assert from 'node:assert/strict'
import * as pw from 'playwright'
import zlib from 'node:zlib'

const url = process.argv[2] ?? 'http://localhost:4173/'
const kind = process.argv[3] ?? 'chromium'

/** One landscape page with a line of text; fake Tauri keeps the file in memory. */
function onePagePdf() {
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>',
    '<< /Length 37 >>\nstream\nBT /F1 24 Tf 72 700 Td (Some words) Tj ET\nendstream',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
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

/** Decompressed appearance streams of every FreeText annotation in a saved file. */
function freeTextAppearances(pdf) {
  const text = pdf.toString('latin1')
  const aps = []
  for (const m of text.matchAll(/\/AP\s*<<\s*\/N\s+(\d+)\s+0\s+R[\s\S]{0,400}?\/Subtype\s*\/FreeText/g)) {
    const obj = new RegExp(`${m[1]} 0 obj([\\s\\S]{0,600}?)stream\\r?\\n([\\s\\S]*?)endstream`).exec(text)
    if (obj) aps.push({ dict: obj[1], content: zlib.inflateSync(Buffer.from(obj[2].trim(), 'latin1')).toString('latin1') })
  }
  return aps
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
    const files = new Map([['/docs/rot.pdf', { bytes: Uint8Array.from(bin, (c) => c.charCodeAt(0)), mtime: 1000 }]])
    let cb = 0
    let opened = false
    window.__fake = { files, writes: [] }
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
            return ['/docs/rot.pdf']
          }
          case 'plugin:dialog|save':
            return null
          case 'plugin:event|listen':
            return ++cb
          case 'installed_plugins': case 'history_list': case 'list_dir': case 'walk_dir':
            return []
          default:
            return null
        }
      },
    }
  }, onePagePdf().toString('base64'))
  await page.goto(url)
  await page.waitForFunction(() => {
    const s = window.__workspace?.useWorkspace.getState()
    return s && s.tabs[0]?.kind === 'pdf' && !!window.__workspace.getEditor(s.activeId)
  })

  // Rotate the page right, then add a text note with the Text tool.
  await page.getByRole('button', { name: 'Rotate right', exact: true }).click()
  await page.getByRole('button', { name: 'Text', exact: true }).click()
  const layer = page.locator('.tool-layer.text').first()
  await layer.waitFor()
  const box = await layer.boundingBox()
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2)
  const draft = page.locator('.text-draft')
  await draft.waitFor()
  await draft.click()
  await page.keyboard.insertText('Rot note')
  // Leaving the draft saves the note.
  await page.getByRole('button', { name: 'Select', exact: true }).click()
  await page.waitForFunction(() => !document.querySelector('.text-draft'))
  await page.waitForTimeout(500)

  const saved = await page.evaluate(async () => {
    const s = window.__workspace.useWorkspace.getState()
    await s.save(s.activeId, { to: '/saved/rot-note.pdf', quiet: true })
    const f = window.__fake.files.get('/saved/rot-note.pdf')
    return f ? btoa(String.fromCharCode(...f.bytes)) : null
  })
  assert.ok(saved, 'the note was not saved')
  const pdf = Buffer.from(saved, 'base64')
  const aps = freeTextAppearances(pdf)
  assert.equal(aps.length, 1, `expected one FreeText annot, found ${aps.length}`)
  console.log(`saved FreeText /AP: ${aps[0].content.replace(/\n/g, ' | ')}`)
  // The text must be painted as one run; the buggy version wrapped it into ~2-letter lines.
  assert.ok(aps[0].content.includes('(Rot note) Tj'), `note text is not drawn as a single run: ${aps[0].content}`)
  assert.deepEqual(errors, [])
  console.log(`pdf-rotate-note: ${kind} passed`)
} finally {
  await browser.close()
}
