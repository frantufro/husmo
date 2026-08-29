#!/usr/bin/env node
// Prepares a release commit.
//
// husmo's version lives in four places that must agree: Cargo.toml, the husmo
// entry in Cargo.lock, package.json (the npm installer), and the Claude Code
// plugin manifest. The release workflow asserts three of them against the git
// tag and fails the publish if any has drifted, so this script moves them
// together, refreshes the lockfile, runs both test suites, and leaves a
// committed release branch for review.
//
//   node scripts/release.mjs 0.2.0
//   node scripts/release.mjs patch --dry-run
//
// Tagging stays a separate, deliberate step; this script never pushes.

import { spawnSync } from "node:child_process"
import fs from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const CARGO_TOML = path.join(ROOT, "Cargo.toml")
const CARGO_LOCK = path.join(ROOT, "Cargo.lock")
const PACKAGE_JSON = path.join(ROOT, "package.json")
const PLUGIN_JSON = path.join(ROOT, "claude-plugin", ".claude-plugin", "plugin.json")

const BUMPS = ["major", "minor", "patch"]

let dryRun = false
let request = null

for (const arg of process.argv.slice(2)) {
  if (arg === "--dry-run") dryRun = true
  else if (arg === "-h" || arg === "--help") usage(0)
  else if (arg.startsWith("-")) die(`unknown argument: ${arg}`)
  else if (request) die(`unexpected extra argument: ${arg}`)
  else request = arg
}

if (!request) usage(1)

function usage(code) {
  const out = code === 0 ? process.stdout : process.stderr
  out.write(`usage: node scripts/release.mjs <version|major|minor|patch> [--dry-run]

Bumps Cargo.toml, Cargo.lock, package.json and the plugin manifest to the same
version, runs cargo test and npm test, and commits the result on a release
branch. Push the branch, merge it, then tag to publish.
`)
  process.exit(code)
}

function die(message) {
  process.stderr.write(`error: ${message}\n`)
  process.exit(1)
}

function step(message) {
  process.stdout.write(`\n\x1b[1m${message}\x1b[0m\n`)
}

function note(message) {
  process.stdout.write(`  ${message}\n`)
}

function run(command, args, { capture = false } = {}) {
  const result = spawnSync(command, args, {
    cwd: ROOT,
    encoding: "utf8",
    stdio: capture ? "pipe" : "inherit",
  })
  if (result.error) die(`${command} could not be run: ${result.error.message}`)
  if (result.status !== 0) {
    if (capture) process.stderr.write(result.stderr || "")
    die(`${command} ${args.join(" ")} exited with ${result.status}`)
  }
  return capture ? result.stdout.trim() : ""
}

// --- versions ---------------------------------------------------------------

// Scans for the version line inside [package], so a dependency pinned to an
// exact version elsewhere in the file cannot be mistaken for husmo's own.
function currentVersion() {
  let seenPackage = false
  for (const line of fs.readFileSync(CARGO_TOML, "utf8").split("\n")) {
    if (/^\[package\]\s*$/.test(line)) {
      seenPackage = true
      continue
    }
    if (seenPackage && /^\[/.test(line)) break
    const match = seenPackage && /^version\s*=\s*"([^"]+)"/.exec(line)
    if (match) return match[1]
  }
  die("could not read the [package] version from Cargo.toml")
  return ""
}

function nextVersion(current, requested) {
  if (!BUMPS.includes(requested)) {
    if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(requested)) {
      die(`"${requested}" is neither a semver version nor one of ${BUMPS.join(", ")}`)
    }
    return requested
  }
  const [major, minor, patch] = current.split(/[.-]/).slice(0, 3).map(Number)
  if ([major, minor, patch].some(Number.isNaN)) {
    die(`cannot bump "${current}" automatically; pass an explicit version`)
  }
  if (requested === "major") return `${major + 1}.0.0`
  if (requested === "minor") return `${major}.${minor + 1}.0`
  return `${major}.${minor}.${patch + 1}`
}

function isAscending(from, to) {
  const parse = (v) => v.split(/[.-]/).slice(0, 3).map(Number)
  const a = parse(to)
  const b = parse(from)
  for (let i = 0; i < 3; i++) {
    if (a[i] !== b[i]) return a[i] > b[i]
  }
  return false
}

// --- edits ------------------------------------------------------------------

// Rewrites only the version under [package], leaving every dependency
// version in the file alone.
function bumpCargoToml(version) {
  const toml = fs.readFileSync(CARGO_TOML, "utf8")
  let seenPackage = false
  let done = false
  const lines = toml.split("\n").map((line) => {
    if (/^\[package\]\s*$/.test(line)) {
      seenPackage = true
      return line
    }
    if (seenPackage && /^\[/.test(line)) seenPackage = false
    if (seenPackage && !done && /^version\s*=\s*"/.test(line)) {
      done = true
      return `version = "${version}"`
    }
    return line
  })
  if (!done) die("could not find a [package] version line in Cargo.toml")
  return { file: CARGO_TOML, contents: lines.join("\n") }
}

function bumpJson(file, version) {
  const text = fs.readFileSync(file, "utf8")
  if (!/^(\s*)"version":\s*"[^"]*"/m.test(text)) {
    die(`could not find a top-level "version" in ${path.relative(ROOT, file)}`)
  }
  return {
    file,
    contents: text.replace(/^(\s*)"version":\s*"[^"]*"/m, `$1"version": "${version}"`),
  }
}

function lockedVersion() {
  const lock = fs.readFileSync(CARGO_LOCK, "utf8")
  const match = /\nname = "husmo"\nversion = "([^"]+)"/.exec(lock)
  return match ? match[1] : null
}

// --- main -------------------------------------------------------------------

const current = currentVersion()
const version = nextVersion(current, request)
const tag = `v${version}`
const branch = `release/${tag}`

step(`husmo ${current} → ${version}${dryRun ? "  (dry run)" : ""}`)

if (version === current) die(`Cargo.toml already reads ${version}`)
if (!isAscending(current, version)) {
  note(`warning: ${version} sorts below the current ${current}`)
}

const status = run("git", ["status", "--porcelain"], { capture: true })
if (status) {
  die(`the working tree has uncommitted changes:\n${status}\n\ncommit or stash them first`)
}
if (run("git", ["rev-parse", "--abbrev-ref", "HEAD"], { capture: true }) !== "main") {
  note("warning: releases are normally cut from main")
}
if (run("git", ["tag", "--list", tag], { capture: true })) die(`tag ${tag} already exists`)
if (run("git", ["branch", "--list", branch], { capture: true })) {
  die(`branch ${branch} already exists`)
}

const edits = [
  bumpCargoToml(version),
  bumpJson(PACKAGE_JSON, version),
  bumpJson(PLUGIN_JSON, version),
]

step("Version sites")
for (const edit of edits) note(`${path.relative(ROOT, edit.file)} → ${version}`)
note(`${path.relative(ROOT, CARGO_LOCK)} → ${version} (refreshed by cargo check)`)

if (dryRun) {
  step("Dry run")
  note("no files written, no tests run, no branch created")
  note(`a real run would commit ${branch} and stop before tagging`)
  process.exit(0)
}

for (const edit of edits) fs.writeFileSync(edit.file, edit.contents)

step("cargo check  (refreshes Cargo.lock)")
run("cargo", ["check", "--quiet"])
const locked = lockedVersion()
if (locked !== version) die(`Cargo.lock reads ${locked ?? "no husmo entry"} after cargo check`)

step("cargo test")
run("cargo", ["test", "--quiet"])

step("npm test")
run("npm", ["test"])

step(`git branch ${branch}`)
run("git", ["checkout", "-b", branch])
run("git", ["add", "Cargo.toml", "Cargo.lock", "package.json", path.relative(ROOT, PLUGIN_JSON)])
run("git", ["commit", "-m", `Release ${tag}`])

step("Next")
note(`git push -u origin ${branch}`)
note("open a PR and squash-merge it")
note(`git checkout main && git pull && git tag ${tag} && git push origin ${tag}`)
note("")
note("the tag runs build → release → homebrew → npm; watch it with `gh run watch`")
note("the frantufro/claude-plugins marketplace pins husmo's version; bump it there too")
