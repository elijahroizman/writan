// Which color role paints something on screen, for the theme editor's picker: click anything in
// the app and the editor jumps to its color. Each role's custom property is swapped for a probe
// color in turn; the role whose swap changes the element's paint is the one. (T3 Code's theme
// editor finds roles the same way.)

import type { Role } from './themes'
import { ROLES } from './themes'

const PROBES = ['#01fea7', '#fe01a7']

type Paint = 'background' | 'border' | 'foreground'

function paint(el: Element, kind: Paint): string {
  const s = getComputedStyle(el)
  if (kind === 'background') return s.backgroundColor
  if (kind === 'border') return parseFloat(s.borderTopWidth) || parseFloat(s.borderBottomWidth) ? `${s.borderTopColor} ${s.borderBottomColor}` : ''
  // SVG icons paint with stroke and fill.
  return el instanceof SVGElement ? `${s.stroke} ${s.fill} ${s.color}` : s.color
}

const transparent = (color: string) => !color || color === 'transparent' || /rgba\([^)]*,\s*0\)$/.test(color)

/** The role whose property changes this paint, if one does. */
function roleFor(el: Element, kind: Paint): Role | null {
  const root = document.documentElement
  const before = paint(el, kind)
  if (!before) return null
  for (const role of ROLES) {
    const value = root.style.getPropertyValue(role.css)
    const priority = root.style.getPropertyPriority(role.css)
    let changed = false
    for (const probe of PROBES) {
      root.style.setProperty(role.css, probe)
      if (paint(el, kind) !== before) changed = true
    }
    if (value) root.style.setProperty(role.css, value, priority)
    else root.style.removeProperty(role.css)
    if (changed) return role.key
  }
  return null
}

/**
 * The role that paints what's at `el`: its background (or the nearest one behind it that has
 * one), its border, or its text, whichever a role sets first.
 */
export function inspectRole(el: Element): Role | null {
  let bg: Element | null = el
  while (bg && transparent(getComputedStyle(bg).backgroundColor)) bg = bg.parentElement
  const textFirst = el instanceof SVGElement || (el.childElementCount === 0 && !!el.textContent?.trim())
  // Text or an icon is what was clicked when it's there; otherwise the box behind it.
  const tries: [Element | null, Paint][] = textFirst
    ? [
        [el, 'foreground'],
        [bg, 'background'],
      ]
    : [
        [bg, 'background'],
        [el, 'border'],
      ]
  for (const [target, kind] of tries) {
    const role = target && roleFor(target, kind)
    if (role) return role
  }
  return null
}
