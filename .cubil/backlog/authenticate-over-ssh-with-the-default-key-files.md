---
created: 2026-11-12
---

# Authenticate over SSH with the default key files

husmo authenticates SSH remotes only through a running ssh-agent (`git2::Cred::ssh_key_from_agent` in `remote_callbacks`, `src/git_sync.rs`), then falls back to a git credential helper and `Cred::default()`. A machine with an SSH key in `~/.ssh` and no agent cannot clone, pull or push, and `husmo init` fails with "remote rejected authentication: Failed getting response".

This blocks agent-provisioning (task `husmo-on-every-agent`). Every Hermes Agent runs in a Lima VM with its own passphrase-less key at `~/.ssh/id_ed25519`, registered on its Forgejo user, and no ssh-agent. husmo runs there as an MCP server that Hermes spawns, so there is no login shell to start an agent either. On 2026-11-12, `husmo init --repo ssh://git@git.apps.tufro.me:2222/frantufro/husmo-data.git` on Pincho passed the host key check and then failed authentication. `git clone` of the same URL works, because it reads `~/.ssh/config` and the key.

## What to build

1. In the credentials callback, after the ssh-agent attempt fails and the remote allows `SSH_KEY`, try `Cred::ssh_key(username, None, key_path, None)` for each of `~/.ssh/id_ed25519`, `~/.ssh/id_ecdsa` and `~/.ssh/id_rsa` that exists, in that order. Keys with a passphrase fail on the first try and the loop moves on to the next candidate.
2. Return an error that names the keys tried when none works, so a failed clone says what to do. The current message names only the libgit2 failure.
3. Test with a local SSH server or document why the existing tests cannot cover it. The agent-first path, the credential helper and `file://` remotes must keep working.
4. Note in `docs/ARCHITECTURE.md` ("Git mechanics") and the module comment of `remote_callbacks` which credentials husmo tries and in what order.

## Related limits to document

libgit2 does not read `~/.ssh/config`, so a `Port` or `IdentityFile` entry has no effect on husmo. A remote on a non-standard port needs an `ssh://` URL with the port, for example `ssh://git@git.apps.tufro.me:2222/frantufro/husmo-data.git`. The short form `git@host:owner/repo.git` connects to port 22.

## Done when

On a machine with a key file, no ssh-agent and an `ssh://` remote, `husmo init --repo <url>` clones and `husmo` saves push to the branch. A release ships it, and agent-provisioning's `25-tools.sh` picks it up on the next `update`, after which `husmo-on-every-agent` continues with the `ssh://` URL.
