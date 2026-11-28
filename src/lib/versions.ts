// Version order and display. Releases are SemVer: 0.19.0, 1.0.0. A nightly is the next patch
// release with a pre-release part holding its UTC build time, 1.0.1-nightly.202611280930, so it
// sorts after the stable release it follows and before the next one (docs/internals/releasing.md).

/** Orders versions the way SemVer does: negative, zero or positive as `a` is older, equal or newer. */
export function compareVersions(a: string, b: string): number {
  const split = (v: string) => {
    const core = v.split('+')[0]
    const dash = core.indexOf('-')
    const release = (dash < 0 ? core : core.slice(0, dash)).split('.').map((n) => parseInt(n, 10) || 0)
    return { release, pre: dash < 0 ? null : core.slice(dash + 1).split('.') }
  }
  const pa = split(a)
  const pb = split(b)
  for (let i = 0; i < Math.max(pa.release.length, pb.release.length); i++) {
    const x = pa.release[i] ?? 0
    const y = pb.release[i] ?? 0
    if (x !== y) return x > y ? 1 : -1
  }
  // A pre-release comes before its release.
  if (!pa.pre || !pb.pre) return pa.pre ? -1 : pb.pre ? 1 : 0
  for (let i = 0; i < Math.max(pa.pre.length, pb.pre.length); i++) {
    const x = pa.pre[i]
    const y = pb.pre[i]
    if (x === undefined) return -1
    if (y === undefined) return 1
    const nx = /^\d+$/.test(x)
    const ny = /^\d+$/.test(y)
    if (nx && ny) {
      // Too long for a double: compare the digits.
      const dx = x.replace(/^0+(?=\d)/, '')
      const dy = y.replace(/^0+(?=\d)/, '')
      if (dx.length !== dy.length) return dx.length > dy.length ? 1 : -1
      if (dx !== dy) return dx > dy ? 1 : -1
    } else if (nx !== ny) {
      // Numeric identifiers sort before alphanumeric ones.
      return nx ? -1 : 1
    } else if (x !== y) {
      return x > y ? 1 : -1
    }
  }
  return 0
}

/** Whether version `a` is newer than `b`. */
export function newer(a: string, b: string): boolean {
  return compareVersions(a, b) > 0
}

const NIGHTLY = /^\d+\.\d+\.\d+-nightly\.(\d{4})(\d{2})(\d{2})(\d{2})(\d{2})$/

/** When a nightly was built, or null for any other version. */
export function nightlyDate(version: string): Date | null {
  const m = NIGHTLY.exec(version)
  if (!m) return null
  const [, y, mo, d, h, mi] = m.map(Number)
  const date = new Date(Date.UTC(y, mo - 1, d, h, mi))
  // Date.UTC rolls month 13 over into the next year; a stamp like that isn't a nightly.
  return date.getUTCMonth() === mo - 1 && date.getUTCDate() === d && h < 24 && mi < 60 ? date : null
}

export const isNightlyVersion = (version: string) => nightlyDate(version) !== null

/** A version for people to read: 0.19.0 as it is, a nightly as "Nightly, 28 Nov 2026 09:30" in local time. */
export function displayVersion(version: string): string {
  const date = nightlyDate(version)
  if (!date) return version
  const month = date.toLocaleDateString(undefined, { month: 'short' })
  const pad = (n: number) => String(n).padStart(2, '0')
  return `Nightly, ${date.getDate()} ${month} ${date.getFullYear()} ${pad(date.getHours())}:${pad(date.getMinutes())}`
}
