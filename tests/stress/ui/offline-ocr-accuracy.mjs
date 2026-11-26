import assert from 'node:assert/strict'
import { arabic, downloadPacks, english, hebrew, makeScans, requirePacks, withOcrBrowser } from './offline-ocr.mjs'

const fixtures = [
  { name: 'Clean English', langs: ['eng'], lines: english, maxCer: 0.05, maxWer: 0.15 },
  { name: 'Clean Hebrew', langs: ['heb'], lines: hebrew, font: 'Hebrew', rtl: true, maxCer: 0.12 },
  { name: 'Clean Arabic', langs: ['ara'], lines: arabic, font: 'Arabic', rtl: true, fontSize: 45, leading: 75, maxCer: 0.15 },
  {
    name: 'Mixed Hebrew and English', langs: ['heb', 'eng'], font: 'Hebrew', rtl: true, maxCer: 0.15,
    lines: [hebrew[0], 'A bright morning filled the quiet valley', hebrew[1], 'We walked across the bridge before lunch'],
  },
  { name: 'English rotated 90°', langs: ['eng'], lines: english, degrees: 90, maxCer: 0.15 },
  { name: 'English skewed -6°', langs: ['eng'], lines: english, degrees: -6, maxCer: 0.15 },
]

// Compare NFC Unicode code points and whitespace-delimited words. Ignore formatting
// controls for bidi display and collapse whitespace, preserving punctuation and case.
const normalize = text => text.normalize('NFC').replace(/\p{Bidi_Control}/gu, '').trim().replace(/\s+/gu, ' ')
const words = text => text ? text.split(' ') : []

function levenshtein(expected, actual) {
  let previous = Array.from({ length: actual.length + 1 }, (_, index) => index)
  for (const [row, token] of expected.entries()) {
    const current = [row + 1]
    for (const [column, candidate] of actual.entries()) {
      current.push(Math.min(
        current[column] + 1,
        previous[column + 1] + 1,
        previous[column] + Number(token !== candidate),
      ))
    }
    previous = current
  }
  return previous[actual.length]
}

const percent = rate => `${(100 * rate).toFixed(2)}%`

requirePacks(['eng', 'heb', 'ara'])
await withOcrBrowser(async ({ page, assets, requests }) => {
  await downloadPacks(page, ['eng', 'heb', 'ara'])
  await makeScans(page, Object.fromEntries(fixtures.map(fixture => [fixture.name, fixture])))
  await assets.close()
  const firstRecognition = requests.length
  const results = await page.evaluate(async fixtures => {
    const client = new window.ocrApi.ocrClient.OcrClient()
    try {
      const results = []
      for (const { name, langs } of fixtures) {
        const { width, height, data } = window.ocrScans[name]
        results.push(await client.recognize({ width, height, data }, { langs }))
      }
      return results
    } finally {
      client.dispose()
    }
  }, fixtures)
  console.log('| Fixture | Packs | CER | WER | Reference chars | Recognition ms |')
  console.log('| --- | --- | ---: | ---: | ---: | ---: |')
  const failures = []
  for (const [index, { name, langs, lines, maxCer, maxWer }] of fixtures.entries()) {
    const result = results[index]
    const expected = normalize(lines.join(' '))
    const actual = normalize(result.text)
    const chars = Array.from(expected)
    const expectedWords = words(expected)
    const cer = levenshtein(chars, Array.from(actual)) / chars.length
    const wer = levenshtein(expectedWords, words(actual)) / expectedWords.length
    console.log(`| ${name} | ${langs.join('+')} | ${percent(cer)} | ${percent(wer)} | ${chars.length} | ${result.elapsedMs.toFixed(1)} |`)
    if (cer >= maxCer || (maxWer !== undefined && wer >= maxWer)) {
      failures.push(`${name}: expected "${expected}", got "${actual}"`)
    }
  }
  assert.deepEqual(requests.slice(firstRecognition).filter(url => url.startsWith(assets.url)), [], 'Recognition must use installed packs')
  assert.deepEqual(failures, [], 'OCR accuracy exceeded the permitted error rate')
})
