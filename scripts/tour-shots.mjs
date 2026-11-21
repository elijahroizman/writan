// The welcome tour's pictures (src/components/tour/*.webp): screenshots of the app itself, in light
// and dark, opened on the sample documents in scripts/tour. The palette's is taken twice, with Mac
// and with Windows/Linux shortcuts.
//
// Needs a test build being served and Playwright (`npm i --no-save playwright`, then
// `npx playwright install chromium`):
//   VITE_TEST_HOOKS=1 npx vite build && npx vite preview --port 1420
//   node scripts/tour-shots.mjs http://localhost:1420/
import { readFile, writeFile } from 'node:fs/promises'
import { chromium } from 'playwright'

const url = process.argv[2] ?? 'http://localhost:1420/'
const samples = new URL('./tour/', import.meta.url)
const out = new URL('../src/components/tour/', import.meta.url)
const SCALE = 2
const QUALITY = 0.86

const open = async (page, ...names) => {
  for (const name of names) {
    await page.evaluate((p) => window.__workspace.useWorkspace.getState().openPaths([p]), `/tour-samples/${name}`)
    await page.waitForTimeout(2500)
  }
}

/** Each picture: the window size, what to do, and the part of the window to keep. */
const SHOTS = {
  palette: {
    platforms: ['mac', 'pc'],
    viewport: { width: 1100, height: 720 },
    clip: { x: 210, y: 0, width: 680, height: 540 },
    async setup(page, platform) {
      await open(page, 'Lease.pdf', 'Library Update.docx')
      await page.keyboard.press(platform === 'mac' ? 'Meta+k' : 'Control+k')
      await page.waitForTimeout(400)
      await page.keyboard.type('spl', { delay: 60 })
      await page.waitForTimeout(400)
    },
  },
  word: {
    viewport: { width: 1100, height: 720 },
    clip: { x: 0, y: 0, width: 790, height: 640 },
    async setup(page) {
      await open(page, 'Library Update.docx')
      await page.evaluate(() => document.activeElement?.blur())
      await page.waitForTimeout(300)
    },
  },
  slides: {
    viewport: { width: 1080, height: 720 },
    clip: { x: 0, y: 0, width: 790, height: 640 },
    async setup(page) {
      await open(page, 'Spring Launch.pptx')
      await page.locator('.slide-item').nth(1).click()
      await page.waitForTimeout(800)
    },
  },
  pdf: {
    viewport: { width: 1100, height: 720 },
    clip: { x: 310, y: 44, width: 790, height: 620 },
    async setup(page) {
      await open(page, 'Lease.pdf')
    },
  },
  extras: {
    viewport: { width: 1100, height: 720 },
    clip: { x: 170, y: 60, width: 760, height: 600 },
    async setup(page) {
      await page.keyboard.press('Control+,')
      await page.waitForTimeout(500)
      // As in the app: the LaTeX extension under General, turned on.
      await page.evaluate(() => {
        const row = document.querySelector('.settings-scroll .plugin-row')
        if (!row) throw new Error('No plugin row')
        row.scrollIntoView({ block: 'start' })
        const sw = row.querySelector('.switch')
        sw.classList.add('on')
        sw.setAttribute('aria-checked', 'true')
      })
      await page.waitForTimeout(300)
    },
  },
  split: {
    viewport: { width: 1000, height: 660 },
    clip: { x: 0, y: 0, width: 1000, height: 660 },
    async setup(page) {
      await open(page, 'Lease.pdf', 'Library Update.docx')
      await page.evaluate(async () => {
        const s = window.__workspace.useWorkspace.getState()
        await s.splitWith(s.tabs.find((t) => t.title === 'Lease').id)
      })
      await page.waitForTimeout(2500)
    },
  },
}

const browser = await chromium.launch({ args: ['--no-sandbox'] })
const only = process.argv.slice(3)
try {
  for (const [name, shot] of Object.entries(SHOTS)) {
    if (only.length && !only.includes(name)) continue
    for (const theme of ['light', 'dark']) {
      for (const platform of shot.platforms ?? [null]) {
        const page = await browser.newPage({ viewport: shot.viewport, deviceScaleFactor: SCALE })
        await page.route('**/tour-samples/*', async (route) => {
          const file = decodeURIComponent(new URL(route.request().url()).pathname.split('/').pop())
          await route.fulfill({ body: await readFile(new URL(file, samples)) })
        })
        await page.addInitScript(
          ([theme, platform]) => {
            if (platform === 'mac') Object.defineProperty(Navigator.prototype, 'platform', { get: () => 'MacIntel' })
            if (sessionStorage.getItem('tour-shot')) return
            sessionStorage.setItem('tour-shot', '1')
            localStorage.clear()
            localStorage.setItem('workspace.setupDone', '1')
            localStorage.setItem('workspace.themeMode', JSON.stringify(theme))
          },
          [theme, platform],
        )
        await page.goto(url)
        await page.addStyleTag({ content: '.update-btn { visibility: hidden !important } * { caret-color: transparent !important }' })
        await page.waitForTimeout(800)
        await shot.setup(page, platform)
        const png = await page.screenshot({ clip: shot.clip })
        const webp = await page.evaluate(
          async ([b64, quality]) => {
            const img = new Image()
            img.src = `data:image/png;base64,${b64}`
            await img.decode()
            const canvas = document.createElement('canvas')
            canvas.width = img.naturalWidth
            canvas.height = img.naturalHeight
            canvas.getContext('2d').drawImage(img, 0, 0)
            return canvas.toDataURL('image/webp', quality).split(',')[1]
          },
          [png.toString('base64'), QUALITY],
        )
        const file = new URL(`${[name, theme, platform].filter(Boolean).join('-')}.webp`, out)
        await writeFile(file, Buffer.from(webp, 'base64'))
        console.log(file.pathname)
        await page.close()
      }
    }
  }
} finally {
  await browser.close()
}
