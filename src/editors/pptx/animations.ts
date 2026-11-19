// A slide's object animations in the slideshow: which frames show after each click, and the
// keyframes for PowerPoint's common effects. Effects without their own keyframes fade, and
// motion paths don't move.

import type { AnimationStep, Effect } from '../../engine/slides/engine'

export const NO_STEPS: AnimationStep[] = []

/** How long a step takes, from its start to its last effect's end. */
export function stepDuration(step: AnimationStep | undefined): number {
  return Math.max(0, ...(step?.effects ?? []).map((e) => e.delayMs + e.durationMs))
}

/** The steps that play by themselves when the slide comes up, before the first click. */
export function leadingAutoSteps(steps: readonly AnimationStep[]): number {
  let n = 0
  while (steps[n]?.auto) n++
  return n
}

/** Where going back from `done` steps lands: the click before, or null for the slide before. */
export function stepBack(steps: readonly AnimationStep[], done: number): number | null {
  for (let k = done - 1; k >= leadingAutoSteps(steps); k--) if (!steps[k].auto) return k
  return null
}

/** What an effect animates: a frame (`"7"`), or one paragraph of its text (`"7/p1"`). */
const target = (e: Effect) => (e.paragraph == null ? String(e.frame) : `${e.frame}/p${e.paragraph}`)

/**
 * The frames hidden once `done` steps have played: a frame whose first effect is an entrance
 * is hidden until it plays, and an exit hides its frame.
 */
export function hiddenAfter(steps: readonly AnimationStep[], done: number): Set<string> {
  const hidden = new Set<string>()
  const seen = new Set<string>()
  for (const e of steps.flatMap((s) => s.effects)) {
    if (seen.has(target(e))) continue
    seen.add(target(e))
    if (e.class === 'entr') hidden.add(target(e))
  }
  for (const e of steps.slice(0, done).flatMap((s) => s.effects)) {
    if (e.class === 'entr') hidden.delete(target(e))
    else if (e.class === 'exit') hidden.add(target(e))
  }
  return hidden
}

// The sides PowerPoint's Fly and Wipe subtypes name: where the frame comes from.
const TOP = 1
const RIGHT = 2
const BOTTOM = 4
const LEFT = 8

interface Rect { x: number; y: number; w: number; h: number }

/** Where an element is on the slide, in slide pixels. */
function slideRect(el: HTMLElement): Rect {
  let x = 0
  let y = 0
  for (let at: HTMLElement | null = el; at && !at.classList.contains('slide-surface'); at = at.offsetParent as HTMLElement | null) {
    x += at.offsetLeft
    y += at.offsetTop
  }
  return { x, y, w: el.offsetWidth, h: el.offsetHeight }
}

/** The keyframes of an entrance, or of an emphasis, in slide pixels; exits play entrances backwards. */
function keyframes(e: Effect, frame: Rect, slide: { width: number; height: number }, base: string): Keyframe[] | null {
  const tf = (t: string) => (base ? `${t} ${base}` : t)
  if (e.class === 'path') return null
  if (e.class === 'emph') {
    if (e.preset === 8) return [{ transform: tf('rotate(0deg)') }, { transform: tf('rotate(360deg)') }]
    return [{ transform: tf('scale(1)') }, { transform: tf('scale(1.1)') }, { transform: tf('scale(1)') }]
  }
  const side = e.subtype || BOTTOM
  switch (e.preset) {
    case 1:
      return [{ opacity: 0 }, { opacity: 0, offset: 0.99 }, { opacity: 1 }]
    case 2: {
      const dx = side & LEFT ? -(frame.x + frame.w) : side & RIGHT ? slide.width - frame.x : 0
      const dy = side & TOP ? -(frame.y + frame.h) : side & BOTTOM ? slide.height - frame.y : 0
      return [{ transform: tf(`translate(${dx}px,${dy}px)`) }, { transform: tf('translate(0,0)') }]
    }
    case 22: {
      // A wipe from the bottom shows the bottom first: the top edge is clipped all the way down.
      const from = side & BOTTOM ? '100% 0 0 0' : side & TOP ? '0 0 100% 0' : side & RIGHT ? '0 0 0 100%' : '0 100% 0 0'
      return [{ clipPath: `inset(${from})` }, { clipPath: 'inset(0)' }]
    }
    case 16:
      return [{ clipPath: 'inset(0 50%)' }, { clipPath: 'inset(0)' }]
    case 23:
    case 53:
      return [{ transform: tf('scale(.3)'), opacity: 0 }, { transform: tf('scale(1)'), opacity: 1 }]
    default:
      return [{ opacity: 0 }, { opacity: 1 }]
  }
}

/**
 * Shows a drawn slide (`root` holds its `.slide-surface`) as it is after `done` steps; with
 * `play`, step `done` plays into that state. Returns a cleanup that stops what's playing.
 */
export function showSteps(root: HTMLElement, steps: readonly AnimationStep[], done: number, play: boolean, slide: { width: number; height: number }): () => void {
  const frames = new Map<string, HTMLElement>()
  root.querySelectorAll<HTMLElement>('.slide-surface > [data-id]').forEach((el) => {
    frames.set(el.dataset.id!, el)
    el.querySelectorAll<HTMLElement>('[data-id^="p"]').forEach((p) => frames.set(`${el.dataset.id}/${p.dataset.id}`, p))
  })
  const show = (n: number) => {
    const hidden = hiddenAfter(steps, n)
    frames.forEach((el, id) => (el.style.visibility = hidden.has(id) ? 'hidden' : ''))
  }
  const playing = play && done > 0 ? steps[done - 1] : undefined
  show(playing ? done - 1 : done)
  if (!playing) return () => {}
  const animations: Animation[] = []
  for (const e of playing.effects) {
    const el = frames.get(target(e))
    if (!el) continue
    const keys = keyframes(e, slideRect(el), slide, el.style.transform)
    if (!keys) continue
    if (e.class === 'entr') el.style.visibility = ''
    const exit = e.class === 'exit'
    animations.push(
      el.animate(exit ? [...keys].reverse().map((k) => ({ ...k, offset: k.offset == null ? undefined : 1 - k.offset })) : keys, {
        duration: Math.max(1, e.durationMs),
        delay: e.delayMs,
        easing: exit ? 'ease-in' : 'ease-out',
        // An entrance stays hidden until it starts, an exit until the step ends.
        fill: exit ? 'forwards' : e.class === 'entr' ? 'backwards' : 'none',
      }),
    )
  }
  Promise.all(animations.map((a) => a.finished))
    .then(() => {
      show(done)
      animations.forEach((a) => a.cancel())
    })
    .catch(() => {})
  return () => animations.forEach((a) => a.cancel())
}
