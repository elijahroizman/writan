#!/usr/bin/env node
// Tests scripts/nightly.mjs in scratch git repositories: which stable release a nightly's version
// follows, and where its list of changes starts. CI runs it (.github/workflows/ci.yml).
//
//   node scripts/check-nightly.mjs

import assert from 'node:assert/strict'
import { execFileSync, spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./nightly.mjs', import.meta.url))

// The scratch repositories ignore the user's and the system's git config (signing, hooks).
const env = {
  ...process.env,
  GIT_CONFIG_GLOBAL: '/dev/null',
  GIT_CONFIG_NOSYSTEM: '1',
  GIT_AUTHOR_NAME: 'Test',
  GIT_AUTHOR_EMAIL: 'test@example.com',
  GIT_COMMITTER_NAME: 'Test',
  GIT_COMMITTER_EMAIL: 'test@example.com',
}
delete env.STABLE_RELEASES
delete env.GITHUB_ACTIONS
delete env.NOTES

let failures = 0
function test(name, fn) {
  const dir = mkdtempSync(join(tmpdir(), 'writan-nightly-'))
  const git = (...args) => execFileSync('git', args, { cwd: dir, env, encoding: 'utf8' }).trim()
  const commit = (subject) => git('commit', '--allow-empty', '-q', '-m', subject)
  const run = (args, extra = {}) => {
    const r = spawnSync(process.execPath, [script, ...args], { cwd: dir, env: { ...env, ...extra }, encoding: 'utf8' })
    return { status: r.status, stdout: r.stdout, stderr: r.stderr }
  }
  try {
    git('init', '-q', '-b', 'main')
    fn({ git, commit, run })
    console.log(`ok - ${name}`)
  } catch (err) {
    failures++
    console.log(`not ok - ${name}\n${err.stack ?? err}`)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}

test('the version follows the latest published stable release, not an unpublished tag', ({ git, commit, run }) => {
  commit('feat: first')
  git('tag', 'v1.0.0')
  commit('feat: second')
  // v1.1.0 is tagged, but its release build failed: it's still a draft.
  git('tag', 'v1.1.0')
  const r = run(['version'], { STABLE_RELEASES: 'v0.19.0\nv1.0.0\nnightly\napt' })
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /^1\.1\.0-nightly\.\d{12}\n$/)
  // A dry run reads the tags, which include the unpublished one.
  const dry = run(['version'])
  assert.equal(dry.status, 0, dry.stderr)
  assert.match(dry.stdout, /^1\.2\.0-nightly\.\d{12}\n$/)
  assert.match(dry.stderr, /Dry run/)
})

test('no published stable release is an error', ({ git, commit, run }) => {
  commit('feat: first')
  git('tag', 'v1.0.0')
  for (const published of ['', 'nightly\nv1.1.0-nightly-2026-11-27-0930']) {
    const r = run(['names'], { STABLE_RELEASES: published })
    assert.equal(r.status, 1)
    assert.equal(r.stdout, '')
    assert.match(r.stderr, /No published stable release/)
  }
  // In GitHub Actions the published releases have to be given.
  const ci = run(['version'], { GITHUB_ACTIONS: 'true' })
  assert.equal(ci.status, 1)
  assert.match(ci.stderr, /Set STABLE_RELEASES/)
})

test("a hotfix tagged after a nightly stays in the next nightly's notes", ({ git, commit, run }) => {
  commit('feat: first')
  git('tag', 'v1.0.0')
  commit('feat(editor): before the nightly')
  git('tag', 'v1.1.0-nightly-2026-11-26-0930')
  commit('feat: in the last nightly')
  git('tag', 'v1.1.0-nightly-2026-11-27-0930')
  commit('fix(updates): the hotfix')
  git('tag', 'v1.0.1')
  commit('feat: after the hotfix')
  const r = run(['notes'], { STABLE_RELEASES: 'v1.0.0 v1.0.1', GITHUB_REPOSITORY: 'owner/repo' })
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /- \*\*updates:\*\* the hotfix/)
  assert.match(r.stdout, /- after the hotfix/)
  assert.doesNotMatch(r.stdout, /in the last nightly|before the nightly|first/)
  assert.match(r.stdout, /Changes since \[v1\.1\.0-nightly-2026-11-27-0930\]\(https:\/\/github\.com\/owner\/repo\/releases\/tag\/v1\.1\.0-nightly-2026-11-27-0930\)\./)
})

test("the first nightly of a minor version starts at the stable release, not the last minor's nightlies", ({ git, commit, run }) => {
  commit('feat: first')
  git('tag', 'v1.0.0')
  commit('feat: in a 1.1.0 nightly')
  git('tag', 'v1.1.0-nightly-2026-11-27-0930')
  // The nightly tagged the old way, before nightlies were the next minor version.
  git('tag', 'v1.0.1-nightly.202611270930')
  commit('feat: in 1.1.0')
  git('tag', 'v1.1.0')
  commit('fix: after 1.1.0')
  const r = run(['notes'], { STABLE_RELEASES: 'v1.0.0\nv1.1.0' })
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /### Fixes\n\n- after 1\.1\.0\n/)
  assert.doesNotMatch(r.stdout, /in 1\.1\.0|in a 1\.1\.0 nightly/)
  assert.match(r.stdout, /Changes since \[v1\.1\.0\]/)
})

test('a nightly on another branch is not a baseline', ({ git, commit, run }) => {
  commit('feat: first')
  git('tag', 'v1.0.0')
  git('checkout', '-q', '-b', 'side')
  commit('feat: only on the side branch')
  git('tag', 'v1.1.0-nightly-2026-11-27-0930')
  git('checkout', '-q', 'main')
  commit('feat: on main')
  const r = run(['notes'], { STABLE_RELEASES: 'v1.0.0' })
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /- on main/)
  assert.doesNotMatch(r.stdout, /side branch/)
  assert.match(r.stdout, /Changes since \[v1\.0\.0\]/)
})

test('without a tag for the stable release every commit is listed', ({ commit, run }) => {
  commit('feat: first')
  commit('fix: second')
  const r = run(['notes'], { STABLE_RELEASES: 'v1.0.0' })
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /- first/)
  assert.match(r.stdout, /- second/)
  assert.doesNotMatch(r.stdout, /Changes since/)
})

if (failures) {
  console.log(`${failures} failed`)
  process.exit(1)
}
