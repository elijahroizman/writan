#!/usr/bin/env node
// Version and release notes for a nightly build (.github/workflows/nightly.yml), and SemVer order
// for the Release workflow's nightly feed. Run from the repository root with the tags fetched.
//
//   node scripts/nightly.mjs version          1.1.0-nightly.202611280930: the next minor release
//                                             after the latest published stable release, with
//                                             the UTC date and time.
//   node scripts/nightly.mjs names [version]  The names a nightly goes by, as key=value lines for
//                                             $GITHUB_OUTPUT: version, tag, label (the tag without
//                                             its v, which the file names use) and title. Without
//                                             a version, for a nightly built now.
//   node scripts/nightly.mjs tag-version <tag>
//                                             The version a release tag stands for: v1.0.0 is
//                                             1.0.0, v1.1.0-nightly-2026-11-28-0930 is
//                                             1.1.0-nightly.202611280930.
//   node scripts/nightly.mjs notes            The pre-release's notes, in Markdown: the PR titles
//                                             merged since the previous nightly of the same minor
//                                             version, or else the latest published stable
//                                             release, grouped by type, after the NOTES
//                                             environment variable.
//   node scripts/nightly.mjs newer <a> <b>    Exits 0 when version a is newer than b.
//
// The latest published stable release comes from STABLE_RELEASES: the tags of the GitHub
// releases that are public and not pre-releases, which the workflow reads with gh. A stable tag
// whose release never published doesn't count, so a nightly built while v1.1.0's build is failing
// stays a 1.1.0 nightly, and 1.1.0 still replaces it in the nightly feed once it's out. Without
// STABLE_RELEASES, outside GitHub Actions, `version`, `names` and `notes` use the git tags
// instead, for a dry run.
//
// A nightly is a pre-release of the next minor version: after 1.0.0, and after a 1.0.1 hotfix
// too, it's 1.1.0-nightly.<stamp>. It sorts after the stable release it follows and before the
// next minor one, and nightlies sort by their build time. The stamp is a single number, since
// SemVer forbids leading zeros in a numeric identifier (0930 alone would be invalid).
//
// That SemVer version is what the updaters compare, and it goes into tauri.conf.json, latest.json
// and the packages. People see one name built from it, everywhere else:
//
//   tag         v1.1.0-nightly-2026-11-28-0930   (a tag can't hold a space or a colon)
//   title       Writan Nightly v1.1.0 2026-11-28 09:30 UTC
//   in the app  Nightly v1.1.0 2026-11-28 09:30 UTC   (displayVersion in src/lib/versions.ts)
//   files       Writan_1.1.0-nightly-2026-11-28-0930_x64.dmg
//
// The tag maps one to one back to the version (tagVersion here, version_from_tag in
// src-tauri/src/update.rs, versionFromTag in src/lib/versions.ts). scripts/check-nightly.mjs
// tests this script.

import { execFileSync } from 'node:child_process'

const git = (...args) => execFileSync('git', args, { encoding: 'utf8' }).trim()

/** SemVer order: negative, zero or positive as `a` is older, equal or newer. */
export function compareVersions(a, b) {
  const split = (v) => {
    const core = v.split('+')[0]
    const dash = core.indexOf('-')
    const release = (dash < 0 ? core : core.slice(0, dash)).split('.').map((n) => BigInt(/^\d+$/.test(n) ? n : 0))
    return { release, pre: dash < 0 ? null : core.slice(dash + 1).split('.') }
  }
  const pa = split(a)
  const pb = split(b)
  for (let i = 0; i < Math.max(pa.release.length, pb.release.length); i++) {
    const x = pa.release[i] ?? 0n
    const y = pb.release[i] ?? 0n
    if (x !== y) return x > y ? 1 : -1
  }
  if (!pa.pre || !pb.pre) return pa.pre ? -1 : pb.pre ? 1 : 0
  for (let i = 0; i < Math.max(pa.pre.length, pb.pre.length); i++) {
    const x = pa.pre[i]
    const y = pb.pre[i]
    if (x === undefined) return -1
    if (y === undefined) return 1
    const nx = /^\d+$/.test(x)
    const ny = /^\d+$/.test(y)
    if (nx && ny) {
      if (BigInt(x) !== BigInt(y)) return BigInt(x) > BigInt(y) ? 1 : -1
    } else if (nx !== ny) {
      return nx ? -1 : 1
    } else if (x !== y) {
      return x > y ? 1 : -1
    }
  }
  return 0
}

const STABLE_TAG = /^v(\d+)\.(\d+)\.(\d+)$/

/** The highest stable version among `tags`, or undefined. */
const highestStable = (tags) =>
  tags
    .filter((tag) => STABLE_TAG.test(tag))
    .map((tag) => tag.slice(1))
    .sort(compareVersions)
    .at(-1)

/** The latest published stable release, which the next nightly is the next minor version after. */
function latestStable() {
  const published = process.env.STABLE_RELEASES
  if (published !== undefined) {
    const latest = highestStable(published.split(/\s+/))
    if (!latest) throw new Error('No published stable release: STABLE_RELEASES has no vX.Y.Z tag.')
    return latest
  }
  if (process.env.GITHUB_ACTIONS === 'true') {
    throw new Error("Set STABLE_RELEASES to the published stable releases' tags (gh api repos/<owner>/<repo>/releases).")
  }
  const latest = highestStable(git('tag', '--list', 'v*').split('\n'))
  if (!latest) throw new Error('No stable vX.Y.Z tag. Fetch the tags (git fetch --tags).')
  console.error(`Dry run: using the highest stable git tag, v${latest}, which may not be published. STABLE_RELEASES sets the published ones.`)
  return latest
}

/** The nightly built at `now` after the stable release `stable`: the next minor version's pre-release. */
export function nightlyVersion(stable, now = new Date()) {
  const [major, minor] = stable.split('.').map(Number)
  const pad = (n) => String(n).padStart(2, '0')
  const stamp = `${now.getUTCFullYear()}${pad(now.getUTCMonth() + 1)}${pad(now.getUTCDate())}${pad(now.getUTCHours())}${pad(now.getUTCMinutes())}`
  return `${major}.${minor + 1}.0-nightly.${stamp}`
}

const NIGHTLY_VERSION = /^(\d+\.\d+\.\d+)-nightly\.(\d{4})(\d{2})(\d{2})(\d{2})(\d{2})$/
const NIGHTLY_TAG = /^v(\d+\.\d+\.\d+)-nightly-(\d{4})-(\d{2})-(\d{2})-(\d{2})(\d{2})$/

/** The names a nightly version goes by. Throws for anything that isn't a nightly version. */
export function nightlyNames(version) {
  const m = NIGHTLY_VERSION.exec(version)
  if (!m) throw new Error(`not a nightly version: ${version}`)
  const [, base, y, mo, d, h, mi] = m
  const label = `${base}-nightly-${y}-${mo}-${d}-${h}${mi}`
  return { version, tag: `v${label}`, label, title: `Writan Nightly v${base} ${y}-${mo}-${d} ${h}:${mi} UTC` }
}

/** The version a release tag stands for: the tag without its v, with a nightly's date put back into one stamp. */
export function tagVersion(tag) {
  const m = NIGHTLY_TAG.exec(tag)
  if (m) return `${m[1]}-nightly.${m.slice(2).join('')}`
  return /^v\d/.test(tag) ? tag.slice(1) : tag
}

// PR titles follow Conventional Commits (.github/workflows/pr-checks.yml), and squash merges make
// them the commit subjects.
const TYPES = [
  ['feat', 'Features'],
  ['fix', 'Fixes'],
  ['perf', 'Performance'],
  ['revert', 'Reverts'],
  ['refactor', 'Refactoring'],
  ['docs', 'Documentation'],
  ['test', 'Tests'],
  ['build', 'Build'],
  ['ci', 'CI'],
  ['style', 'Style'],
  ['chore', 'Chores'],
]
const TITLE = /^(\w+)(?:\(([a-z0-9][a-z0-9-]*)\))?(!)?: (.+)$/

/** Commit subjects grouped under a heading for each type, in the order of TYPES, then the rest. */
export function groupTitles(subjects) {
  const groups = new Map(TYPES.map(([, heading]) => [heading, []]))
  groups.set('Other changes', [])
  const headings = new Map(TYPES)
  for (const subject of subjects) {
    const m = TITLE.exec(subject)
    const heading = m && headings.get(m[1])
    if (!heading) {
      groups.get('Other changes').push(`- ${subject}`)
      continue
    }
    const [, , scope, breaking, summary] = m
    groups.get(heading).push(`- ${breaking ? '**Breaking:** ' : ''}${scope ? `**${scope}:** ` : ''}${summary}`)
  }
  return [...groups].filter(([, lines]) => lines.length).map(([heading, lines]) => `### ${heading}\n\n${lines.join('\n')}`)
}

/**
 * Where a nightly's list of changes starts: the newest earlier nightly of the same minor version
 * that this commit has, or else the stable release `stable` it follows. Not simply the newest tag
 * this commit has: a v1.0.1 hotfix tagged on main after a 1.1.0 nightly would be that, and the
 * hotfix's changes would be left out of the next 1.1.0 nightly, whose readers never see 1.0.1's
 * notes, as it's older than their nightly. A failed nightly stays a draft, and a draft has no
 * tag, so its changes show up in the next one.
 */
function notesBaseline(stable) {
  const series = nightlyVersion(stable).split('-')[0]
  const nightlies = git('tag', '--merged', 'HEAD', '--list', `v${series}-nightly-*`)
    .split('\n')
    .filter((tag) => NIGHTLY_TAG.test(tag))
    .sort((a, b) => compareVersions(tagVersion(a), tagVersion(b)))
  if (nightlies.length) return nightlies.at(-1)
  try {
    git('rev-parse', '--quiet', '--verify', `refs/tags/v${stable}`)
    return `v${stable}`
  } catch {
    // No tag for it here: every commit is new.
    return ''
  }
}

function notes() {
  const since = notesBaseline(latestStable())
  const range = since ? [`${since}..HEAD`] : ['HEAD']
  const subjects = git('log', '--no-merges', '--format=%s', ...range)
    .split('\n')
    .filter(Boolean)
  const sha = git('rev-parse', 'HEAD')
  const repo = process.env.GITHUB_REPOSITORY || 'elijahroizman/writan'
  const out = []
  out.push(`A nightly build of \`main\` at [\`${sha.slice(0, 7)}\`](https://github.com/${repo}/commit/${sha}). Nightlies get new features as soon as they land and may break things. Pick the channel in Settings › General › Updates.`)
  const extra = (process.env.NOTES || '').trim()
  if (extra) out.push(extra)
  const groups = groupTitles(subjects)
  if (groups.length) out.push(...groups)
  else out.push('No changes since the previous build.')
  if (since) out.push(`Changes since [${since}](https://github.com/${repo}/releases/tag/${since}).`)
  return out.join('\n\n') + '\n'
}

const [command, ...args] = process.argv.slice(2)
try {
  if (command === 'version') {
    process.stdout.write(nightlyVersion(latestStable()) + '\n')
  } else if (command === 'names' && args.length <= 1) {
    const names = nightlyNames(args[0] ?? nightlyVersion(latestStable()))
    process.stdout.write(Object.entries(names).map(([key, value]) => `${key}=${value}\n`).join(''))
  } else if (command === 'tag-version' && args.length === 1) {
    process.stdout.write(tagVersion(args[0]) + '\n')
  } else if (command === 'notes') {
    process.stdout.write(notes())
  } else if (command === 'newer' && args.length === 2) {
    process.exit(compareVersions(args[0], args[1]) > 0 ? 0 : 1)
  } else if (command !== undefined) {
    console.error('usage: node scripts/nightly.mjs version | names [version] | tag-version <tag> | notes | newer <a> <b>')
    process.exit(2)
  }
} catch (err) {
  console.error(err instanceof Error ? err.message : String(err))
  process.exit(1)
}
