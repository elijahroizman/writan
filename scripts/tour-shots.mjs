// The onboarding's pictures (src/components/tour/*.webp): screenshots of the app itself, in light
// and dark, opened on the sample documents in scripts/tour.
//
// Needs a test build being served, Playwright (`npm i --no-save playwright`, then
// `npx playwright install chromium`), and for `collab` and `agents` the standalone relay with
// MCP on, which the browser version shares through:
//   VITE_TEST_HOOKS=1 npx vite build && npx vite preview --port 1420
//   cd src-tauri && cargo run --example collab_relay -- --port 47812 --mcp tour-token --bridge tour-key
//   node scripts/tour-shots.mjs http://localhost:1420/ [names…]
import { readFile, writeFile } from 'node:fs/promises'
import { chromium } from 'playwright'

const url = process.argv[2] ?? 'http://localhost:1420/'
const samples = new URL('./tour/', import.meta.url)
const out = new URL('../src/components/tour/', import.meta.url)
const SCALE = 2
const QUALITY = 0.86
const MCP = 'http://127.0.0.1:47812/mcp/tour-token'

const open = async (page, ...names) => {
  for (const name of names) {
    await page.evaluate((p) => window.__workspace.useWorkspace.getState().openPaths([p]), `/tour-samples/${name}`)
    await page.waitForTimeout(2500)
  }
}

/** Clicks into the Word page at `y` pixels below its top, `x` from its left. */
const clickPage = (page, x, y) => page.locator('.docx-content-mount').first().click({ position: { x, y } })

/** An MCP client speaking for `agent`, as the agent would once Settings › AI set it up. */
async function mcpClient(agent) {
  let session = null
  let id = 0
  const post = async (body) => {
    const res = await fetch(`${MCP}?agent=${agent}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json, text/event-stream', ...(session ? { 'Mcp-Session-Id': session } : {}) },
      body: JSON.stringify(body),
    })
    if (res.headers.get('mcp-session-id')) session = res.headers.get('mcp-session-id')
    return res.status === 202 ? null : res.json()
  }
  await post({ jsonrpc: '2.0', id: ++id, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: `${agent}-tour`, version: '1' } } })
  await post({ jsonrpc: '2.0', method: 'notifications/initialized' })
  return async (name, args = {}) => {
    const r = await post({ jsonrpc: '2.0', id: ++id, method: 'tools/call', params: { name, arguments: args } })
    if (r.error || r.result.isError) throw new Error(`${name}: ${JSON.stringify(r.error ?? r.result.content)}`)
    return r.result.content.map((c) => c.text).join('\n')
  }
}

const SHOTS = {
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
  collab: {
    // Maya shares the lease update and Sam joins: Sam's caret, Maya's share panel with its code.
    viewport: { width: 1180, height: 760 },
    clip: { x: 380, y: 0, width: 800, height: 700 },
    async setup(page, { newPage }) {
      await open(page, 'Library Update.docx')
      await page.getByRole('button', { name: 'Share on your network' }).click()
      const panel = page.getByRole('dialog', { name: 'Share on your network' })
      await panel.getByLabel('Your name').fill('Maya')
      await panel.getByRole('button', { name: /^(Save and )?Share$/ }).click()
      const invite = await page.getByLabel('Invite link').inputValue()
      await page.keyboard.press('Escape')

      const guest = await newPage()
      await guest.getByRole('tab', { name: 'Home' }).click()
      await guest.getByRole('button', { name: 'Join a shared document…' }).click()
      const dialog = guest.getByRole('dialog', { name: 'Join a shared document' })
      await dialog.getByLabel('Code or link').fill(invite)
      await dialog.getByLabel('Your name').fill('Sam')
      await dialog.getByRole('button', { name: 'Join' }).click()
      await dialog.waitFor({ state: 'detached' })
      await guest.waitForTimeout(2500)
      await clickPage(guest, 420, 300)
      await guest.keyboard.press('End')
      await guest.keyboard.type(' Sam checked these numbers.', { delay: 20 })
      await clickPage(page, 300, 180)
      await page.keyboard.press('End')
      await page.waitForTimeout(1200)

      // The app shows a six-digit code for sharing over the internet; a browser shares on its
      // network with a link, so the code is put in as the app would show it.
      await page.evaluate(() => {
        const id = window.__workspace.useWorkspace.getState().activeId
        const collab = window.__collab.useCollab
        collab.setState({ [id]: { ...collab.getState()[id], code: '482913', expiresAt: Date.now() + 9 * 60_000 + 41_000, internet: true } })
      })
      await page.getByRole('button', { name: /Shared/ }).click()
      await page.waitForTimeout(600)
      await page.evaluate(() => document.activeElement?.blur())
      return () => guest.close()
    },
  },
  agents: {
    // Claude writes in the document, and Codex says what it's doing in the agents' chat.
    viewport: { width: 1180, height: 760 },
    clip: { x: 380, y: 0, width: 800, height: 700 },
    init: () => {
      localStorage.setItem('workspace.agentsBridgeKey', 'tour-key')
      localStorage.setItem('workspace.ai', JSON.stringify({ agents: true, token: 'tour-token', messages: true }))
    },
    async setup(page) {
      await open(page, 'Library Update.docx')
      await page.evaluate(() => document.activeElement?.blur())
      const claude = await mcpClient('claude')
      const codex = await mcpClient('codex')
      const doc = /document: "([^"]+)"/.exec(await claude('list_documents'))?.[1]
      if (!doc) throw new Error('The agent found no document')
      await claude('read_document', { document: doc })
      await codex('read_document', { document: doc })
      await claude('insert_paragraphs', { document: doc, after: 2, text: 'The reading room reopens on Monday with longer evening hours.' })
      await codex('send_message', { document: doc, text: 'I’ll check the budget table while Claude writes the summary.' })
      await claude('send_message', { document: doc, text: 'Thanks. The summary is in, under the opening paragraph.' })
      await page.waitForTimeout(1200)
      await page.getByRole('button', { name: /Agents’ chat/ }).click()
      await page.waitForTimeout(600)
    },
  },
}

const browser = await chromium.launch({ args: ['--no-sandbox'] })
const only = process.argv.slice(3)

/** A window on the app in `theme`, with the samples served and nothing saved from before. */
async function newPage(shot, theme) {
  const page = await browser.newPage({ viewport: shot.viewport, deviceScaleFactor: SCALE })
  await page.route('**/tour-samples/*', async (route) => {
    const file = decodeURIComponent(new URL(route.request().url()).pathname.split('/').pop())
    await route.fulfill({ body: await readFile(new URL(file, samples)) })
  })
  await page.addInitScript((theme) => {
    if (sessionStorage.getItem('tour-shot')) return
    sessionStorage.setItem('tour-shot', '1')
    localStorage.clear()
    localStorage.setItem('workspace.setupDone', '2')
    localStorage.setItem('workspace.themeMode', JSON.stringify(theme))
  }, theme)
  if (shot.init) await page.addInitScript(shot.init)
  await page.goto(url)
  await page.addStyleTag({ content: '.update-btn { visibility: hidden !important } * { caret-color: transparent !important }' })
  await page.waitForTimeout(800)
  return page
}

try {
  for (const [name, shot] of Object.entries(SHOTS)) {
    if (only.length && !only.includes(name)) continue
    for (const theme of ['light', 'dark']) {
      const page = await newPage(shot, theme)
      const cleanup = await shot.setup(page, { newPage: () => newPage({ ...shot, init: undefined }, theme) })
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
      const file = new URL(`${name}-${theme}.webp`, out)
      await writeFile(file, Buffer.from(webp, 'base64'))
      console.log(file.pathname)
      await cleanup?.()
      await page.close()
    }
  }
} finally {
  await browser.close()
}
