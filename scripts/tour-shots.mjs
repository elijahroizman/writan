// The onboarding's pictures (src/components/tour/*.webp): screenshots of the app itself, in light
// and dark, opened on the sample documents in scripts/tour.
//
// Needs a test build being served, Playwright (`npm i --no-save playwright`, then
// `npx playwright install chromium`), and for `collab` and `agents` the standalone relay with
// MCP on, which the browser version shares through. `extras` needs the LaTeX plugin built
// (`npm run plugins`) and pdflatex on this computer:
//   VITE_TEST_HOOKS=1 npx vite build && npx vite preview --port 1420
//   cd src-tauri && cargo run --example collab_relay -- --port 47812 --mcp tour-token --bridge tour-key
//   node scripts/tour-shots.mjs http://localhost:1420/ [names…]
import { execFile } from 'node:child_process'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { promisify } from 'node:util'
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

/** Just the document: the side panels hidden, no arrival notes, and no autosave note
 * (a browser can't write the file). */
async function documentOnly(page) {
  await page.addStyleTag({ content: '.arrivals, .autosave-chip { display: none !important }' })
  await page.keyboard.press('Alt+Control+\\')
  await page.waitForTimeout(300)
  await page.keyboard.press('Control+\\')
  await page.waitForTimeout(600)
}

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
  },
  extras: {
    // A LaTeX document beside its PDF, compiled by this computer's pdflatex.
    viewport: { width: 900, height: 760 },
    clip: { x: 0, y: 0, width: 900, height: 700 },
    init: () => {
      localStorage.setItem('workspace.plugins', JSON.stringify({ latex: true }))
      window.__texFake = {
        status: async () => ({ installed: true, distribution: 'TeX Live', engines: ['pdflatex'] }),
        compile: async (source) => {
          const { pdf, log } = await window.__compileTex(source)
          return { pdf: pdf ? Uint8Array.from(atob(pdf), (c) => c.charCodeAt(0)) : null, log, file: 'Orbits.tex' }
        },
      }
    },
    async prepare(page) {
      await page.route('**/plugins/latex/**', async (route) => {
        const file = new URL(route.request().url()).pathname.split('/plugins/latex/')[1]
        await route.fulfill({ path: new URL(`../plugins/latex/dist/${file}`, import.meta.url).pathname })
      })
      await page.exposeFunction('__compileTex', async (source) => {
        const dir = await mkdtemp(join(tmpdir(), 'tour-tex-'))
        try {
          await writeFile(join(dir, 'doc.tex'), source)
          await promisify(execFile)('pdflatex', ['-interaction=nonstopmode', 'doc.tex'], { cwd: dir }).catch(() => {})
          const pdf = await readFile(join(dir, 'doc.pdf')).catch(() => null)
          const log = await readFile(join(dir, 'doc.log'), 'utf8').catch(() => '')
          return { pdf: pdf?.toString('base64') ?? null, log }
        } finally {
          await rm(dir, { recursive: true, force: true })
        }
      })
    },
    async setup(page) {
      await open(page, 'Orbits.tex')
      await page.keyboard.press('Control+\\')
      await page.getByRole('radio', { name: 'Source and PDF' }).click()
      await page.waitForTimeout(4000)
      await page.evaluate(() => document.activeElement?.blur())
    },
  },
  collab: {
    // Maya shares the library update and two people join: their carets, and Maya's share panel
    // with its code.
    viewport: { width: 900, height: 800 },
    clip: { x: 0, y: 0, width: 900, height: 740 },
    async setup(page, { newPage }) {
      await open(page, 'Library Update.docx')
      await documentOnly(page)
      await page.getByRole('button', { name: 'Share on your network' }).click()
      const panel = page.getByRole('dialog', { name: 'Share on your network' })
      await panel.getByLabel('Your name').fill('Maya')
      await panel.getByRole('button', { name: /^(Save and )?Share$/ }).click()
      const invite = await page.getByLabel('Invite link').inputValue()
      await page.keyboard.press('Escape')

      const join = async (name, after, text) => {
        const guest = await newPage()
        await guest.getByRole('tab', { name: 'Home' }).click()
        await guest.getByRole('button', { name: 'Join a shared document…' }).click()
        const dialog = guest.getByRole('dialog', { name: 'Join a shared document' })
        await dialog.getByLabel('Code or link').fill(invite)
        await dialog.getByLabel('Your name').fill(name)
        await dialog.getByRole('button', { name: 'Join' }).click()
        await dialog.waitFor({ state: 'detached' })
        await guest.waitForTimeout(2500)
        await documentOnly(guest)
        await guest.locator('.docx-pages').getByText(after).first().click()
        await guest.keyboard.press('End')
        await guest.keyboard.type(text, { delay: 20 })
        return guest
      }
      const guests = [await join('Sam', 'A step-free entrance', ' and a ramp'), await join('Lena', 'Twelve bookable study booths', ', two of them quiet')]
      await page.waitForTimeout(1500)

      // The app shows a six-digit code for sharing over the internet; a browser shares on its
      // network with a link, so the code is put in as the app would show it.
      await page.evaluate(() => {
        const id = window.__workspace.useWorkspace.getState().activeId
        const collab = window.__collab.useCollab
        collab.setState({ [id]: { ...collab.getState()[id], code: '482913', expiresAt: Date.now() + 9 * 60_000 + 41_000, internet: true } })
      })
      await page.getByRole('button', { name: /Shared/ }).click()
      // Both type once more, so both name tags show.
      for (const guest of guests) await guest.keyboard.type('.')
      await page.waitForTimeout(600)
      await page.evaluate(() => document.activeElement?.blur())
      return () => Promise.all(guests.map((g) => g.close()))
    },
  },
  agents: {
    // Claude writes in the document, and Codex says what it's doing in the agents' chat.
    viewport: { width: 900, height: 800 },
    clip: { x: 0, y: 0, width: 900, height: 740 },
    init: () => {
      localStorage.setItem('workspace.agentsBridgeKey', 'tour-key')
      localStorage.setItem('workspace.ai', JSON.stringify({ agents: true, token: 'tour-token', messages: true }))
    },
    async setup(page) {
      await open(page, 'Library Update.docx')
      await documentOnly(page)
      await page.evaluate(() => document.activeElement?.blur())
      const claude = await mcpClient('claude')
      const codex = await mcpClient('codex')
      const doc = /document: "([^"]+)"/.exec(await claude('list_documents'))?.[1]
      if (!doc) throw new Error('The agent found no document')
      await claude('read_document', { document: doc })
      await codex('read_document', { document: doc })
      await claude('insert_paragraphs', { document: doc, after: 2, text: 'Evening hours start on Monday.' })
      await codex('send_message', { document: doc, text: 'I’ll check the budget table against the invoices.' })
      await claude('send_message', { document: doc, text: 'Done with the opening hours. The summary is next.' })
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
  await shot.prepare?.(page)
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
      const cleanup = await shot.setup(page, { newPage: () => newPage({ ...shot, init: undefined, prepare: undefined }, theme) })
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
