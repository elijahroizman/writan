// Themes: the built-in ones tint the window, theme files from Writan, T3 Code and VS Code come in,
// a theme's CSS can't load files, the editor previews live and Cancel puts things back, Change
// theme… picks one, and every selector the guide promises theme authors is in the app.
import { readFileSync } from 'node:fs'
import { launch, start, check, done } from './harness.mjs'

const source = readFileSync(new URL('../../../src/lib/themes.ts', import.meta.url), 'utf8')
const hooks = [...source.slice(source.indexOf('THEME_CSS_HOOKS')).matchAll(/selector: '([^']+)'/g)].map((m) => m[1])

// A folder, so the folder sidebar shows.
const init = () => {
  window.__fsFake = { list: () => [{ name: 'notes.md', path: '/themes-probe/notes.md', isDir: false }], isDir: (p) => p === '/themes-probe' }
  localStorage.setItem('workspace.folder', JSON.stringify({ root: '/themes-probe', visible: true }))
}
const { browser, page, logs } = await launch({ tauri: false, viewport: { width: 1280, height: 820 }, init })
const vars = (...names) => page.evaluate((names) => names.map((n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim()), names)
/** HSL saturation of a computed rgb() color. */
const saturation = (color) => {
  const [r, g, b] = color.match(/[\d.]+/g).slice(0, 3).map((v) => Number(v) / 255)
  const max = Math.max(r, g, b)
  const min = Math.min(r, g, b)
  const l = (max + min) / 2
  return max === min ? 0 : (max - min) / (l > 0.5 ? 2 - max - min : max + min)
}
const surface = (name) =>
  page.evaluate((name) => {
    const i = document.createElement('i')
    i.style.color = `var(${name})`
    document.body.append(i)
    const c = getComputedStyle(i).color
    i.remove()
    return c
  }, name)
const addFromJson = async (json) => {
  await page.getByRole('button', { name: 'Add theme' }).click()
  await page.getByRole('button', { name: 'Paste JSON' }).click()
  await page.locator('.add-theme textarea').fill(json)
  await page.locator('.add-theme').getByRole('button', { name: 'Add theme' }).click()
  await page.waitForSelector('.add-theme-note, .add-theme-error')
  const text = await page.locator('.add-theme-note, .add-theme-error').first().textContent()
  await page.keyboard.press('Escape')
  await page.waitForSelector('.add-theme', { state: 'detached' })
  return text
}

try {
  await start(page)
  const seen = new Set()
  const look = async () => {
    for (const h of hooks) if (await page.locator(h).count()) seen.add(h)
  }

  // Built-in themes carry their hue on the surfaces, in both appearances.
  for (const mode of ['light', 'dark']) {
    await page.evaluate((mode) => window.__workspace.useWorkspace.getState().setThemeMode(mode), mode)
    for (const id of ['iris', 'rose', 'grove', 'ocean', 'ember']) {
      await page.evaluate((id) => {
        localStorage.setItem('workspace.appearance', JSON.stringify({ light: id, dark: id, custom: [] }))
      }, id)
      await page.reload()
      await start(page)
      const panel = saturation(await surface('--panel'))
      const chrome = saturation(await surface('--chrome'))
      check(`${id} tints the ${mode} sidebars and title bar`, panel > 0.2 && chrome > 0.2, `panel ${panel.toFixed(2)}, chrome ${chrome.toFixed(2)}`)
    }
  }
  await page.evaluate(() => localStorage.setItem('workspace.appearance', JSON.stringify({ light: 'workspace', dark: 'workspace', custom: [] })))
  await page.evaluate(() => window.__workspace.useWorkspace.getState().setThemeMode('dark'))
  await page.reload()
  await start(page)
  await look()

  // A document shows the workbench's selectors.
  await page.evaluate(() => window.__workspace.useWorkspace.getState().newDocument('docx'))
  await page.waitForSelector('.workbench .canvas')
  await look()

  // Change theme… lists the themes and uses the one picked.
  await page.keyboard.press('Control+Alt+A')
  await page.waitForSelector('.palette')
  await look()
  check('Change theme… lists the themes', (await page.locator('.palette-item').allTextContents()).some((t) => t.includes('Grove')))
  await page.locator('.palette-input').fill('grove')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)
  check('picking a theme uses it for light and dark', (await page.evaluate(() => JSON.parse(localStorage.getItem('workspace.appearance')))).dark === 'grove')

  await page.keyboard.press('Control+Comma')
  await page.getByRole('button', { name: /Appearance/ }).first().click()
  await page.waitForSelector('.theme-section')
  await look()

  // A Writan theme file, with CSS that tries to load a file and CSS that doesn't.
  const note = await addFromJson(
    JSON.stringify({
      name: 'Probe',
      accent: '#ff00aa',
      light: '#ddeeff',
      dark: '#102030',
      variants: { dark: { panel: '#203040' } },
      css: '.titlebar { outline: 3px solid rgb(1, 2, 3); background-image: url("https://example.invalid/leak.png") } @import "https://example.invalid/x.css";',
    }),
  )
  check('a Writan theme file is added and used', /Added “Probe”/.test(note ?? ''), note)
  const [accent, panel] = await vars('--accent', '--panel')
  check('its accent and its own panel color apply', accent === '#ff00aa' && panel === '#203040', `${accent} ${panel}`)
  const titlebar = await page.locator('.titlebar').evaluate((el) => ({ outline: getComputedStyle(el).outlineColor, image: getComputedStyle(el).backgroundImage }))
  check('its CSS applies', titlebar.outline === 'rgb(1, 2, 3)', titlebar.outline)
  check('but not the parts that load files', titlebar.image === 'none' && !(await page.evaluate(() => [...document.adoptedStyleSheets].some((s) => [...s.cssRules].some((r) => r.cssText.includes('example.invalid'))))), titlebar.image)

  // VS Code's own format, comments and all, and T3 Code's.
  const vscode = await addFromJson(`{
    // A VS Code theme
    "name": "Code Probe",
    "type": "dark",
    "colors": { "editor.background": "#112233", "button.background": "#33aa55", "sideBar.background": "#0a1a2a", },
  }`)
  check('a VS Code theme is added', /Added “Code Probe”/.test(vscode ?? ''), vscode)
  check('its colors become Writan’s', (await vars('--canvas', '--accent', '--panel')).join() === '#112233,#33aa55,#0a1a2a', (await vars('--canvas', '--accent', '--panel')).join())
  const t3 = await addFromJson(JSON.stringify({ name: 'T3 Probe', appearance: 'dark', canvas: '#1a1b26', accent: '#7aa2f7', colors: { text: '#c0caf5' } }))
  check('a T3 Code theme is added', /Added “T3 Probe”/.test(t3 ?? ''), t3)
  check('its text color comes along', (await vars('--fg'))[0] === '#c0caf5', (await vars('--fg'))[0])
  const bad = await addFromJson('{"name": "Nothing"}')
  check('a file that isn’t a theme says so', /isn’t a Writan, T3 Code or VS Code theme/.test(bad ?? ''), bad)

  // The editor previews live, and Cancel puts the theme back.
  const before = (await vars('--accent'))[0]
  await page.getByRole('button', { name: 'Create theme' }).click()
  await page.waitForSelector('.theme-panel')
  await look()
  await page.locator('.theme-panel input[type=color]').first().evaluate((el) => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, '#00ffaa')
    el.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await page.waitForTimeout(100)
  check('the editor shows a change at once', (await vars('--accent'))[0] === '#00ffaa', (await vars('--accent'))[0])
  check('Esc in the editor leaves Settings open', await page.locator('.theme-panel .field input').first().press('Escape').then(() => page.locator('.settings').count()) === 1)
  check('closing it puts the theme back', (await vars('--accent'))[0] === before, (await vars('--accent'))[0])

  // Saving keeps it; its menu has the actions.
  await page.getByRole('button', { name: 'Create theme' }).click()
  await page.locator('.theme-panel .field input').first().fill('Saved Probe')
  await page.getByRole('button', { name: 'Save theme' }).click()
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('workspace.appearance')))
  check('a saved theme is kept and used', stored.custom.some((t) => t.name === 'Saved Probe' && stored.dark === t.id))
  await page.getByRole('button', { name: 'More for Saved Probe' }).click()
  await look()
  const items = await page.locator('.menu .menu-item').allTextContents()
  check('a theme’s menu edits, duplicates, exports and deletes', ['Edit…', 'Duplicate and edit…', 'Export…', 'Delete'].every((l) => items.includes(l)), items.join(', '))
  await page.locator('.menu').getByText('Delete', { exact: true }).click()
  await page.waitForTimeout(200)
  await look()
  await page.keyboard.press('Escape')

  const missing = hooks.filter((h) => !seen.has(h))
  check('every selector the theme guide names is in the app', missing.length === 0, missing.join(', '))
} finally {
  await done(browser, logs)
}
