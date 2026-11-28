import { resolveTransitionDurationMs } from './transitions'
import type { ReactNode } from 'react'
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import * as platform from '../../lib/platform'
import { track } from '../../state/analytics'
import { SlideImage } from '../../engine/slides/SlideImage'
import { SlideSwap } from './SlideSwap'
import type { Element, SlideInfo, SlidesEngine } from '../../engine/slides/engine'
import { parseVideoLink } from './video'
import { VideoFrame } from './VideoFrame'
import type { AnimationStep } from '../../engine/slides/engine'
import { NO_STEPS, hiddenAfter, showSteps, stepBack, stepDuration } from './animations'

const resolveDuration = (slide: SlideInfo | undefined) => resolveTransitionDurationMs(slide?.transition)

/**
 * Full-screen slideshow. Arrow keys, space or a click advance; esc ends. Clicking an object
 * with a link opens it, and video links play right on the slide. Moving forward plays the
 * slide's animations a click at a time, then the next slide's transition, and a slide set to
 * advance after a time does so.
 */
export function PresentMode({
  engine,
  startIndex,
  onExit,
}: {
  engine: SlidesEngine
  startIndex: number
  onExit: (lastIndex: number) => void
}) {
  // Hidden slides are skipped, but we report positions in the full deck.
  const order = useMemo(() => engine.slides.map((s, i) => ({ s, i })).filter(({ s }) => !s.hidden), [engine, engine.slides])
  const [pos, setPos] = useState(() => Math.max(0, order.findIndex(({ i }) => i >= startIndex)))
  // The slide being left, while the next one's transition plays over it.
  const [swap, setSwap] = useState<{ from: number; run: number }>({ from: -1, run: 0 })
  const [video, setVideo] = useState<{ embed: string; rect: { left: number; top: number; width: number; height: number } } | null>(null)
  const [box, setBox] = useState({ width: window.innerWidth, height: window.innerHeight })
  // How many of the slide's animation steps have played, and whether the last one plays now.
  const [build, setBuild] = useState({ done: 0, play: false })
  const stageRef = useRef<HTMLDivElement>(null)
  const exitRef = useRef(onExit)
  exitRef.current = onExit
  const elementsCacheRef = useRef(new Map<string, Element[]>())

  useEffect(() => track('feature_used', { feature: 'present_slides', kind: 'pptx' }), [])

  // Pre-warm the next slide in idle time so advancing is instant.
  useEffect(() => {
    const next = order[pos + 1]
    if (!next) return
    const warm = () => {
      engine.html(next.i)
    }
    if (typeof requestIdleCallback === 'function') {
      const id = requestIdleCallback(warm, { timeout: 200 })
      return () => cancelIdleCallback(id)
    }
    const id = window.setTimeout(warm, 50)
    return () => window.clearTimeout(id)
  }, [engine, order, pos])

  const current = order[pos]
  const steps = current?.s.animations ?? NO_STEPS
  const finish = useCallback(() => exitRef.current(order[pos]?.i ?? startIndex), [order, pos, startIndex])
  const go = useCallback(
    (delta: number) => {
      setVideo(null)
      if (delta > 0 && build.done < steps.length) return setBuild({ done: build.done + 1, play: true })
      const back = delta < 0 ? stepBack(steps, build.done) : null
      if (back != null) return setBuild({ done: back, play: false })
      const next = Math.min(order.length - 1, Math.max(0, pos + delta))
      if (next === pos) return
      setSwap((s) => ({ from: delta > 0 ? pos : -1, run: s.run + 1 }))
      setPos(next)
      // Going back shows the slide before as it was left.
      setBuild({ done: delta > 0 ? 0 : (order[next].s.animations?.length ?? 0), play: false })
    },
    [order, pos, build.done, steps],
  )
  const jump = (next: number) => {
    setSwap((s) => ({ from: -1, run: s.run + 1 }))
    setPos(next)
    setBuild({ done: 0, play: false })
  }

  // The time until what's playing has finished: the step that just played, or the transition.
  const settling = build.play ? stepDuration(steps[build.done - 1]) : build.done === 0 && swap.from >= 0 ? resolveDuration(current?.s) : 0

  // Steps set to start with or after the previous one play by themselves.
  useEffect(() => {
    if (!steps[build.done]?.auto) return
    const timer = window.setTimeout(() => setBuild({ done: build.done + 1, play: true }), settling)
    return () => window.clearTimeout(timer)
  }, [steps, build.done, settling])

  // Advance on its own after the slide's timing, as PowerPoint does.
  const advanceAfter = current?.s.transition?.afterMs
  useEffect(() => {
    if (advanceAfter == null || video || pos >= order.length - 1 || build.done < steps.length) return
    const timer = window.setTimeout(() => go(1), advanceAfter + settling)
    return () => window.clearTimeout(timer)
  }, [advanceAfter, video, pos, order.length, go, build.done, steps.length, settling])

  useEffect(() => {
    void platform.setFullscreen(true)
    return () => {
      void platform.setFullscreen(false)
    }
  }, [])

  // Measure the stage itself. Full screen changes its box after
  // window.innerHeight, and a stale size scales the slide past the screen.
  useEffect(() => {
    const el = stageRef.current
    if (!el) return
    const measure = () => {
      const rect = el.getBoundingClientRect()
      if (rect.width > 0 && rect.height > 0) setBox({ width: rect.width, height: rect.height })
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (['ArrowRight', 'ArrowDown', ' ', 'PageDown', 'Enter', 'n'].includes(e.key)) go(1)
      else if (['ArrowLeft', 'ArrowUp', 'PageUp', 'Backspace', 'p'].includes(e.key)) go(-1)
      else if (e.key === 'Home') jump(0)
      else if (e.key === 'End') jump(order.length - 1)
      else if (e.key === 'Escape') (video ? setVideo(null) : finish())
      else return
      e.preventDefault()
      e.stopImmediatePropagation()
    }
    // Capture phase so the editor underneath never sees these keys.
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [go, finish, order.length, video])

  const scale = Math.min(box.width / engine.width, box.height / engine.height)
  const width = engine.width * scale
  const height = engine.height * scale
  const left = (box.width - width) / 2
  const top = (box.height - height) / 2

  const onClick = (e: React.MouseEvent) => {
    if (!current) return
    // Hit-test linked objects, topmost first.
    const stage = stageRef.current!.getBoundingClientRect()
    const x = (e.clientX - stage.left - left) / scale
    const y = (e.clientY - stage.top - top) / scale
    const cacheKey = `${current.s.id}:${current.s.revision}`
    let elements = elementsCacheRef.current.get(cacheKey)
    if (!elements) {
      elements = JSON.parse(engine.doc.elements(current.i)) as Element[]
      if (elementsCacheRef.current.size > 32) elementsCacheRef.current.clear()
      elementsCacheRef.current.set(cacheKey, elements)
    }
    const hit = linkedAt(elements, x, y, hiddenAfter(steps, build.done))
    const url = hit?.link?.url
    if (hit && url) {
      const videoLink = parseVideoLink(url)
      if (videoLink && videoLink.provider !== 'other') {
        setVideo({ embed: videoLink.embedUrl, rect: { left: left + hit.x * scale, top: top + hit.y * scale, width: hit.width * scale, height: hit.height * scale } })
      } else {
        void platform.openExternal(url)
      }
      return
    }
    if (video) setVideo(null)
    else if (build.done < steps.length) go(1)
    else if (current.s.transition?.onClick === false) return
    else if (pos >= order.length - 1) finish()
    else go(1)
  }

  return (
    <div className="present" onClick={onClick} ref={stageRef}>
      {current && (
        <div style={{ position: 'absolute', left, top, width, height }}>
          <SlideSwap
            run={swap.run}
            transition={current.s.transition}
            outgoing={
              order[swap.from] && (
                <Steps engine={engine} steps={order[swap.from].s.animations ?? NO_STEPS} done={order[swap.from].s.animations?.length ?? 0} play={false}>
                  <SlideImage engine={engine} index={order[swap.from].i} revision={order[swap.from].s.revision} width={width} lazy={false} />
                </Steps>
              )
            }
            incoming={
              <Steps key={current.s.id} engine={engine} steps={steps} done={build.done} play={build.play}>
                <SlideImage engine={engine} index={current.i} revision={current.s.revision} width={width} lazy={false} />
              </Steps>
            }
          />
        </div>
      )}
      {video && <VideoFrame embed={video.embed} style={video.rect} onClose={() => setVideo(null)} />}
      <div className="present-hud">
        {pos + 1} / {order.length} · Esc to exit
      </div>
    </div>
  )
}

/** A drawn slide shown as it is after `done` animation steps, with step `done` playing if `play`. */
function Steps({ engine, steps, done, play, children }: { engine: SlidesEngine; steps: readonly AnimationStep[]; done: number; play: boolean; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null)
  useLayoutEffect(() => {
    if (!ref.current || !steps.length) return
    return showSteps(ref.current, steps, done, play, engine)
  }, [engine, steps, done, play])
  return (
    <div ref={ref} style={{ display: 'contents' }}>
      {children}
    </div>
  )
}

/** Linked frames are hit in paint order, using their rotated bounds; frames an animation hides aren't. */
function linkedAt(elements: readonly Element[], x: number, y: number, hidden: ReadonlySet<string>): Element | undefined {
  for (const el of [...elements].reverse()) {
    if (hidden.has(el.id)) continue
    const child = el.children && linkedAt(el.children, x, y, hidden)
    if (child) return child
    if (!el.link?.url) continue
    const angle = -el.rotation * Math.PI / 180
    const dx = x - el.x - el.width / 2
    const dy = y - el.y - el.height / 2
    const lx = dx * Math.cos(angle) - dy * Math.sin(angle)
    const ly = dx * Math.sin(angle) + dy * Math.cos(angle)
    if (Math.abs(lx) <= el.width / 2 && Math.abs(ly) <= el.height / 2) return el
  }
}
