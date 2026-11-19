// A video link added to a slide: its poster has a neutral play button, a double-click plays it
// while editing, a click plays it in the slideshow (over the slide, not behind it), and picking a
// frame next to it leaves the keyboard on the slide so Delete still removes the frame.
// node tests/stress/ui/slides-video.mjs   (ENGINE=webkit for WebKit)
import { launch, start, check, done } from './harness.mjs'

const { browser, context, page, logs } = await launch({ tauri: false })
// No network: the poster falls back to a plain one, and the player is a stand-in page.
await context.route('https://i.ytimg.com/**', (route) => route.abort())
await context.route('https://www.youtube-nocookie.com/**', (route) => route.fulfill({ contentType: 'text/html', body: '<body style="background:#123">player</body>' }))
try {
  await start(page)
  await page.evaluate(() => window.__workspace.useWorkspace.getState().newDocument('pptx'))
  const canvas = page.getByRole('textbox', { name: 'Slide', exact: true })
  await canvas.waitFor()
  const elements = () => page.evaluate(() => window.__slides.elements)

  await page.getByRole('button', { name: 'Video', exact: true }).click()
  await page.getByPlaceholder('Paste a YouTube or Vimeo link').fill('https://youtu.be/dQw4w9WgXcQ')
  await page.getByRole('button', { name: 'Add', exact: true }).click()
  await page.waitForFunction(() => window.__slides.elements.some((e) => e.link?.url?.includes('youtube')))
  const video = (await elements()).find((e) => e.link?.url?.includes('youtube'))
  const title = (await elements()).find((e) => e.type === 'text' && e.x < video.x)
  check('the video sits over the title, as in the report', !!title && title.y + title.height > video.y && title.x + title.width > video.x, JSON.stringify({ title, video }))

  // A slide point on screen, in the editor or the slideshow.
  const onScreen = (selector, x, y) =>
    page.evaluate(
      ({ selector, x, y }) => {
        const surface = [...document.querySelectorAll(selector)].at(-1)
        const r = surface.getBoundingClientRect()
        const k = r.width / window.__slides.width
        return { x: r.left + x * k, y: r.top + y * k }
      },
      { selector, x, y },
    )
  const topAt = (p) => page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.tagName, p)
  const videoCenter = { x: video.x + video.width / 2, y: video.y + video.height / 2 }

  // The play button is no longer YouTube red: sample the disc to the left of the triangle.
  const disc = await page.evaluate(async ({ id }) => {
    const img = document.querySelector(`.sc-slide .slide-surface [data-id="${id}"] img`)
    if (!img) return null
    await img.decode()
    const c = document.createElement('canvas')
    c.width = img.naturalWidth
    c.height = img.naturalHeight
    const ctx = c.getContext('2d')
    ctx.drawImage(img, 0, 0)
    const [r, g, b] = ctx.getImageData(Math.round(c.width / 2 - 40 * (c.width / 1280)), Math.round(c.height / 2), 1, 1).data
    return { r, g, b }
  }, video)
  check('the poster\'s play button is not red', !!disc && disc.r - Math.max(disc.g, disc.b) < 30, JSON.stringify(disc))

  // Picking the video opens its Link section; picking the title then must not focus the link field.
  const slidePoint = (x, y) => onScreen('.sc-slide .slide-surface', x, y)
  await page.mouse.click(...Object.values(await slidePoint(videoCenter.x, videoCenter.y)))
  check('a picked video offers to play', await page.getByRole('button', { name: 'Play video' }).count() === 1)
  await page.mouse.click(...Object.values(await slidePoint(title.x + 20, title.y + title.height / 2)))
  check('the title is picked', (await page.evaluate(() => window.__slides.state.frames)).join() === title.id)
  check('the keyboard stays on the slide after picking a frame', await page.evaluate(() => document.activeElement?.hasAttribute('data-engine-input')), await page.evaluate(() => document.activeElement?.outerHTML.slice(0, 120)))
  await page.keyboard.press('Backspace')
  check('Backspace deletes the picked title', !(await elements()).some((e) => e.id === title.id))

  // Double-clicking the video plays it on the slide, on top.
  const center = await slidePoint(videoCenter.x, videoCenter.y)
  await page.mouse.dblclick(center.x, center.y)
  await page.locator('.sc-frame-overlay iframe').waitFor()
  check('a double-click plays the video while editing', (await page.locator('.sc-frame-overlay iframe').getAttribute('src'))?.includes('youtube-nocookie.com/embed/dQw4w9WgXcQ'))
  check('the editing player is on top of the slide', await topAt(center) === 'IFRAME', await topAt(center))
  const corner = await slidePoint(20, 20)
  await page.mouse.click(corner.x, corner.y)
  check('clicking elsewhere stops it', await page.locator('.sc-frame-overlay').count() === 0)

  // In the slideshow a click plays it, over the slide rather than behind it.
  await page.evaluate(() => { const w = window.__workspace; w.getEditor(w.useWorkspace.getState().activeId).actions['pptx.present-start']() })
  await page.locator('.present .slide-surface').first().waitFor()
  await page.waitForTimeout(300)
  const shown = await onScreen('.present .slide-surface', videoCenter.x, videoCenter.y)
  await page.mouse.click(shown.x, shown.y)
  await page.locator('.present-video iframe').waitFor()
  check('a click plays the video in the slideshow', await page.locator('.present .present-video iframe').count() === 1)
  check('the slideshow player is on top of the slide', await topAt(shown) === 'IFRAME', await topAt(shown))
  await page.getByRole('button', { name: 'Close video' }).click()
  check('the close button stops it and stays on the slide', await page.locator('.present-video').count() === 0 && await page.locator('.present').count() === 1)
} finally {
  await done(browser, logs)
}
