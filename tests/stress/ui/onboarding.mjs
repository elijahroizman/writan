// The onboarding (src/components/Setup.tsx, src/state/setup.ts): a new install sees it, a copy
// that finished the 0.x one sees the new one once, quitting halfway shows it again, Skip lands on
// the last page, and finishing records the analytics choice, the update channel and an
// onboarding_finished event.
import assert from 'node:assert/strict'
import { check, done, launch, start } from './harness.mjs'

/** Before the app runs: what an earlier launch left, once per tab, and a stand-in for the agent
 * and analytics commands. */
function setup(saved) {
  return [
    ({ saved }) => {
      const bridge = window.__TAURI_INTERNALS__
      const invoke = bridge.invoke
      window.sent = []
      window.agentFiles = {}
      bridge.invoke = async (cmd, args, opts) => {
        if (cmd === 'analytics_send') {
          window.sent.push(...JSON.parse(JSON.stringify(args.events)))
          return null
        }
        if (cmd === 'agents_installed') {
          const has = window.agentFiles[args.agent] ?? {}
          return { mcp: !!has.mcp, skill: !!has.skill, present: args.agent === 'claude' || args.agent === 'codex' }
        }
        if (cmd === 'agents_install_mcp' || cmd === 'agents_install_skill') {
          const what = cmd === 'agents_install_mcp' ? 'mcp' : 'skill'
          ;(window.agentFiles[args.agent] ??= {})[what] = what === 'mcp' ? args.url : true
          return 'Added.'
        }
        return invoke(cmd, args, opts)
      }
      window.__analyticsTest = { key: 'phc_test' }
      if (sessionStorage.getItem('onboarding-test')) return
      sessionStorage.setItem('onboarding-test', '1')
      localStorage.removeItem('workspace.setupDone')
      for (const [key, value] of Object.entries(saved)) localStorage.setItem(key, value)
    },
    { saved },
  ]
}

async function open(saved) {
  const { browser, page, logs } = await launch()
  // After the harness's own scripts, so the bridge is there to wrap.
  await page.addInitScript(...setup(saved))
  await start(page)
  return { browser, page, logs }
}

const dialog = (page) => page.getByRole('dialog', { name: /./ }).and(page.locator('.welcome'))
const step = (page) => page.locator('.welcome').getAttribute('data-step')
const stored = (page, key) => page.evaluate((k) => localStorage.getItem(k), key)

// A new install: it shows, → steps, Skip lands on the last page, which Esc doesn't close, and
// finishing records the choices and the event.
{
  const { browser, page, logs } = await open({})
  try {
    await dialog(page).waitFor()
    check('a new install sees the onboarding', (await step(page)) === 'welcome')
    assert.equal(await stored(page, 'workspace.setupDone'), 'pending')
    check('the dialog is labelled by its title', (await page.getByRole('dialog', { name: 'One app for the documents people send you' }).count()) === 1)
    await page.keyboard.press('ArrowRight')
    check('→ goes to the next page', (await step(page)) === 'collab')
    await page.keyboard.press('ArrowLeft')
    await page.keyboard.press('ArrowRight')
    await page.getByRole('button', { name: 'Skip' }).click()
    check('Skip lands on the last page', (await step(page)) === 'done')
    check('the last page has no Skip', (await page.getByRole('button', { name: 'Skip' }).count()) === 0)
    await page.keyboard.press('Escape')
    await page.keyboard.press('ArrowRight')
    check('Esc and → leave the last page open', (await step(page)) === 'done' && (await stored(page, 'workspace.setupDone')) === 'pending')

    const usage = page.getByRole('switch', { name: 'Help make Writan better' })
    const expected = await page.evaluate(() => window.__analytics.defaultAnalyticsOn())
    check('the usage switch starts at the default for this place', (await usage.getAttribute('aria-checked')) === String(expected))
    if (!expected) await usage.click()
    await page.getByRole('radio', { name: /Nightly/ }).check()
    await page.getByRole('button', { name: 'Start using Writan' }).click()
    await dialog(page).waitFor({ state: 'detached' })
    check('finishing stores the analytics choice', (await stored(page, 'workspace.analytics')) === 'true')
    check('finishing stores the update channel', (await stored(page, 'workspace.updateChannel')) === 'nightly')
    check('finishing stores the new version', (await stored(page, 'workspace.setupDone')) === '2')

    await page.evaluate(() => window.__analytics.flushAnalytics())
    const event = await page.evaluate(() => window.sent.find((e) => e.event === 'onboarding_finished')?.properties)
    check('onboarding_finished is sent', !!event, JSON.stringify(event))
    check(
      'it says Skip was used, three pages were seen, and the channel picked',
      event?.skipped === true && event.steps_seen === 3 && event.steps_total === 5 && event.channel === 'nightly' && event.agents_connected === 0 && event.upgraded === false,
      JSON.stringify(event),
    )

    await page.reload()
    await page.waitForFunction(() => window.__workspace)
    await page.waitForTimeout(600)
    check('it doesn’t show again after finishing', (await page.locator('.welcome').count()) === 0)

    await page.keyboard.press('Control+,')
    await page.getByRole('button', { name: 'Show the tour again' }).click()
    await dialog(page).waitFor()
    check('Settings › General shows it again', (await step(page)) === 'welcome')
  } finally {
    await done(browser, logs)
  }
}

// A copy that finished the 0.x onboarding sees the new one once. Walking every page connects an
// agent, and turning usage data off is stored too.
{
  const { browser, page, logs } = await open({ 'workspace.setupDone': '1', 'workspace.session': '{"paths":[],"active":null}' })
  try {
    await dialog(page).waitFor()
    check('an upgrader with a finished 0.x setup sees it', (await step(page)) === 'welcome')
    const pages = []
    while ((await step(page)) !== 'done') {
      pages.push(await step(page))
      if ((await step(page)) === 'agents') {
        // As if agents had started: the browser has no MCP server of its own.
        await page.evaluate(() => window.__ai.useAgentsStatus.setState({ state: 'on', local: 'http://127.0.0.1:47800/mcp/test', network: [], message: null }))
        await page.getByRole('button', { name: 'Connect Claude Code' }).click()
        await page.locator('.tour-agent', { hasText: 'Claude Code' }).getByText('Connected').waitFor()
        check('Connect adds the MCP server with the agent in its address', (await page.evaluate(() => window.agentFiles.claude?.mcp)) === 'http://127.0.0.1:47800/mcp/test?agent=claude')
        check('only the agents found are listed', (await page.locator('.tour-agent').count()) === 2)
      }
      await page.getByRole('button', { name: /^(Get started|Next)/ }).click()
    }
    check('the pages come in order', pages.join(',') === 'welcome,collab,agents,extras', pages.join(','))
    const usage = page.getByRole('switch', { name: 'Help make Writan better' })
    if ((await usage.getAttribute('aria-checked')) === 'true') await usage.click()
    await page.getByRole('radio', { name: /Stable/ }).check()
    await page.getByRole('button', { name: 'Start using Writan' }).click()
    await dialog(page).waitFor({ state: 'detached' })
    check('turning usage data off is stored', (await stored(page, 'workspace.analytics')) === 'false')
    check('Stable is stored', (await stored(page, 'workspace.updateChannel')) === 'stable')
    await page.evaluate(() => window.__analytics.flushAnalytics())
    check('with usage data off, nothing is sent', (await page.evaluate(() => window.sent.length)) === 0)

    await page.reload()
    await page.waitForFunction(() => window.__workspace)
    await page.waitForTimeout(600)
    check('the upgrader sees it once', (await page.locator('.welcome').count()) === 0)
  } finally {
    await done(browser, logs)
  }
}

// Quitting halfway shows it again on the next launch.
{
  const { browser, page, logs } = await open({ 'workspace.setupDone': '1' })
  try {
    await dialog(page).waitFor()
    await page.getByRole('button', { name: 'Get started' }).click()
    await page.reload()
    await page.waitForFunction(() => window.__workspace)
    await dialog(page).waitFor()
    check('quitting halfway shows it again', (await step(page)) === 'welcome')
  } finally {
    await done(browser, logs)
  }
}
