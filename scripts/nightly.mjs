#!/usr/bin/env node
// Version and release notes for a nightly build (.github/workflows/nightly.yml), and SemVer order
// for the Release workflow's nightly feed. Run from the repository root with the tags fetched.
//
//   node scripts/nightly.mjs version          1.0.1-nightly.202611280930: the latest stable tag's
//                                             next patch release, with the UTC date and time.
//   node scripts/nightly.mjs notes            The pre-release's notes, in Markdown: the PR titles
//                                             merged since the previous stable or nightly tag,
//                                             grouped by type, after the NOTES environment variable.
//   node scripts/nightly.mjs newer <a> <b>    Exits 0 when version a is newer than b.
//
// A nightly sorts after the stable release it follows and before the next one, and nightlies
// sort by their build time. The stamp is a single number, since SemVer forbids leading zeros in
// a numeric identifier (0930 alone would be invalid).

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

function latestStable() {
  const versions = git('tag', '--list', 'v*')
    .split('\n')
    .filter((tag) => STABLE_TAG.test(tag))
    .map((tag) => tag.slice(1))
    .sort(compareVersions)
  const latest = versions.at(-1)
  if (!latest) throw new Error('No stable vX.Y.Z tag. Fetch the tags (fetch-depth: 0).')
  return latest
}

export function nightlyVersion(stable, now = new Date()) {
  const [major, minor, patch] = stable.split('.').map(Number)
  const pad = (n) => String(n).padStart(2, '0')
  const stamp = `${now.getUTCFullYear()}${pad(now.getUTCMonth() + 1)}${pad(now.getUTCDate())}${pad(now.getUTCHours())}${pad(now.getUTCMinutes())}`
  return `${major}.${minor}.${patch + 1}-nightly.${stamp}`
}

// PR titles follow Conventional Commits (.github/workflows/pr-title.yml), and squash merges make
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

function notes() {
  // The newest stable or nightly tag this commit has. A failed nightly stays a draft, and a draft
  // has no tag, so its changes show up in the next one.
  let since = ''
  try {
    since = git('describe', '--tags', '--abbrev=0', '--match', 'v[0-9]*', 'HEAD')
  } catch {
    // No tag yet: every commit is new.
  }
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
if (command === 'version') {
  process.stdout.write(nightlyVersion(latestStable()) + '\n')
} else if (command === 'notes') {
  process.stdout.write(notes())
} else if (command === 'newer' && args.length === 2) {
  process.exit(compareVersions(args[0], args[1]) > 0 ? 0 : 1)
} else if (command !== undefined) {
  console.error('usage: node scripts/nightly.mjs version | notes | newer <a> <b>')
  process.exit(2)
}
