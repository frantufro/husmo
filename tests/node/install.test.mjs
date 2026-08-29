// Offline tests for bin/install.mjs.
//
// Every test runs the installer as a subprocess against a throwaway HOME and
// XDG_CONFIG_HOME, with PATH scrubbed down to node plus the system binaries so
// a real husmo on the developer's machine cannot influence a run. The GitHub
// API and the release download are served by a local http server through the
// HUSMO_GITHUB_API_BASE and HUSMO_DOWNLOAD_BASE hooks, so no test touches the
// network.

import assert from "node:assert/strict"
import { after, before, beforeEach, describe, it } from "node:test"
import { spawn, spawnSync } from "node:child_process"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..")
const INSTALLER = path.join(REPO_ROOT, "bin", "install.mjs")
const SKILL_NAME = "husmo-document-management"
const PACKAGED_SKILL = fs.readFileSync(
  path.join(REPO_ROOT, "claude-plugin", "skills", SKILL_NAME, "SKILL.md"),
  "utf8",
)

const SYSTEM_PATH = [path.dirname(process.execPath), "/usr/bin", "/bin"].join(path.delimiter)

let sandbox

beforeEach(() => {
  sandbox = fs.mkdtempSync(path.join(os.tmpdir(), "husmo-install-test-"))
})

function dir(...parts) {
  const target = path.join(sandbox, ...parts)
  fs.mkdirSync(target, { recursive: true })
  return target
}

function run(args, { env = {}, cwd = sandbox } = {}) {
  const home = path.join(sandbox, "home")
  fs.mkdirSync(home, { recursive: true })
  const result = spawnSync(process.execPath, [INSTALLER, ...args], {
    cwd,
    encoding: "utf8",
    env: {
      HOME: home,
      XDG_CONFIG_HOME: path.join(sandbox, "xdg"),
      PATH: SYSTEM_PATH,
      ...env,
    },
  })
  return { ...result, output: `${result.stdout}${result.stderr}` }
}

function globalConfigPath() {
  return path.join(sandbox, "xdg", "opencode", "opencode.json")
}

function readRegistration(file) {
  const raw = fs.readFileSync(file, "utf8").replace(/^\s*\/\/.*$/gm, "")
  return JSON.parse(raw).mcp.husmo
}

// A stand-in husmo on PATH, so a global registration has an absolute path to
// name and the upgrade check has a version to compare.
function fakeBinaryOnPath(version = "0.1.0") {
  const binDir = dir("fakebin")
  const binary = path.join(binDir, "husmo")
  fs.writeFileSync(binary, `#!/bin/sh\necho "husmo ${version}"\n`)
  fs.chmodSync(binary, 0o755)
  return { binDir, binary, PATH: `${binDir}${path.delimiter}${SYSTEM_PATH}` }
}

describe("argument handling", () => {
  it("prints usage and exits 0 for --help", () => {
    const result = run(["--help"])
    assert.equal(result.status, 0)
    assert.match(result.stdout, /husmo-install/)
  })

  it("rejects an unknown argument", () => {
    const result = run(["install", "--nope"])
    assert.equal(result.status, 1)
    assert.match(result.stderr, /unknown argument: --nope/)
  })

  it("rejects --force together with --skip-existing", () => {
    const result = run(["install", "--force", "--skip-existing"])
    assert.equal(result.status, 1)
    assert.match(result.stderr, /contradict/)
  })
})

describe("skill installation", () => {
  it("installs the packaged skill into the global skills directory", () => {
    const result = run(["install", "--no-binary"])
    assert.equal(result.status, 0)
    const installed = path.join(sandbox, "xdg", "opencode", "skills", SKILL_NAME, "SKILL.md")
    assert.equal(fs.readFileSync(installed, "utf8"), PACKAGED_SKILL)
  })

  it("reports an unchanged skill as up to date", () => {
    run(["install", "--no-binary"])
    const result = run(["install", "--no-binary"])
    assert.equal(result.status, 0)
    assert.match(result.stdout, /skill\s+up to date/)
  })

  it("refuses a differing skill on a non-TTY without a flag", () => {
    run(["install", "--no-binary"])
    const installed = path.join(sandbox, "xdg", "opencode", "skills", SKILL_NAME, "SKILL.md")
    fs.writeFileSync(installed, "locally edited\n")
    const result = run(["install", "--no-binary"])
    assert.equal(result.status, 1)
    assert.equal(fs.readFileSync(installed, "utf8"), "locally edited\n")
  })

  it("overwrites a differing skill with --force", () => {
    run(["install", "--no-binary"])
    const installed = path.join(sandbox, "xdg", "opencode", "skills", SKILL_NAME, "SKILL.md")
    fs.writeFileSync(installed, "locally edited\n")
    const result = run(["install", "--no-binary", "--force"])
    assert.equal(result.status, 0)
    assert.equal(fs.readFileSync(installed, "utf8"), PACKAGED_SKILL)
  })
})

describe("mcp registration", () => {
  it("creates opencode.json when none exists", () => {
    const result = run(["install", "--no-binary"])
    assert.equal(result.status, 0)
    const config = JSON.parse(fs.readFileSync(globalConfigPath(), "utf8"))
    assert.equal(config.$schema, "https://opencode.ai/config.json")
    assert.deepEqual(config.mcp.husmo, {
      type: "local",
      command: ["husmo"],
      timeout: 120000,
    })
  })

  it("preserves comments and unrelated keys in an existing config", () => {
    const file = globalConfigPath()
    fs.mkdirSync(path.dirname(file), { recursive: true })
    fs.writeFileSync(
      file,
      '{\n  // keep me\n  "model": "lmstudio/devstral",\n  "provider": { "x": { "apiKey": "sk-secret" } }\n}\n',
    )
    const result = run(["install", "--no-binary"])
    assert.equal(result.status, 0)
    const text = fs.readFileSync(file, "utf8")
    assert.match(text, /\/\/ keep me/)
    assert.match(text, /sk-secret/)
    assert.match(text, /"model": "lmstudio\/devstral"/)
    assert.deepEqual(readRegistration(file).command, ["husmo"])
  })

  it("leaves a byte-identical file on a second run", () => {
    run(["install", "--no-binary"])
    const before = fs.readFileSync(globalConfigPath(), "utf8")
    const result = run(["install", "--no-binary"])
    assert.equal(result.status, 0)
    assert.match(result.stdout, /mcp\s+up to date/)
    assert.equal(fs.readFileSync(globalConfigPath(), "utf8"), before)
  })

  it("names the binary by absolute path in global scope", () => {
    const fake = fakeBinaryOnPath()
    const result = run(["install"], { env: { PATH: fake.PATH } })
    assert.equal(result.status, 0)
    assert.deepEqual(readRegistration(globalConfigPath()).command, [fake.binary])
  })

  it("names the binary through PATH in project scope", () => {
    const fake = fakeBinaryOnPath()
    const project = dir("project")
    const result = run(["install", "--project"], { env: { PATH: fake.PATH }, cwd: project })
    assert.equal(result.status, 0)
    const file = path.join(project, "opencode.json")
    assert.deepEqual(readRegistration(file).command, ["husmo"])
    assert.ok(fs.existsSync(path.join(project, ".opencode", "skills", SKILL_NAME, "SKILL.md")))
  })

  it("writes into an existing opencode.jsonc rather than creating a .json beside it", () => {
    const project = dir("project")
    const jsonc = path.join(project, "opencode.jsonc")
    fs.writeFileSync(jsonc, '{\n  // jsonc\n  "model": "x"\n}\n')
    const result = run(["install", "--no-binary", "--project"], { cwd: project })
    assert.equal(result.status, 0)
    assert.ok(fs.existsSync(jsonc))
    assert.equal(fs.existsSync(path.join(project, "opencode.json")), false)
    assert.deepEqual(readRegistration(jsonc).command, ["husmo"])
  })

  it("refuses a differing registration on a non-TTY and leaves the file untouched", () => {
    run(["install", "--no-binary"])
    const file = globalConfigPath()
    const edited = fs.readFileSync(file, "utf8").replace("120000", "9000")
    fs.writeFileSync(file, edited)
    const result = run(["install", "--no-binary"])
    assert.equal(result.status, 1)
    assert.equal(fs.readFileSync(file, "utf8"), edited)
    assert.match(result.stderr, /--force/)
  })

  it("keeps a differing registration with --skip-existing and exits 0", () => {
    run(["install", "--no-binary"])
    const file = globalConfigPath()
    const edited = fs.readFileSync(file, "utf8").replace("120000", "9000")
    fs.writeFileSync(file, edited)
    const result = run(["install", "--no-binary", "--skip-existing"])
    assert.equal(result.status, 0)
    assert.equal(fs.readFileSync(file, "utf8"), edited)
  })

  it("replaces a differing registration with --force", () => {
    run(["install", "--no-binary"])
    const file = globalConfigPath()
    fs.writeFileSync(file, fs.readFileSync(file, "utf8").replace("120000", "9000"))
    const result = run(["install", "--no-binary", "--force"])
    assert.equal(result.status, 0)
    assert.equal(readRegistration(file).timeout, 120000)
  })

  it("refuses to touch a config it cannot parse", () => {
    const file = globalConfigPath()
    fs.mkdirSync(path.dirname(file), { recursive: true })
    fs.writeFileSync(file, "{ this is not json at all ]\n")
    const result = run(["install", "--no-binary"])
    assert.equal(result.status, 1)
    assert.equal(fs.readFileSync(file, "utf8"), "{ this is not json at all ]\n")
    assert.match(result.stderr, /not valid JSON/)
  })
})

describe("--dry-run", () => {
  it("writes nothing at all", () => {
    const result = run(["install", "--no-binary", "--dry-run"])
    assert.equal(result.status, 0)
    assert.match(result.stdout, /dry run/)
    assert.equal(fs.existsSync(globalConfigPath()), false)
    assert.equal(fs.existsSync(path.join(sandbox, "xdg", "opencode", "skills")), false)
  })
})

describe("data repo readiness", () => {
  it("reports a missing config file and points at husmo init", () => {
    const result = run(["install", "--no-binary"])
    assert.match(result.stdout, /no config at/)
    assert.match(result.stdout, /husmo init --repo/)
  })

  it("flags a data_repo_path that is not a git repo", () => {
    const configFile = path.join(dir("xdg", "husmo"), "config.toml")
    const dataRepo = dir("data")
    fs.writeFileSync(configFile, `data_repo_path = "${dataRepo}"\n`)
    const result = run(["install", "--no-binary"])
    assert.match(result.stdout, /is not a git repo/)
  })

  it("accepts a data_repo_path that is a git repo", () => {
    const configFile = path.join(dir("xdg", "husmo"), "config.toml")
    const dataRepo = dir("data")
    fs.mkdirSync(path.join(dataRepo, ".git"))
    fs.writeFileSync(configFile, `data_repo_path = "${dataRepo}"\nallowed_source_dirs = ["/tmp"]\n`)
    const result = run(["install", "--no-binary"])
    assert.match(result.stdout, /=\s+data\s+/)
    assert.doesNotMatch(result.stdout, /is not a git repo/)
  })

  it("flags a config with no readable data_repo_path", () => {
    const configFile = path.join(dir("xdg", "husmo"), "config.toml")
    fs.writeFileSync(configFile, "# nothing useful here\n")
    const result = run(["install", "--no-binary"])
    assert.match(result.stdout, /no readable data_repo_path/)
  })

  it("honours HUSMO_CONFIG over XDG_CONFIG_HOME", () => {
    const explicit = path.join(dir("elsewhere"), "husmo.toml")
    const dataRepo = dir("data")
    fs.mkdirSync(path.join(dataRepo, ".git"))
    fs.writeFileSync(explicit, `data_repo_path = "${dataRepo}"\n`)
    const result = run(["install", "--no-binary"], { env: { HUSMO_CONFIG: explicit } })
    assert.match(result.stdout, new RegExp(`=\\s+data\\s+${dataRepo}`))
  })
})

describe("binary download", () => {
  let server
  let base

  before(async () => {
    // A real gzipped tar holding an executable named husmo, built once with
    // the system tar so the installer's own `tar -xzf` can read it back.
    const work = fs.mkdtempSync(path.join(os.tmpdir(), "husmo-tarball-"))
    fs.writeFileSync(path.join(work, "husmo"), "#!/bin/sh\necho 'husmo 9.9.9'\n")
    fs.chmodSync(path.join(work, "husmo"), 0o755)
    const packed = spawnSync("tar", ["-czf", path.join(work, "husmo.tar.gz"), "-C", work, "husmo"])
    assert.equal(packed.status, 0)
    const tarball = path.join(work, "husmo.tar.gz")

    // The fixture server lives in its own process. These tests drive the
    // installer through spawnSync, which blocks this process's event loop for
    // the whole run, so a server hosted here could never answer the child it
    // is waiting on.
    server = spawn(process.execPath, ["--input-type=module", "-e", SERVER_SOURCE], {
      stdio: ["ignore", "pipe", "inherit"],
      env: { ...process.env, HUSMO_FIXTURE_TARBALL: tarball },
    })
    base = await new Promise((resolve, reject) => {
      let seen = ""
      server.stdout.setEncoding("utf8")
      server.stdout.on("data", (chunk) => {
        seen += chunk
        const match = /PORT (\d+)/.exec(seen)
        if (match) resolve(`http://127.0.0.1:${match[1]}`)
      })
      server.once("exit", (code) => reject(new Error(`fixture server exited with ${code}`)))
    })
  })

  after(() => {
    server.kill()
  })

  function downloadEnv(extra = {}) {
    return {
      HUSMO_GITHUB_API_BASE: base,
      HUSMO_DOWNLOAD_BASE: base,
      HUSMO_TARGET_OVERRIDE: "aarch64-apple-darwin",
      ...extra,
    }
  }

  it("downloads the binary when husmo is absent from PATH", () => {
    const result = run(["install"], { env: downloadEnv() })
    assert.equal(result.status, 0)
    const installed = path.join(sandbox, "home", ".local", "bin", "husmo")
    assert.ok(fs.existsSync(installed))
    assert.ok(fs.statSync(installed).mode & 0o111)
    assert.deepEqual(readRegistration(globalConfigPath()).command, [installed])
  })

  it("leaves an existing binary alone and names its upgrade command", () => {
    const fake = fakeBinaryOnPath("0.0.1")
    const result = run(["install"], { env: downloadEnv({ PATH: fake.PATH }) })
    assert.equal(result.status, 0)
    assert.match(result.stdout, /9\.9\.9 is available/)
    assert.match(result.stdout, /install\.sh/)
    assert.equal(fs.existsSync(path.join(sandbox, "home", ".local", "bin", "husmo")), false)
  })

  it("stays quiet about upgrades when the binary is current", () => {
    const fake = fakeBinaryOnPath("9.9.9")
    const result = run(["install"], { env: downloadEnv({ PATH: fake.PATH }) })
    assert.equal(result.status, 0)
    assert.doesNotMatch(result.stdout, /is available/)
  })

  it("rejects x86_64 macOS with a build-from-source hint", () => {
    const result = run(["install"], { env: downloadEnv({ HUSMO_TARGET_OVERRIDE: "x64:darwin" }) })
    assert.equal(result.status, 1)
    assert.match(result.stderr, /x86_64 macOS is not supported/)
  })

  it("registers husmo through PATH when the download fails", () => {
    const result = run(["install"], {
      env: downloadEnv({ HUSMO_DOWNLOAD_BASE: "http://127.0.0.1:1" }),
    })
    assert.equal(result.status, 1)
    assert.match(result.stderr, /could not install the husmo binary/)
    assert.deepEqual(readRegistration(globalConfigPath()).command, ["husmo"])
  })
})

// Runs in its own node process (see the binary-download suite). It serves the
// tarball named by HUSMO_FIXTURE_TARBALL and announces its port on stdout.
const SERVER_SOURCE = `
import http from "node:http"
import fs from "node:fs"
const tarball = fs.readFileSync(process.env.HUSMO_FIXTURE_TARBALL)
const server = http.createServer((req, res) => {
  if (req.url.endsWith("/releases/latest")) {
    res.writeHead(200, { "content-type": "application/json" })
    res.end(JSON.stringify({ tag_name: "v9.9.9" }))
    return
  }
  if (req.url.endsWith(".tar.gz")) {
    res.writeHead(200, { "content-type": "application/gzip" })
    res.end(tarball)
    return
  }
  res.writeHead(404)
  res.end()
})
server.listen(0, "127.0.0.1", () => console.log("PORT " + server.address().port))
`
