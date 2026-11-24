// A phone photo pasted into a shared Word document reaches the other copy small and quickly: it's
// scaled down to what the page can show before it's sent, not sent whole.
//
// Needs the standalone relay running on the default port, which the browser version shares
// through:
//   cd src-tauri && cargo run --example collab_relay -- --port 47812
import { launch, start, check, done } from './harness.mjs'

const a = await launch({ tauri: false })
const b = await launch({ tauri: false })
const logs = []
a.page.on('pageerror', (e) => logs.push(`[pageerror a] ${e.message}`))
b.page.on('pageerror', (e) => logs.push(`[pageerror b] ${e.message}`))

// Counts what each copy sends the relay.
const countSends = () => {
  const send = WebSocket.prototype.send
  window.__sent = []
  WebSocket.prototype.send = function (data) {
    window.__sent.push(typeof data === 'string' ? data.length : 0)
    return send.call(this, data)
  }
}
await a.page.addInitScript(countSends)

try {
  await start(a.page)
  await start(b.page)

  await a.page.getByRole('button', { name: /New document/ }).first().click()
  await a.page.waitForFunction(() => window.__docxEditor)
  await a.page.locator('.docx-content-mount').click({ position: { x: 200, y: 120 } })
  await a.page.keyboard.type('A photo:')
  await a.page.getByRole('button', { name: 'Share on your network' }).click()
  const panel = a.page.getByRole('dialog', { name: 'Share on your network' })
  await panel.getByLabel('Your name').fill('Ada')
  await panel.getByRole('button', { name: /^(Save and )?Share$/ }).click()
  const invite = await a.page.getByLabel('Invite link').inputValue()
  await a.page.keyboard.press('Escape')

  await b.page.getByRole('tab', { name: 'Home' }).click()
  await b.page.getByRole('button', { name: 'Join a shared document…' }).click()
  const dialog = b.page.getByRole('dialog', { name: 'Join a shared document' })
  await dialog.getByLabel('Code or link').fill(invite)
  await dialog.getByLabel('Your name').fill('Bo')
  await dialog.getByRole('button', { name: 'Join' }).click()
  await b.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('A photo:'))

  // A 12-megapixel photo, as a phone takes it: noise, so it doesn't compress to nothing.
  await a.page.locator('.docx-content-mount').click({ position: { x: 400, y: 120 } })
  await a.page.keyboard.press('End')
  const original = await a.page.evaluate(async () => {
    const c = document.createElement('canvas')
    c.width = 4032
    c.height = 3024
    const ctx = c.getContext('2d')
    const img = ctx.createImageData(c.width, c.height)
    let x = 1
    for (let i = 0; i < img.data.length; i += 4) {
      x = (x * 1103515245 + 12345) & 0x7fffffff
      const v = (i / 4) % c.width
      img.data[i] = (v / 16 + (x & 63)) & 255
      img.data[i + 1] = ((i / 4 / c.width) / 12 + ((x >> 6) & 63)) & 255
      img.data[i + 2] = (x >> 12) & 255
      img.data[i + 3] = 255
    }
    ctx.putImageData(img, 0, 0)
    const blob = await new Promise((r) => c.toBlob(r, 'image/jpeg', 0.92))
    const file = new File([blob], 'photo.jpg', { type: 'image/jpeg' })
    window.__sent = []
    const dt = new DataTransfer()
    dt.items.add(file)
    window.__pastedAt = Date.now()
    document.querySelector('[data-engine-input]').dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }))
    return { bytes: file.size, at: window.__pastedAt }
  })
  await b.page.waitForFunction(() => document.querySelector('.docx-pages img'), null, { timeout: 60000 })
  const arrived = Date.now()
  const got = await b.page.evaluate(async () => {
    const img = document.querySelector('.docx-pages img')
    const blob = await fetch(img.src).then((r) => r.blob())
    await img.decode()
    return { bytes: blob.size, width: img.naturalWidth, height: img.naturalHeight }
  })
  const sent = await a.page.evaluate(() => Math.max(0, ...window.__sent))
  console.log(`photo: ${(original.bytes / 1e6).toFixed(2)} MB; Bo got ${(got.bytes / 1e6).toFixed(2)} MB, ${got.width}×${got.height}; biggest message ${(sent / 1e6).toFixed(2)} MB; on Bo's screen ${arrived - original.at} ms after the paste`)
  check('the photo reaches the other copy', got.width > 0)
  check('it is scaled down to twice the text width before it goes', got.width <= 1300 && got.width >= 1200, `${got.width}×${got.height}`)
  check('what goes to the relay is a fraction of the photo', sent < original.bytes / 3, `${sent} of ${original.bytes}`)
  const hostImg = await a.page.evaluate(() => document.querySelector('.docx-pages img')?.naturalWidth)
  check('both copies show the same picture', hostImg === got.width, `${hostImg} and ${got.width}`)
} finally {
  await a.browser.close()
  await done(b.browser, logs)
}
