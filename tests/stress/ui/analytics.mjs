// Usage analytics (src/state/analytics.ts): nothing is sent without a key or before the person
// chooses, what's sent carries no file names or paths, and turning it off forgets the install ID.
import assert from 'node:assert/strict'
import { resolve } from 'node:path'
import { BASE, check, done, launch, open, start } from './harness.mjs'

const path = resolve(import.meta.dirname, '../samples/S1-markdown-latin1.md')
const NAME = 'S1-markdown-latin1'

/** Records analytics_send calls instead of posting them. */
function recordSends() {
  const bridge = window.__TAURI_INTERNALS__
  const invoke = bridge.invoke
  window.sent = []
  bridge.invoke = async (cmd, args, opts) => {
    if (cmd === 'analytics_send') {
      window.sent.push(JSON.parse(JSON.stringify(args)))
      return null
    }
    return invoke(cmd, args, opts)
  }
}

const sends = (page) => page.evaluate(() => window.sent.length)
const installId = (page) => page.evaluate(() => localStorage.getItem('workspace.analyticsId'))
const flush = (page) => page.evaluate(() => window.__analytics.flushAnalytics())

// A build without a key sends nothing and saves no ID, even with the setting on.
{
  const { browser, page, logs } = await launch({ init: recordSends })
  try {
    await start(page)
    await page.evaluate(() => window.__analytics.setAnalytics(true))
    await open(page, path)
    await page.evaluate(() => window.__analytics.track('feature_used', { feature: 'find' }))
    await flush(page)
    assert.equal(await sends(page), 0)
    assert.equal(await installId(page), null)
    check('no key: nothing sent, no install ID', true)
  } finally {
    await done(browser, logs)
  }
}

// With a key: nothing goes until the person turns it on, then the waiting events go.
{
  const { browser, page, logs } = await launch({ init: recordSends })
  await page.addInitScript(() => {
    window.__analyticsTest = { key: 'phc_test' }
  })
  try {
    await start(page)
    assert.equal(await page.evaluate(() => window.__analytics.useAnalytics.getState().choice), null)
    await open(page, path)
    await page.waitForFunction(() => document.querySelector('.cm-content'))
    await flush(page)
    assert.equal(await sends(page), 0, 'nothing is sent before the person chooses')
    assert.equal(await installId(page), null, 'no ID before the person chooses')
    check('undecided: nothing sent', true)

    await page.evaluate(() => window.__analytics.setAnalytics(true))
    await flush(page)
    assert.equal(await sends(page), 1)
    const batch = await page.evaluate(() => window.sent[0])
    assert.equal(batch.key, 'phc_test')
    assert.equal(batch.host, 'https://us.i.posthog.com')
    const names = batch.events.map((e) => e.event)
    for (const name of ['app_launched', 'app_active', 'document_opened']) assert.ok(names.includes(name), `${name} in ${names}`)
    const id = await installId(page)
    assert.match(id, /^[0-9a-f-]{36}$/)
    for (const e of batch.events) {
      assert.equal(e.properties.distinct_id, id)
      assert.equal(e.properties.$process_person_profile, false)
      assert.equal(e.properties.$geoip_disable, true)
      assert.equal(e.properties.channel, 'stable')
      assert.match(e.properties.app_version, /^\d+\.\d+\.\d+/)
    }
    const opened = batch.events.find((e) => e.event === 'document_opened').properties
    assert.equal(opened.kind, 'md')
    assert.equal(opened.extension, 'md')
    assert.equal(opened.size_bucket, '<100KB')
    assert.equal(typeof opened.open_ms, 'number')
    const json = JSON.stringify(batch)
    assert.ok(!json.includes(NAME) && !json.includes(import.meta.dirname), 'no file name or path in the batch')
    check('on: waiting events sent without names or paths', true)

    // Off: the ID goes, and nothing more is sent.
    await page.evaluate(() => {
      window.__analytics.track('feature_used', { feature: 'find' })
      window.__analytics.setAnalytics(false)
      window.__analytics.track('feature_used', { feature: 'print' })
    })
    await flush(page)
    assert.equal(await sends(page), 1)
    assert.equal(await installId(page), null)
    assert.equal(await page.evaluate(() => localStorage.getItem('workspace.analytics')), 'false')
    check('off: queue dropped, ID deleted', true)

    // On again: a new ID.
    await page.evaluate(() => {
      window.__analytics.setAnalytics(true)
      window.__analytics.track('feature_used', { feature: 'find' })
    })
    await flush(page)
    assert.equal(await sends(page), 2)
    const again = await page.evaluate(() => window.sent[1])
    assert.deepEqual(again.events.map((e) => e.event), ['feature_used'])
    assert.notEqual(again.events[0].properties.distinct_id, id)
    check('on again: new ID', true)
  } finally {
    await done(browser, logs)
  }
}

// The default: off in Europe and the UK, by time zone or by language region.
{
  const { browser, logs } = await launch({ tauri: false })
  try {
    const cases = [
      { timezoneId: 'America/New_York', locale: 'en-US', on: true },
      { timezoneId: 'Asia/Tokyo', locale: 'ja-JP', on: true },
      { timezoneId: 'Europe/Berlin', locale: 'de-DE', on: false },
      { timezoneId: 'Atlantic/Reykjavik', locale: 'en-US', on: false },
      { timezoneId: 'Asia/Nicosia', locale: 'en-US', on: false },
      { timezoneId: 'America/New_York', locale: 'en-GB', on: false },
    ]
    for (const c of cases) {
      const context = await browser.newContext({ timezoneId: c.timezoneId, locale: c.locale })
      const page = await context.newPage()
      await page.goto(BASE)
      await page.waitForFunction(() => window.__analytics)
      const on = await page.evaluate(() => window.__analytics.defaultAnalyticsOn())
      assert.equal(on, c.on, `${c.timezoneId} ${c.locale}`)
      await context.close()
    }
    check('default is off in Europe and the UK', true)
  } finally {
    await done(browser, logs)
  }
}
