// A chart on a sheet, drawn as SVG from what the engine says it shows (ChartView): columns,
// bars, lines, areas, pies, doughnuts and scatter plots, with axes, a legend and a tooltip.
// Charts sit on the sheet's white paper, in the workbook theme's colors, as in Excel.

import { useMemo, useState } from 'react'
import type { ChartView } from './engine'

const INK = '#404040'
const MUTED = '#6b6b6b'
const GRID = '#e3e3e3'
const AXIS = '#bfbfbf'
const FONT = 11
const TITLE = 14

/** About how wide text is, for laying out labels without measuring. */
const textWidth = (s: string, size = FONT) => s.length * size * 0.56

/** Round tick values covering lo..hi. */
function ticks(lo: number, hi: number, count: number): number[] {
  if (!(hi > lo)) {
    if (lo === hi) [lo, hi] = lo === 0 ? [0, 1] : lo > 0 ? [0, lo * 1.2] : [lo * 1.2, 0]
    else return [0, 1]
  }
  const raw = (hi - lo) / Math.max(1, count)
  const mag = 10 ** Math.floor(Math.log10(raw))
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= raw) ?? 10 * mag
  const start = Math.floor(lo / step + 1e-9) * step
  const out: number[] = []
  for (let v = start; v <= hi + step * 0.5 && out.length < 50; v += step) out.push(Number(v.toPrecision(12)))
  if (out[out.length - 1] < hi - 1e-9) out.push(Number((out[out.length - 1] + step).toPrecision(12)))
  return out
}

/** 1,234 · 12.9K · 4.2M, or a percent. */
function compact(v: number, percent = false) {
  if (percent) return `${Math.round(v * 100)}%`
  const a = Math.abs(v)
  const fmt = (n: number, unit: string) => `${Number(n.toFixed(n < 10 ? 1 : 0))}${unit}`
  if (a >= 1e9) return fmt(v / 1e9, 'B')
  if (a >= 1e6) return fmt(v / 1e6, 'M')
  if (a >= 1e4) return fmt(v / 1e3, 'K')
  if (a >= 1000) return v.toLocaleString(undefined, { maximumFractionDigits: 0 })
  return String(Number(v.toPrecision(6)))
}

const full = (v: number) => v.toLocaleString(undefined, { maximumFractionDigits: 6 })

function clip(s: string, room: number, size = FONT) {
  if (textWidth(s, size) <= room) return s
  const n = Math.max(1, Math.floor(room / (size * 0.56)) - 1)
  return s.length > n ? `${s.slice(0, n)}…` : s
}

interface Tip {
  x: number
  y: number
  title: string
  lines: { color: string; label: string; value: string }[]
}

/** Each series' color: its own, or the theme's next accent. */
function seriesColor(c: ChartView, i: number) {
  return c.series[i]?.color ?? c.palette[i % c.palette.length] ?? '#4472C4'
}

export function ChartSvg({ chart, width, height }: { chart: ChartView; width: number; height: number }) {
  const [tip, setTip] = useState<Tip | null>(null)
  const pie = chart.kind === 'pie' || chart.kind === 'doughnut'
  // What the legend lists: series, or a pie's categories.
  const legendItems = useMemo(
    () =>
      pie
        ? chart.categories.map((c, i) => ({ label: c || `${i + 1}`, color: chart.palette[i % chart.palette.length] }))
        : chart.series.map((s, i) => ({ label: s.name, color: seriesColor(chart, i) })),
    [chart, pie],
  )
  const pad = 12
  let top = pad
  let left = pad
  let right = width - pad
  let bottom = height - pad
  const title = chart.title?.trim()
  if (title) top += TITLE + 10
  const legend = chart.legend && legendItems.length > 0 ? chart.legend : null
  const legendRows: { label: string; color: string; x: number; y: number }[] = []
  if (legend === 'bottom' || legend === 'top') {
    // Rows of swatches, centered.
    const items = legendItems.map((it) => ({ ...it, w: 14 + Math.min(textWidth(it.label), 140) + 14 }))
    const rows: (typeof items)[] = [[]]
    let rowW = 0
    for (const it of items) {
      if (rowW + it.w > width - 2 * pad && rows[rows.length - 1].length) {
        rows.push([])
        rowW = 0
      }
      rows[rows.length - 1].push(it)
      rowW += it.w
    }
    const shown = rows.slice(0, 3)
    const h = shown.length * 16 + 6
    const y0 = legend === 'bottom' ? bottom - h + 12 : top + 2
    shown.forEach((row, ri) => {
      const total = row.reduce((s, it) => s + it.w, 0)
      let x = (width - total) / 2
      for (const it of row) {
        legendRows.push({ label: it.label, color: it.color, x, y: y0 + ri * 16 })
        x += it.w
      }
    })
    if (legend === 'bottom') bottom -= h + 4
    else top += h
  } else if (legend === 'right' || legend === 'left') {
    const w = Math.min(Math.max(...legendItems.map((it) => textWidth(it.label))) + 24, width * 0.35)
    const n = Math.min(legendItems.length, Math.floor((bottom - top) / 18))
    const y0 = (top + bottom) / 2 - (n * 18) / 2 + 6
    legendItems.slice(0, n).forEach((it, i) => legendRows.push({ ...it, label: clip(it.label, w - 20), x: legend === 'right' ? right - w + 6 : left, y: y0 + i * 18 }))
    if (legend === 'right') right -= w + 6
    else left += w + 6
  }

  const show = (e: React.MouseEvent, t: Omit<Tip, 'x' | 'y'>) => {
    const box = (e.currentTarget as SVGElement).ownerSVGElement!.getBoundingClientRect()
    setTip({ x: e.clientX - box.left, y: e.clientY - box.top, ...t })
  }
  const hide = () => setTip(null)

  let plot: React.ReactNode
  if (pie) plot = <PiePlot chart={chart} box={{ left, top, right, bottom }} show={show} hide={hide} />
  else if (chart.kind === 'other') plot = <text x={width / 2} y={(top + bottom) / 2} textAnchor="middle" fill={MUTED} fontSize={12}>This kind of chart isn’t shown here</text>
  else plot = <AxisPlot chart={chart} box={{ left, top, right, bottom }} show={show} hide={hide} />

  return (
    <div className="sg-chart-svg" onMouseLeave={hide}>
      <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} fontFamily="Calibri, Carlito, 'Segoe UI', sans-serif" fontSize={FONT}>
        <rect x={0.5} y={0.5} width={width - 1} height={height - 1} fill="#fff" stroke="#d9d9d9" />
        {title && (
          <text x={width / 2} y={pad + TITLE} textAnchor="middle" fill={INK} fontSize={TITLE}>
            {clip(title, width - 2 * pad, TITLE)}
          </text>
        )}
        {plot}
        {legendRows.map((it, i) => (
          <g key={i} transform={`translate(${it.x} ${it.y})`}>
            <rect x={0} y={-8} width={9} height={9} rx={2} fill={it.color} />
            <text x={14} y={0} fill={INK}>
              {clip(it.label, 140)}
            </text>
          </g>
        ))}
      </svg>
      {tip && (
        <div className="sg-chart-tip" style={{ left: Math.min(tip.x + 12, width - 140), top: Math.max(4, tip.y - 12) }}>
          {tip.title && <strong>{tip.title}</strong>}
          {tip.lines.map((l, i) => (
            <span key={i}>
              <i style={{ background: l.color }} />
              {l.label && <em>{l.label}</em>}
              {l.value}
            </span>
          ))}
        </div>
      )}
    </div>
  )
}

interface PlotProps {
  chart: ChartView
  box: { left: number; top: number; right: number; bottom: number }
  show: (e: React.MouseEvent, t: Omit<Tip, 'x' | 'y'>) => void
  hide: () => void
}

function PiePlot({ chart, box, show, hide }: PlotProps) {
  const s = chart.series[0]
  if (!s) return null
  const values = s.values.map((v) => (v && v > 0 ? v : 0))
  const total = values.reduce((a, b) => a + b, 0)
  const cx = (box.left + box.right) / 2
  const cy = (box.top + box.bottom) / 2
  const r = Math.max(4, Math.min(box.right - box.left, box.bottom - box.top) / 2 - 2)
  const hole = chart.kind === 'doughnut' ? r * 0.55 : 0
  if (!total) return <circle cx={cx} cy={cy} r={r} fill="none" stroke={GRID} />
  let a = -Math.PI / 2
  return (
    <g>
      {values.map((v, i) => {
        if (!v) return null
        const sweep = (v / total) * Math.PI * 2
        const a2 = a + sweep
        const p = (ang: number, rad: number) => `${cx + rad * Math.cos(ang)} ${cy + rad * Math.sin(ang)}`
        const large = sweep > Math.PI ? 1 : 0
        const d =
          sweep >= Math.PI * 2 - 1e-6
            ? `M ${p(0, r)} A ${r} ${r} 0 1 1 ${p(Math.PI, r)} A ${r} ${r} 0 1 1 ${p(0, r)} Z` + (hole ? ` M ${p(0, hole)} A ${hole} ${hole} 0 1 0 ${p(Math.PI, hole)} A ${hole} ${hole} 0 1 0 ${p(0, hole)} Z` : '')
            : hole
              ? `M ${p(a, r)} A ${r} ${r} 0 ${large} 1 ${p(a2, r)} L ${p(a2, hole)} A ${hole} ${hole} 0 ${large} 0 ${p(a, hole)} Z`
              : `M ${cx} ${cy} L ${p(a, r)} A ${r} ${r} 0 ${large} 1 ${p(a2, r)} Z`
        a = a2
        const color = chart.palette[i % chart.palette.length]
        return (
          <path
            key={i}
            d={d}
            fill={color}
            stroke="#fff"
            strokeWidth={2}
            fillRule="evenodd"
            onMouseMove={(e) => show(e, { title: chart.categories[i] ?? '', lines: [{ color, label: '', value: `${full(v)} (${Math.round((v / total) * 1000) / 10}%)` }] })}
            onMouseLeave={hide}
          />
        )
      })}
    </g>
  )
}

function AxisPlot({ chart, box, show, hide }: PlotProps) {
  const horizontal = chart.kind === 'bar'
  const scatter = chart.kind === 'scatter'
  const stacked = chart.grouping !== 'standard' && !scatter
  const percent = chart.grouping === 'percentStacked' && !scatter
  const n = scatter ? 0 : Math.max(chart.categories.length, ...chart.series.map((s) => s.values.length))
  // Values as plotted: stacked series start where the ones before them end.
  const base: number[][] = []
  const tops: number[][] = []
  const totals = Array.from({ length: n }, (_, i) => chart.series.reduce((sum, s) => sum + Math.abs(s.values[i] ?? 0), 0))
  const posAcc = new Array(n).fill(0)
  const negAcc = new Array(n).fill(0)
  chart.series.forEach((s, si) => {
    base[si] = []
    tops[si] = []
    for (let i = 0; i < n; i++) {
      let v = s.values[i] ?? 0
      if (percent) v = totals[i] ? v / totals[i] : 0
      if (stacked) {
        const acc = v >= 0 ? posAcc : negAcc
        base[si][i] = acc[i]
        acc[i] += v
        tops[si][i] = acc[i]
      } else {
        base[si][i] = 0
        tops[si][i] = v
      }
    }
  })
  let lo = 0
  let hi = 0
  for (const t of tops) for (const v of t) if (Number.isFinite(v)) [lo, hi] = [Math.min(lo, v), Math.max(hi, v)]
  if (scatter) {
    const ys = chart.series.flatMap((s) => s.values.filter((v): v is number => v != null))
    lo = Math.min(0, ...ys)
    hi = Math.max(0, ...ys)
  }
  if (percent) [lo, hi] = [Math.min(lo, 0), Math.max(hi, 1) > 1.0001 ? hi : 1]
  // Value axis room: the widest tick label.
  const valueTicks = ticks(lo, hi, horizontal ? Math.max(2, Math.floor((box.right - box.left) / 70)) : Math.max(2, Math.floor((box.bottom - box.top) / 40)))
  const vmin = valueTicks[0]
  const vmax = valueTicks[valueTicks.length - 1]
  const labelW = Math.max(...valueTicks.map((t) => textWidth(compact(t, percent))))
  const catLabels = chart.categories.length ? chart.categories : Array.from({ length: n }, (_, i) => String(i + 1))
  const catW = horizontal ? Math.min(Math.max(...catLabels.map((c) => textWidth(c)), 10), (box.right - box.left) * 0.3) : 0
  // Axis titles sit where the axes are: along the bottom, turned on the side — when the
  // plot still fits with them. A tight chart keeps its plot and drops the titles.
  const xTitle = (horizontal ? chart.valTitle : chart.catTitle)?.trim()
  const yTitle = (horizontal ? chart.catTitle : chart.valTitle)?.trim()
  const right = box.right - 4
  const top = box.top + 4
  const showX = !!xTitle && box.bottom - (horizontal ? FONT + 6 : FONT + 8) - (FONT + 8) - top >= 10
  const showY = !!yTitle && right - box.left - (horizontal ? catW + 6 : labelW + 6) - (FONT + 6) >= 10
  const left = box.left + (horizontal ? catW + 6 : labelW + 6) + (showY ? FONT + 6 : 0)
  const bottom = box.bottom - (horizontal ? FONT + 6 : FONT + 8) - (showX ? FONT + 8 : 0)
  if (right - left < 10 || bottom - top < 10) return null

  // Scatter's x axis is numbers too.
  const xs = scatter ? chart.series.flatMap((s) => (s.x ?? s.values.map((_, i) => i + 1)).filter((v): v is number => v != null)) : []
  const xTicks = scatter ? ticks(Math.min(...xs, 0), Math.max(...xs, 1), Math.max(2, Math.floor((right - left) / 70))) : []
  const valuePos = (v: number) =>
    horizontal ? left + ((v - vmin) / (vmax - vmin)) * (right - left) : bottom - ((v - vmin) / (vmax - vmin)) * (bottom - top)
  const xPos = (v: number) => left + ((v - xTicks[0]) / (xTicks[xTicks.length - 1] - xTicks[0])) * (right - left)
  const band = (horizontal ? bottom - top : right - left) / Math.max(1, n)
  const catPos = (i: number) => (horizontal ? top + band * (i + 0.5) : left + band * (i + 0.5))
  const zero = valuePos(Math.max(vmin, Math.min(0, vmax)))
  const els: React.ReactNode[] = []

  // Gridlines and value labels.
  for (const t of valueTicks) {
    const p = valuePos(t)
    els.push(
      horizontal ? (
        <g key={`g${t}`}>
          <line x1={p} x2={p} y1={top} y2={bottom} stroke={GRID} />
          <text x={p} y={bottom + FONT + 4} textAnchor="middle" fill={MUTED}>
            {compact(t, percent)}
          </text>
        </g>
      ) : (
        <g key={`g${t}`}>
          <line x1={left} x2={right} y1={p} y2={p} stroke={GRID} />
          <text x={left - 6} y={p + 4} textAnchor="end" fill={MUTED}>
            {compact(t, percent)}
          </text>
        </g>
      ),
    )
  }
  if (scatter) {
    for (const t of xTicks) {
      els.push(
        <text key={`x${t}`} x={xPos(t)} y={bottom + FONT + 4} textAnchor="middle" fill={MUTED}>
          {compact(t)}
        </text>,
      )
    }
  } else {
    // Category labels, skipping some when they'd overlap.
    const each = horizontal ? FONT + 2 : Math.max(...catLabels.map((c) => Math.min(textWidth(c), 120))) + 8
    const step = Math.max(1, Math.ceil(each / Math.max(1, band)))
    catLabels.forEach((c, i) => {
      if (i % step) return
      els.push(
        horizontal ? (
          <text key={`c${i}`} x={left - 6} y={catPos(i) + 4} textAnchor="end" fill={MUTED}>
            {clip(c, catW)}
          </text>
        ) : (
          <text key={`c${i}`} x={catPos(i)} y={bottom + FONT + 6} textAnchor="middle" fill={MUTED}>
            {clip(c, band * step - 4)}
          </text>
        ),
      )
    })
  }
  // The baseline.
  els.push(horizontal ? <line key="axis" x1={zero} x2={zero} y1={top} y2={bottom} stroke={AXIS} /> : <line key="axis" x1={left} x2={right} y1={zero} y2={zero} stroke={AXIS} />)
  if (showX && xTitle)
    els.push(
      <text key="xt" x={(left + right) / 2} y={box.bottom - 4} textAnchor="middle" fill={MUTED}>
        {clip(xTitle, right - left)}
      </text>,
    )
  if (showY && yTitle)
    els.push(
      <text key="yt" transform={`translate(${box.left + FONT} ${(top + bottom) / 2}) rotate(-90)`} textAnchor="middle" fill={MUTED}>
        {clip(yTitle, bottom - top)}
      </text>,
    )

  const tipFor = (i: number) => ({
    title: catLabels[i] ?? '',
    lines: chart.series.map((s, si) => ({ color: seriesColor(chart, si), label: s.name, value: s.values[i] == null ? '—' : full(s.values[i]!) })),
  })

  if (chart.kind === 'column' || chart.kind === 'bar') {
    const groups = stacked ? 1 : chart.series.length
    const slot = band * 0.62
    const thick = Math.max(1, Math.min(slot / groups, 48))
    const gap = groups > 1 && thick > 6 ? 2 : 0
    chart.series.forEach((s, si) => {
      const color = seriesColor(chart, si)
      for (let i = 0; i < n; i++) {
        if (s.values[i] == null) continue
        const a = valuePos(base[si][i])
        const b = valuePos(tops[si][i])
        const offset = stacked ? -thick / 2 : -(groups * thick) / 2 + si * thick
        const c = catPos(i) + offset + gap / 2
        const t = thick - gap
        const lo2 = Math.min(a, b)
        const len = Math.abs(b - a)
        if (len < 0.5) continue
        // Stacked segments are separated by a 2px gap in the paper's color.
        const inset = stacked && si > 0 ? 1 : 0
        const rect = horizontal ? { x: lo2 + inset, y: c, width: Math.max(0.5, len - inset), height: t } : { x: c, y: lo2, width: t, height: Math.max(0.5, len - inset) }
        els.push(
          <rect key={`b${si}-${i}`} {...rect} rx={Math.min(3, t / 4)} fill={color} onMouseMove={(e) => show(e, tipFor(i))} onMouseLeave={hide} />,
        )
      }
    })
  } else if (chart.kind === 'line' || chart.kind === 'area') {
    const order = chart.kind === 'area' && stacked ? [...chart.series.keys()].reverse() : [...chart.series.keys()]
    for (const si of order) {
      const s = chart.series[si]
      const color = seriesColor(chart, si)
      const pts: [number, number][] = []
      for (let i = 0; i < n; i++) if (s.values[i] != null || stacked) pts.push([catPos(i), valuePos(tops[si][i])])
      if (!pts.length) continue
      const line = pts.map((p, k) => `${k ? 'L' : 'M'} ${p[0]} ${p[1]}`).join(' ')
      if (chart.kind === 'area') {
        const floor = stacked && si > 0 ? Array.from({ length: n }, (_, i) => [catPos(i), valuePos(base[si][i])] as [number, number]).reverse() : [[pts[pts.length - 1][0], zero], [pts[0][0], zero]]
        els.push(<path key={`a${si}`} d={`${line} ${floor.map((p) => `L ${p[0]} ${p[1]}`).join(' ')} Z`} fill={color} fillOpacity={stacked ? 0.55 : 0.18} />)
      }
      els.push(<path key={`l${si}`} d={line} fill="none" stroke={color} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />)
      if (pts.length <= 40 && chart.kind === 'line')
        pts.forEach((p, k) => els.push(<circle key={`p${si}-${k}`} cx={p[0]} cy={p[1]} r={3} fill={color} stroke="#fff" strokeWidth={1.5} />))
    }
    // Hover bands, one per category.
    for (let i = 0; i < n; i++) {
      els.push(<rect key={`h${i}`} x={catPos(i) - band / 2} y={top} width={band} height={bottom - top} fill="transparent" onMouseMove={(e) => show(e, tipFor(i))} onMouseLeave={hide} />)
    }
  } else if (scatter) {
    chart.series.forEach((s, si) => {
      const color = seriesColor(chart, si)
      const xsOf = s.x ?? s.values.map((_, i) => i + 1)
      s.values.forEach((v, i) => {
        const x = xsOf[i]
        if (v == null || x == null) return
        els.push(
          <circle
            key={`s${si}-${i}`}
            cx={xPos(x)}
            cy={valuePos(v)}
            r={4}
            fill={color}
            stroke="#fff"
            strokeWidth={2}
            onMouseMove={(e) => show(e, { title: s.name, lines: [{ color, label: '', value: `${full(x)}, ${full(v)}` }] })}
            onMouseLeave={hide}
          />,
        )
      })
    })
  }
  return <g>{els}</g>
}
