#!/usr/bin/env node
// Installs husmo's agent skill into OpenCode, registers husmo's MCP server in
// OpenCode's config, and fetches the husmo binary when the host does not
// already have one.
//
// The skill files shipped here are the same ones the Claude Code plugin uses:
// claude-plugin/skills/ is the single source of truth, packed verbatim into
// this package. The binary download mirrors install.sh and honours the same
// HUSMO_* environment hooks so it can be tested offline.
//
// The registration exists because husmo's tool surface is MCP: a skill on its
// own describes tools OpenCode never loaded. See
// docs/adr/0003-the-installer-writes-an-mcp-registration.md for why this
// installer edits opencode.json itself, why the command form varies by scope,
// and why the entry pins a timeout.

import { spawnSync } from "node:child_process"
import { createInterface } from "node:readline"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { createRequire } from "node:module"
import { applyEdits, modify, parse as parseJsonc } from "jsonc-parser"

const REPO = "frantufro/husmo"
const BIN_NAME = "husmo"
const SERVER_KEY = "husmo"
const PKG_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const SKILLS_SRC = path.join(PKG_ROOT, "claude-plugin", "skills")
const PKG_VERSION = createRequire(import.meta.url)("../package.json").version
const USER_AGENT = `husmo-install/${PKG_VERSION}`

const API_TIMEOUT_MS = 10_000
const DOWNLOAD_TIMEOUT_MS = 60_000

// OpenCode's own default is 30s. husmo's first save/search-semantic downloads
// ~90MB of embedding-model weights and runs first inference inside a single
// tool call, which overruns that on a slow connection.
const MCP_TIMEOUT_MS = 120_000

const OPENCODE_SCHEMA = "https://opencode.ai/config.json"

const USAGE = `husmo-install — install the husmo skill and MCP registration into OpenCode

usage:
  npx @frantufro/husmo install [options]

options:
  --project         install into ./.opencode/skills and the project's opencode.json
                    (default: the OpenCode user config directory)
  --force           overwrite an existing skill or registration without asking
  --skip-existing   keep what is already there and exit 0
  --no-binary       install the skill and registration only; never download the binary
  --dry-run         report every action without writing anything
  -h, --help        print this message

The husmo binary is downloaded only when \`${BIN_NAME}\` is absent from PATH.
A data repo is a separate step: run \`${BIN_NAME} init\` once to create one.`

function apiBase() {
  return process.env.HUSMO_GITHUB_API_BASE || "https://api.github.com"
}

function downloadBase() {
  return process.env.HUSMO_DOWNLOAD_BASE || "https://github.com"
}

function parseArgs(argv) {
  const flags = {
    command: null,
    project: false,
    force: false,
    skipExisting: false,
    binary: true,
    dryRun: false,
    help: false,
  }
  for (const arg of argv) {
    switch (arg) {
      case "install":
        flags.command = "install"
        break
      case "--project":
        flags.project = true
        break
      case "--force":
        flags.force = true
        break
      case "--skip-existing":
        flags.skipExisting = true
        break
      case "--no-binary":
        flags.binary = false
        break
      case "--dry-run":
        flags.dryRun = true
        break
      case "-h":
      case "--help":
        flags.help = true
        break
      default:
        return { error: `unknown argument: ${arg}` }
    }
  }
  if (flags.force && flags.skipExisting) {
    return { error: "--force and --skip-existing contradict each other" }
  }
  return { flags }
}

function tilde(target) {
  const home = os.homedir()
  return home && target.startsWith(home + path.sep) ? "~" + target.slice(home.length) : target
}

function report(mark, label, message) {
  process.stdout.write(`  ${mark} ${label.padEnd(7)} ${message}\n`)
}

function detail(message) {
  process.stdout.write(`    ${message}\n`)
}

function fail(message) {
  process.stderr.write(`  ✗ ${message}\n`)
}

function configHome() {
  const xdg = process.env.XDG_CONFIG_HOME
  return xdg && xdg.trim() !== "" ? xdg : path.join(os.homedir(), ".config")
}

// Stages beside the destination so the final step is a same-filesystem rename.
function writeFileAtomic(destFile, contents) {
  const destDir = path.dirname(destFile)
  fs.mkdirSync(destDir, { recursive: true })
  const stage = path.join(destDir, `.${path.basename(destFile)}.install.${process.pid}.tmp`)
  try {
    fs.writeFileSync(stage, contents)
    fs.renameSync(stage, destFile)
  } catch (err) {
    fs.rmSync(stage, { force: true })
    throw err
  }
}

function confirm(question) {
  return new Promise((resolve) => {
    const rl = createInterface({ input: process.stdin, output: process.stderr })
    rl.question(`  ? ${question} [y/N] `, (answer) => {
      rl.close()
      resolve(/^y(es)?$/i.test(answer.trim()))
    })
  })
}

// Shared conflict policy for the skill file and the registration: identical is
// a no-op, --skip-existing keeps, --force overwrites, a TTY asks, and a
// non-TTY run with neither flag refuses rather than rewriting unattended.
async function resolveConflict({ what, where, flags, describe }) {
  if (flags.skipExisting) return "keep"
  if (flags.force) return "overwrite"
  if (!process.stdin.isTTY) {
    fail(`${where} differs from what this installer would write`)
    if (describe) describe()
    process.stderr.write("    pass --force to overwrite, or --skip-existing to keep it\n")
    return "refuse"
  }
  if (describe) describe()
  return (await confirm(`Overwrite the ${what} in ${where}?`)) ? "overwrite" : "keep"
}

// ---------------------------------------------------------------- skill files

function skillsRoot(project) {
  if (project) return path.join(process.cwd(), ".opencode", "skills")
  return path.join(configHome(), "opencode", "skills")
}

function packagedSkills() {
  return fs
    .readdirSync(SKILLS_SRC, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .filter((name) => fs.existsSync(path.join(SKILLS_SRC, name, "SKILL.md")))
    .sort()
}

async function installSkill(name, root, flags) {
  const source = fs.readFileSync(path.join(SKILLS_SRC, name, "SKILL.md"))
  const destDir = path.join(root, name)
  const destFile = path.join(destDir, "SKILL.md")
  const shown = tilde(destDir)

  if (fs.existsSync(destFile)) {
    if (fs.readFileSync(destFile).equals(source)) {
      report("=", "skill", `up to date  ${shown}`)
      return true
    }
    const choice = await resolveConflict({
      what: "skill",
      where: tilde(destFile),
      flags,
    })
    if (choice === "refuse") return false
    if (choice === "keep") {
      report("→", "skill", `kept existing  ${shown}`)
      return true
    }
    if (flags.dryRun) {
      report("↑", "skill", `would update  ${shown}`)
      return true
    }
    writeFileAtomic(destFile, source)
    report("↑", "skill", `updated  ${shown}`)
    return true
  }

  if (flags.dryRun) {
    report("✓", "skill", `would install  ${shown}`)
    return true
  }
  writeFileAtomic(destFile, source)
  report("✓", "skill", `installed  ${shown}`)
  return true
}

// ------------------------------------------------------------- mcp registration

// Mirrors OpenCode's own resolution: the first candidate that exists wins, and
// a fresh install lands in the first candidate.
function opencodeConfigFile(project) {
  const candidates = project
    ? [
        path.join(process.cwd(), "opencode.json"),
        path.join(process.cwd(), "opencode.jsonc"),
        path.join(process.cwd(), ".opencode", "opencode.json"),
        path.join(process.cwd(), ".opencode", "opencode.jsonc"),
      ]
    : [
        path.join(configHome(), "opencode", "opencode.json"),
        path.join(configHome(), "opencode", "opencode.jsonc"),
      ]
  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) return candidate
  }
  return candidates[0]
}

// A project opencode.json is a file teams commit, so it names the binary
// through PATH. A global one is machine-specific, so it takes the absolute
// path — which also covers this installer's own ~/.local/bin download target
// being missing from PATH.
function buildRegistration(project, binaryPath) {
  const command = project || !binaryPath ? [BIN_NAME] : [binaryPath]
  return { type: "local", command, timeout: MCP_TIMEOUT_MS }
}

function sameRegistration(a, b) {
  if (!a || typeof a !== "object") return false
  return JSON.stringify(orderKeys(a)) === JSON.stringify(orderKeys(b))
}

function orderKeys(value) {
  if (Array.isArray(value)) return value.map(orderKeys)
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, orderKeys(value[key])]),
    )
  }
  return value
}

function renderRegistration(value) {
  return JSON.stringify(value, null, 2)
    .split("\n")
    .map((line) => `      ${line}`)
    .join("\n")
}

async function installRegistration(flags, binaryPath) {
  const file = opencodeConfigFile(flags.project)
  const shown = tilde(file)
  const desired = buildRegistration(flags.project, binaryPath)

  let text = `${JSON.stringify({ $schema: OPENCODE_SCHEMA }, null, 2)}\n`
  if (fs.existsSync(file)) {
    try {
      text = fs.readFileSync(file, "utf8")
    } catch (err) {
      fail(`could not read ${shown}: ${err.message}`)
      return false
    }
    const errors = []
    const parsed = parseJsonc(text, errors, { allowTrailingComma: true })
    if (errors.length > 0) {
      fail(`${shown} is not valid JSON/JSONC; leaving it untouched`)
      detail("add this to its \"mcp\" object by hand:")
      process.stderr.write(`${renderRegistration({ [SERVER_KEY]: desired })}\n`)
      return false
    }
    const existing = parsed && parsed.mcp ? parsed.mcp[SERVER_KEY] : undefined
    if (existing !== undefined) {
      if (sameRegistration(existing, desired)) {
        report("=", "mcp", `up to date  ${shown}`)
        return true
      }
      const choice = await resolveConflict({
        what: "husmo registration",
        where: shown,
        flags,
        describe: () => {
          detail("existing:")
          process.stderr.write(`${renderRegistration(existing)}\n`)
          detail("proposed:")
          process.stderr.write(`${renderRegistration(desired)}\n`)
        },
      })
      if (choice === "refuse") return false
      if (choice === "keep") {
        report("→", "mcp", `kept existing  ${shown}`)
        return true
      }
    }
  }

  if (flags.dryRun) {
    report("✓", "mcp", `would register husmo in  ${shown}`)
    process.stdout.write(`${renderRegistration(desired)}\n`)
    return true
  }

  // Surgical JSONC patch: comments and the rest of the file survive, and the
  // result matches what `opencode mcp add` would have written.
  const edits = modify(text, ["mcp", SERVER_KEY], desired, {
    formattingOptions: { tabSize: 2, insertSpaces: true },
  })
  try {
    writeFileAtomic(file, applyEdits(text, edits))
  } catch (err) {
    fail(`could not write ${shown}: ${err.message}`)
    return false
  }
  report("✓", "mcp", `registered husmo in  ${shown}`)

  if (flags.project && process.env.XDG_CONFIG_HOME) {
    detail("XDG_CONFIG_HOME is set and a committed registration cannot pin it.")
    detail("if husmo cannot find its config under OpenCode, add to this entry:")
    process.stdout.write(
      `${renderRegistration({ environment: { HUSMO_CONFIG: husmoConfigPath() } })}\n`,
    )
  }
  return true
}

// -------------------------------------------------------------- husmo binary

function targetFromParts(arch, platform) {
  const osName = { linux: "unknown-linux-gnu", darwin: "apple-darwin", macos: "apple-darwin" }[platform]
  if (!osName) throw new Error(`Unsupported OS: ${platform}`)
  const normalized = { x64: "x86_64", x86_64: "x86_64", arm64: "aarch64", aarch64: "aarch64" }[arch]
  if (!normalized) throw new Error(`Unsupported architecture: ${arch}`)
  if (normalized === "x86_64" && osName === "apple-darwin") {
    throw new Error("x86_64 macOS is not supported. Use an Apple Silicon Mac or build from source.")
  }
  return { triple: `${normalized}-${osName}` }
}

function detectTarget() {
  const override = process.env.HUSMO_TARGET_OVERRIDE
  if (override) {
    if (override.includes(":")) {
      const [arch, platform] = override.split(":")
      return targetFromParts(arch, platform)
    }
    return { triple: override }
  }
  return targetFromParts(process.arch, process.platform)
}

function findOnPath(name) {
  for (const dir of (process.env.PATH || "").split(path.delimiter).filter(Boolean)) {
    const candidate = path.join(dir, name)
    try {
      if (fs.statSync(candidate).isFile()) {
        fs.accessSync(candidate, fs.constants.X_OK)
        return candidate
      }
    } catch {}
  }
  return null
}

function versionOf(binary) {
  const probe = spawnSync(binary, ["--version"], { encoding: "utf8" })
  const match = /(\d+\.\d+\.\d+)/.exec(`${probe.stdout || ""}${probe.stderr || ""}`)
  return match ? match[1] : null
}

function isNewer(latest, current) {
  const parse = (value) => {
    const match = /^v?(\d+)\.(\d+)\.(\d+)/.exec(value || "")
    return match ? match.slice(1, 4).map(Number) : null
  }
  const a = parse(latest)
  const b = parse(current)
  if (!a || !b) return false
  for (let i = 0; i < 3; i++) {
    if (a[i] !== b[i]) return a[i] > b[i]
  }
  return false
}

// husmo has no `update` subcommand, so the upgrade hint has to name whichever
// installer put the binary where it is.
function upgradeHint(binaryPath) {
  if (/(^|\/)(Cellar|homebrew|linuxbrew)\//.test(binaryPath)) {
    return [`brew upgrade ${BIN_NAME}`]
  }
  if (binaryPath.includes(path.join(".cargo", "bin"))) {
    return ["cargo install --path . --force  (from a checkout)"]
  }
  return [`curl -sSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sh`]
}

async function latestVersion() {
  const url = `${apiBase()}/repos/${REPO}/releases/latest`
  const res = await fetch(url, {
    headers: { accept: "application/vnd.github+json", "user-agent": USER_AGENT },
    signal: AbortSignal.timeout(API_TIMEOUT_MS),
  })
  if (!res.ok) throw new Error(`GitHub API returned HTTP ${res.status}`)
  const body = await res.json()
  const tag = body && body.tag_name
  if (typeof tag !== "string") throw new Error("missing tag_name in GitHub API response")
  return tag.replace(/^v/, "")
}

function binaryDir() {
  const asRoot = typeof process.getuid === "function" && process.getuid() === 0
  return asRoot ? "/usr/local/bin" : path.join(os.homedir(), ".local", "bin")
}

async function downloadBinary(version, target, destDir) {
  const url = `${downloadBase()}/${REPO}/releases/download/v${version}/${BIN_NAME}-${target.triple}.tar.gz`
  const res = await fetch(url, {
    redirect: "follow",
    headers: { "user-agent": USER_AGENT },
    signal: AbortSignal.timeout(DOWNLOAD_TIMEOUT_MS),
  })
  if (!res.ok) throw new Error(`download failed: HTTP ${res.status} for ${url}`)

  const work = fs.mkdtempSync(path.join(os.tmpdir(), "husmo-install-"))
  try {
    const tarball = path.join(work, `${BIN_NAME}.tar.gz`)
    fs.writeFileSync(tarball, Buffer.from(await res.arrayBuffer()))
    const tar = spawnSync("tar", ["-xzf", tarball, "-C", work, BIN_NAME], { encoding: "utf8" })
    if (tar.error) throw new Error(`could not run tar: ${tar.error.message}`)
    if (tar.status !== 0) throw new Error(`tar failed: ${(tar.stderr || "").trim()}`)

    fs.mkdirSync(destDir, { recursive: true })
    const stage = path.join(destDir, `.${BIN_NAME}.install.${process.pid}.tmp`)
    const dest = path.join(destDir, BIN_NAME)
    try {
      fs.copyFileSync(path.join(work, BIN_NAME), stage)
      fs.chmodSync(stage, 0o755)
      fs.renameSync(stage, dest)
    } catch (err) {
      fs.rmSync(stage, { force: true })
      throw err
    }
    return dest
  } finally {
    fs.rmSync(work, { recursive: true, force: true })
  }
}

function warnIfNotOnPath(dir) {
  const entries = (process.env.PATH || "").split(path.delimiter)
  if (entries.includes(dir)) return
  process.stdout.write(`\n  note: ${tilde(dir)} is not in your PATH. Add this to your shell config:\n`)
  process.stdout.write(`        export PATH="${tilde(dir)}:$PATH"\n`)
}

// Returns the absolute path of the binary the registration should name, or
// null when there is none to name.
async function ensureBinary(flags) {
  const existing = findOnPath(BIN_NAME)
  if (existing) {
    const current = versionOf(existing)
    report("=", "binary", `${BIN_NAME} ${current || "(unknown version)"} at ${tilde(existing)}`)
    try {
      const latest = await latestVersion()
      if (isNewer(latest, current)) {
        detail(`${latest} is available — upgrade with:`)
        for (const line of upgradeHint(existing)) detail(`  ${line}`)
      }
    } catch {
      // A version check is a courtesy; a network failure here changes nothing.
    }
    return { path: existing, ok: true }
  }

  if (!flags.binary) {
    report("→", "binary", `${BIN_NAME} is absent from PATH and --no-binary was given`)
    return { path: null, ok: true }
  }

  let target
  try {
    target = detectTarget()
  } catch (err) {
    fail(err.message)
    return { path: null, ok: false }
  }

  if (flags.dryRun) {
    report("↓", "binary", `would download ${BIN_NAME} for ${target.triple} into ${tilde(binaryDir())}`)
    return { path: path.join(binaryDir(), BIN_NAME), ok: true }
  }

  try {
    const version = await latestVersion()
    report("↓", "binary", `downloading ${BIN_NAME} ${version} for ${target.triple}…`)
    const dir = binaryDir()
    const dest = await downloadBinary(version, target, dir)
    report("✓", "binary", tilde(dest))
    warnIfNotOnPath(dir)
    return { path: dest, ok: true }
  } catch (err) {
    fail(`could not install the ${BIN_NAME} binary: ${err.message}`)
    detail("install it directly instead:")
    detail(`  brew install frantufro/tap/${BIN_NAME}`)
    detail(`  curl -sSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sh`)
    return { path: null, ok: false }
  }
}

// ----------------------------------------------------------------- data repo

// Mirrors src/config.rs resolve_path: HUSMO_CONFIG wins outright, then
// XDG_CONFIG_HOME, then HOME.
function husmoConfigPath() {
  const explicit = process.env.HUSMO_CONFIG
  if (explicit) return explicit
  const xdg = process.env.XDG_CONFIG_HOME
  if (xdg) return path.join(xdg, "husmo", "config.toml")
  return path.join(os.homedir(), ".config", "husmo", "config.toml")
}

// One field out of a small TOML file, without a TOML dependency on top of
// jsonc-parser. Returns null when the line is absent or shaped unexpectedly.
function readDataRepoPath(contents) {
  const basic = /^[ \t]*data_repo_path[ \t]*=[ \t]*"((?:[^"\\]|\\.)*)"/m.exec(contents)
  if (basic) {
    return basic[1].replace(/\\(["\\nt])/g, (_, ch) => ({ n: "\n", t: "\t" })[ch] || ch)
  }
  const literal = /^[ \t]*data_repo_path[ \t]*=[ \t]*'([^']*)'/m.exec(contents)
  return literal ? literal[1] : null
}

function checkDataRepo() {
  const configPath = husmoConfigPath()
  if (!fs.existsSync(configPath)) {
    report("→", "data", `no config at ${tilde(configPath)}`)
    detail("create a data repo before using husmo's tools:")
    detail("  cd wherever/you/want/the/data/repo")
    detail(`  ${BIN_NAME} init --repo git@github.com:you/husmo-data.git`)
    return
  }

  let contents
  try {
    contents = fs.readFileSync(configPath, "utf8")
  } catch (err) {
    report("→", "data", `could not read ${tilde(configPath)}: ${err.message}`)
    return
  }

  const dataRepo = readDataRepoPath(contents)
  if (!dataRepo) {
    report("→", "data", `${tilde(configPath)} has no readable data_repo_path`)
    detail(`re-run \`${BIN_NAME} init\`, or set data_repo_path by hand`)
    return
  }

  if (!fs.existsSync(path.join(dataRepo, ".git"))) {
    report("→", "data", `${tilde(dataRepo)} is not a git repo`)
    detail(`the clone moved or was removed — re-run \`${BIN_NAME} init\`, or`)
    detail(`point data_repo_path in ${tilde(configPath)} at the right place`)
    return
  }

  report("=", "data", `${tilde(dataRepo)}`)
}

// --------------------------------------------------------------------- entry

async function main() {
  const { flags, error } = parseArgs(process.argv.slice(2))
  if (error) {
    process.stderr.write(`${error}\n\n${USAGE}\n`)
    return 1
  }
  if (flags.help || flags.command !== "install") {
    process.stdout.write(`${USAGE}\n`)
    return flags.help || process.argv.length <= 2 ? 0 : 1
  }

  if (flags.dryRun) process.stdout.write("  (dry run — nothing will be written)\n")

  let ok = true
  const root = skillsRoot(flags.project)
  for (const name of packagedSkills()) {
    if (!(await installSkill(name, root, flags))) ok = false
  }

  // The binary resolves first: a global registration names its absolute path.
  const binary = await ensureBinary(flags)
  if (!binary.ok) ok = false
  if (!(await installRegistration(flags, binary.path))) ok = false

  checkDataRepo()

  if (ok) {
    process.stdout.write("\n  restart OpenCode to pick up the skill and the husmo tools.\n")
  }
  return ok ? 0 : 1
}

main().then(
  (code) => process.exit(code),
  (err) => {
    fail(err && err.stack ? err.stack : String(err))
    process.exit(1)
  },
)
