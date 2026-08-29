# Link Database

A local, git-backed store of content an AI agent can save and retrieve, used
to bring links and documents into an agent's context.

## Language

### Documents

**Document**:
A saved unit of content, optionally sourced from a URL.
_Avoid_: Entry, Link, Item

**Canonical URL**:
The URL a Document was fetched from, when it has one. Pasted or typed content
has no Canonical URL.
_Avoid_: Source, Link (when used as a noun for the field)

**Tag**:
A free-form label attached to a Document for organizing and filtering.

**Related**:
A deliberate, symmetric, untyped connection between two Documents, declared
explicitly rather than discovered by extraction.

**Outgoing Link**:
A hyperlink found in a Document's content pointing at another page, which may
optionally be archived as its own Document.

**Data Repo**:
The git repository holding every Document, living outside this repo at a path
named by husmo's own config file.
_Avoid_: store, database, vault, library

### Distribution

**Skill**:
A directory containing a `SKILL.md` whose YAML frontmatter carries a `name`
and a `description`, which a Host loads to teach a model a capability.
_Avoid_: prompt, instructions, rule, agent

**Host**:
A program that discovers Skills and connects to MCP Servers. husmo targets
OpenCode and Claude Code.
_Avoid_: client, editor, tool, IDE

**Binary**:
The compiled `husmo` executable.
_Avoid_: CLI, tool, program

**MCP Server**:
The process a Binary becomes when run with no subcommand, exposing husmo's
tools to a Host over stdio.
_Avoid_: daemon, service, backend

**Registration**:
The entry a Host's config carries under `mcp`, naming the Binary that Host
spawns to reach husmo's tools.
_Avoid_: entry, wiring, hookup, MCP config

**Scope**:
Which pair of locations an installation writes to. `global` is the Host's user
config directory; `project` is a directory inside the current repository.
_Avoid_: level, location, target

**Installer**:
`bin/install.mjs`, published to npm as `@frantufro/husmo`, which places a
Skill, writes a Registration, and fetches the Binary when one is missing.
_Avoid_: plugin, bootstrapper, setup script

## Relationships

- A **Document** has at most one **Canonical URL**.
- A **Document** has a title, zero or more **Tags**, a saved-at timestamp, an
  optional summary, and an optional author.
- A **Canonical URL** identifies at most one **Document**; re-saving the same
  **Canonical URL** overwrites that **Document**'s content rather than
  creating another.
- A **Document** may be **Related** to any number of other Documents.
  Retrieving a Document always lists what it's **Related** to; the content of
  those Documents is only pulled in when explicitly requested.
- A **Document**'s content may contain **Outgoing Links**; archiving one turns
  it into its own Document, and this is a distinct concept from being
  **Related** — an **Outgoing Link** is discovered, **Related** is declared.
- Every **Document** lives in the one **Data Repo** a **Registration**'s
  **MCP Server** was configured to open.
- A **Host** loads a **Skill** and spawns an **MCP Server** through a
  **Registration**; the **Installer** delivers all three.
- The **Installer** writes one **Scope** per run, into one **Host**.

## Example dialogue

> **Dev:** "I installed the Skill with `--project` and husmo's tools still
> don't show up."
>
> **Maintainer:** "A **Skill** and a **Registration** are separate things. The
> **Skill** tells the model what to do with **Outgoing Links**; the
> **Registration** is what makes the **Host** spawn the **MCP Server** at all.
> Check whether your `opencode.json` has one."
>
> **Dev:** "It does, and the server starts, but every `save` fails."
>
> **Maintainer:** "Then the **Registration** is fine and the **Data Repo**
> isn't. The **Registration** names the **Binary**; husmo's own config file
> names the **Data Repo**. `husmo init` writes that one."

## Flagged ambiguities

- "link" was used informally to mean a saved web page — resolved: a saved link
  is just a **Document** whose **Canonical URL** is set.
- "relate documents" could have meant the same thing as archiving an
  **Outgoing Link** — resolved: these are distinct. **Related** is a
  deliberate edge between any two Documents; an **Outgoing Link** is a
  hyperlink discovered in a Document's content.
- "config" names three things — resolved: husmo's own config file names the
  **Data Repo**; a **Host**'s config file holds the **Registration**; the
  **Registration** is one entry inside it.
- "husmo" names five things — resolved: this project, the **Binary**, the
  **MCP Server** that Binary runs as, the npm package publishing the
  **Installer**, and the key a **Registration** appears under. The npm
  package's executable is `husmo-install`, which keeps `husmo` on `PATH`
  meaning the **Binary**.
- "install" covers four operations — resolved: the **Installer** places a
  **Skill**, writes a **Registration**, and at most once fetches a missing
  **Binary**. Bootstrapping a **Data Repo** is `husmo init`, a separate step
  the **Installer** only reports on.
