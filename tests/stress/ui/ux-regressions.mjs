// Regression checks for Word viewport page counter, Windows default-app settings, and
// clickable "Save to enable autosave" status messaging.
// node tests/stress/ui/ux-regressions.mjs
import { launch, start, tabs, check, done } from './harness.mjs'

// ── 1 & 4: Word viewport page counter + untitled autosave chip
{
  const { browser, page, logs } = await launch()
  await start(page)

  // Create a new untitled Word document.
  await page.evaluate(() => window.__workspace.useWorkspace.getState().newDocument('docx'))
  await page.waitForSelector('.docx-viewport .docx-page')
  await page.waitForFunction(() => !!window.__docxEditor)

  // Priority 4: untitled doc displays clickable "Save to enable autosave" button.
  const chipBtn = page.locator('button.autosave-chip')
  await chipBtn.waitFor()
  const chipText = (await chipBtn.textContent())?.trim() ?? ''
  check('untitled document shows "Save to enable autosave"', chipText === 'Save to enable autosave', chipText)

  // Priority 1: build an 11-page Word document (10 page breaks) and verify scroll page counter.
  await page.evaluate(() => {
    const ed = window.__docxEditor
    ed.run({ cmd: 'insertText', text: 'Page 1 start' })
    for (let i = 2; i <= 11; i++) {
      ed.run({ cmd: 'pageBreak' })
      ed.run({ cmd: 'insertText', text: `Page ${i}` })
    }
    // Move caret back to page 1 so caret-based tracking stays on page 1 while we scroll.
    ed.run({ cmd: 'move', to: 'docStart', extend: false })
  })

  const readout = page.locator('.view[data-active=true] .canvas-bar span.readout').first()
  await page.waitForFunction(() => window.__docxEditor?.state.page.total === 11)
  // Scroll explicitly to top to start on page 1.
  await page.evaluate(() => {
    const vp = document.querySelector('.docx-viewport')
    vp.scrollTop = 0
  })
  await page.waitForFunction(() => document.querySelector('.view[data-active=true] .canvas-bar span.readout')?.textContent?.trim() === '1 / 11')
  check('initial page indicator reads 1 / 11', (await readout.textContent())?.trim() === '1 / 11', await readout.textContent())

  // Scroll to page 6 (index 5) and verify readout updates while caret stays on page 1.
  await page.evaluate(() => {
    const vp = document.querySelector('.docx-viewport')
    vp.scrollTop = (vp.scrollHeight * 5.5) / 11
  })
  await page.waitForFunction(() => document.querySelector('.view[data-active=true] .canvas-bar span.readout')?.textContent?.trim() === '6 / 11')
  const caretPageMid = await page.evaluate(() => window.__docxEditor.state.page.current)
  check('scrolling to page 6 updates indicator to 6 / 11', (await readout.textContent())?.trim() === '6 / 11', await readout.textContent())
  check('caret page tracking remains preserved at page 1 while scrolling', caretPageMid === 1, String(caretPageMid))

  // Scroll to page 11 (index 10) and verify readout updates to 11 / 11.
  await page.evaluate(() => {
    const vp = document.querySelector('.docx-viewport')
    vp.scrollTop = vp.scrollHeight
  })
  await page.waitForFunction(() => document.querySelector('.view[data-active=true] .canvas-bar span.readout')?.textContent?.trim() === '11 / 11')
  check('scrolling to page 11 updates indicator to 11 / 11', (await readout.textContent())?.trim() === '11 / 11', await readout.textContent())

  // Scroll back to top -> indicator returns to 1 / 11.
  await page.evaluate(() => {
    const vp = document.querySelector('.docx-viewport')
    vp.scrollTop = 0
  })
  await page.waitForFunction(() => document.querySelector('.view[data-active=true] .canvas-bar span.readout')?.textContent?.trim() === '1 / 11')
  check('scrolling back to top restores 1 / 11', (await readout.textContent())?.trim() === '1 / 11', await readout.textContent())

  // Priority 4: clicking "Save to enable autosave" initiates Save.
  await page.evaluate(() => {
    window.__fake.savePath = '/tmp/writan-ux-regression.docx'
  })
  await chipBtn.click()
  await page.waitForFunction(() => window.__fake.writes.some((w) => w.path === '/tmp/writan-ux-regression.docx'))
  const afterSaveTabs = await tabs(page)
  check('clicking autosave chip triggers Save and sets path', afterSaveTabs.some((t) => t.path === '/tmp/writan-ux-regression.docx' && !t.dirty), JSON.stringify(afterSaveTabs))
  check('save button disappears once file is saved', (await page.locator('button.autosave-chip').count()) === 0)

  await done(browser, logs)
}

// ── 2: Windows default-app settings (normal + partial unknown + detection failure)
{
  const { browser, page, logs } = await launch({
    init: () => {
      Object.defineProperty(navigator, 'platform', { value: 'Win32', configurable: true })
      const origInvoke = window.__TAURI_INTERNALS__.invoke
      window.__defaultAppsMock = {
        mode: 'normal',
        setCalls: [],
      }
      window.__TAURI_INTERNALS__.invoke = async (cmd, args, opts) => {
        if (cmd === 'default_apps') {
          const mode = window.__defaultAppsMock.mode
          if (mode === 'error') throw new Error('Registry access denied')
          if (mode === 'unknown') return args.exts.map((ext) => ({ ext, isDefault: null }))
          // Simulate Windows where xlsm and tsv are unregistered secondary extensions (null)
          // and docx is already default.
          return args.exts.map((ext) => ({
            ext,
            isDefault: ext === 'xlsm' || ext === 'tsv' ? null : ext === 'docx',
          }))
        }
        if (cmd === 'set_default_apps') {
          window.__defaultAppsMock.setCalls.push(args.exts)
          return { openedSettings: true }
        }
        return origInvoke(cmd, args, opts)
      }
    },
  })
  await start(page)

  // Open Settings -> General
  await page.keyboard.press('Control+,')
  await page.waitForSelector('.settings .default-apps')

  // Normal mode: docx shows "Opens in Writan"; xlsx & csv (with null xlsm/tsv) are enabled, not disabled!
  await page.waitForFunction(() => document.querySelectorAll('.default-app button').length > 0 && !document.querySelector('.default-app button:disabled'))
  const normalSummary = await page.evaluate(() =>
    Array.from(document.querySelectorAll('.default-app')).map((row) => ({
      name: row.querySelector('.default-app-name')?.textContent?.trim(),
      status: row.querySelector('button') ? (row.querySelector('button').disabled ? 'disabled' : 'enabled') : row.querySelector(':scope > .faint')?.textContent?.trim(),
    })),
  )
  check(
    'Windows normal detection enables Excel/CSV buttons despite unregistered secondary extensions and marks Word default',
    normalSummary.some((r) => r.name?.startsWith('Word Document') && r.status === 'Opens in Writan') &&
      normalSummary.some((r) => r.name?.startsWith('Excel Workbook') && r.status === 'enabled') &&
      normalSummary.some((r) => r.name?.startsWith('CSV File') && r.status === 'enabled') &&
      normalSummary.every((r) => r.status !== 'disabled'),
    JSON.stringify(normalSummary),
  )

  // Detection failure mode: close and reopen Settings with default_apps throwing an error.
  await page.keyboard.press('Escape')
  await page.waitForSelector('.settings', { state: 'detached' })
  await page.evaluate(() => {
    window.__defaultAppsMock.mode = 'error'
    window.__defaultAppsMock.setCalls = []
  })
  await page.keyboard.press('Control+,')
  await page.waitForSelector('.settings .default-apps')
  const alert = page.locator('.settings [role="alert"]')
  await alert.waitFor()
  const alertText = (await alert.textContent())?.trim() ?? ''
  check('detection failure shows clear error message', alertText.includes('Couldn’t check current default apps') && alertText.includes('Registry access denied'), alertText)

  const shortcutBtn = page.getByRole('button', { name: 'Open Windows Default Apps' })
  await shortcutBtn.waitFor()
  const disabledButtonsOnError = await page.locator('.default-app button:disabled').count()
  check('Make default buttons are not permanently disabled when detection fails', disabledButtonsOnError === 0, String(disabledButtonsOnError))

  await shortcutBtn.click()
  await page.waitForFunction(() => window.__defaultAppsMock.setCalls.length === 1)
  const callsAfterShortcut = await page.evaluate(() => window.__defaultAppsMock.setCalls.length)
  check('Open Windows Default Apps shortcut invokes set_default_apps', callsAfterShortcut === 1, String(callsAfterShortcut))

  // All-unknown mode: close and reopen Settings with default_apps returning null for every extension.
  await page.keyboard.press('Escape')
  await page.waitForSelector('.settings', { state: 'detached' })
  await page.evaluate(() => {
    window.__defaultAppsMock.mode = 'unknown'
    window.__defaultAppsMock.setCalls = []
  })
  await page.keyboard.press('Control+,')
  await page.waitForSelector('.settings .default-apps')
  await page.getByRole('button', { name: 'Open Windows Default Apps' }).waitFor()
  const disabledOnUnknown = await page.locator('.default-app button:disabled').count()
  check('Make default buttons remain enabled and shortcut appears when status is unknown', disabledOnUnknown === 0, String(disabledOnUnknown))

  await done(browser, logs)
}
