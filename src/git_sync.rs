//! The git pull -> write -> commit -> push wrapper around the data repo,
//! per `docs/ARCHITECTURE.md` ("Git mechanics"). Every mutating operation
//! (`save`, `delete`, `relate`, `unrelate`) runs through [`sync_write`]
//! instead of committing manually. Single machine for now: the pull before
//! write is a forward-looking safety habit, not a full multi-machine
//! conflict-resolution system — a diverged history is reported as an error
//! rather than merged.

use std::path::{Path, PathBuf};

use git2::Repository;

/// The name of the remote every data repo is expected to have configured.
const REMOTE_NAME: &str = "origin";

/// An error encountered while running the pull -> write -> commit -> push
/// cycle.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SyncError<E> {
    /// The repository at the given path could not be opened.
    #[error("failed to open git repository at {}", path.display())]
    Open {
        /// The path that was opened.
        path: PathBuf,
        /// The underlying git failure.
        #[source]
        source: git2::Error,
    },
    /// Pulling from the remote before the write failed.
    #[error("failed to pull from remote before write")]
    Pull(#[source] git2::Error),
    /// The write closure itself returned an error. No commit or push is
    /// attempted in this case.
    #[error("write failed")]
    Write(#[source] E),
    /// Committing the write failed.
    #[error("failed to commit write")]
    Commit(#[source] git2::Error),
    /// Pushing the commit to the remote failed.
    #[error("failed to push to remote after commit")]
    Push(#[source] git2::Error),
}

/// Runs `write` wrapped in the git pull -> write -> commit -> push cycle
/// described in `docs/ARCHITECTURE.md` ("Git mechanics"): pulls from
/// `origin`, calls `write`, stages and commits every change in the working
/// tree with `message`, and pushes the result to `origin`.
///
/// If `write` returns an error, no commit or push is attempted — the repo
/// is left exactly as the pull left it.
///
/// # Errors
///
/// Returns [`SyncError::Open`] if `repo_dir` isn't a git repository,
/// [`SyncError::Pull`] if the pull fails (including a diverged history that
/// can't be fast-forwarded), [`SyncError::Write`] if `write` fails,
/// [`SyncError::Commit`] if the commit fails, or [`SyncError::Push`] if the
/// push fails.
pub fn sync_write<T, E>(
    repo_dir: &Path,
    message: &str,
    write: impl FnOnce() -> Result<T, E>,
) -> Result<T, SyncError<E>> {
    let repo = Repository::open(repo_dir).map_err(|source| SyncError::Open {
        path: repo_dir.to_path_buf(),
        source,
    })?;

    pull(&repo).map_err(SyncError::Pull)?;

    let result = write().map_err(SyncError::Write)?;

    if let CommitOutcome::Committed = commit_all(&repo, message).map_err(SyncError::Commit)? {
        push(&repo).map_err(SyncError::Push)?;
    }

    Ok(result)
}

/// Whether [`commit_all`] produced a new commit or found nothing to commit.
enum CommitOutcome {
    /// A new commit was created on top of the previous HEAD.
    Committed,
    /// The working tree matched the previous HEAD's tree exactly, so no
    /// commit was made.
    NoChanges,
}

/// Builds the remote callbacks used for fetch and push: an SSH key from the
/// running ssh-agent for `git+ssh` remotes, falling back to the system
/// credential helper configured in `repo`'s git config (covers HTTPS
/// remotes with a stored token/password) and finally to whatever default
/// credential git2 can produce. Remotes that need no authentication at all
/// (e.g. a local `file://` path, as in the tests) never invoke this.
fn remote_callbacks(repo: &Repository) -> git2::RemoteCallbacks<'_> {
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(|url, username_from_url, allowed_types| {
        if allowed_types.contains(git2::CredentialType::SSH_KEY)
            && let Some(username) = username_from_url
            && let Ok(credential) = git2::Cred::ssh_key_from_agent(username)
        {
            return Ok(credential);
        }
        if let Ok(config) = repo.config()
            && let Ok(credential) = git2::Cred::credential_helper(&config, url, username_from_url)
        {
            return Ok(credential);
        }
        git2::Cred::default()
    });
    callbacks
}

/// Fetches from `origin` and fast-forwards the current branch to match, if
/// it isn't already up to date. Returns an error if the local branch has
/// commits the remote doesn't (a diverged history) — reconciling that is
/// out of scope for now (see the module docs).
fn pull(repo: &Repository) -> Result<(), git2::Error> {
    let mut remote = repo.find_remote(REMOTE_NAME)?;
    let mut fetch_options = git2::FetchOptions::new();
    fetch_options.remote_callbacks(remote_callbacks(repo));
    remote.fetch::<&str>(&[], Some(&mut fetch_options), None)?;

    let fetch_head = repo.find_reference("FETCH_HEAD")?;
    let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;

    let (analysis, _preference) = repo.merge_analysis(&[&fetch_commit])?;
    if analysis.is_up_to_date() {
        return Ok(());
    }
    if !analysis.is_fast_forward() {
        return Err(git2::Error::from_str(
            "local and remote history have diverged; a fast-forward pull isn't possible",
        ));
    }

    let mut head_ref = repo.head()?;
    let refname = head_ref.name()?.to_string();
    head_ref.set_target(fetch_commit.id(), "fast-forward pull")?;
    repo.set_head(&refname)?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
    Ok(())
}

/// Stages every change in the working tree and commits it on top of HEAD
/// with `message`, using a fixed `husmo` author/committer identity (the
/// data repo's commits are made on the user's behalf by the tool, not
/// authored interactively). Makes no commit, and returns
/// [`CommitOutcome::NoChanges`], if the staged tree is identical to HEAD's
/// (e.g. `write` left the working tree exactly as it found it) — there is
/// nothing meaningful to commit or push in that case.
fn commit_all(repo: &Repository, message: &str) -> Result<CommitOutcome, git2::Error> {
    let mut index = repo.index()?;
    index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
    index.write()?;
    let tree_id = index.write_tree()?;
    let parent = repo.head()?.peel_to_commit()?;
    if tree_id == parent.tree_id() {
        return Ok(CommitOutcome::NoChanges);
    }
    let tree = repo.find_tree(tree_id)?;

    let signature = git2::Signature::now("husmo", "husmo@localhost")?;
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &[&parent],
    )?;
    Ok(CommitOutcome::Committed)
}

/// Pushes the current branch to `origin`.
fn push(repo: &Repository) -> Result<(), git2::Error> {
    let mut remote = repo.find_remote(REMOTE_NAME)?;
    let mut push_options = git2::PushOptions::new();
    push_options.remote_callbacks(remote_callbacks(repo));
    let head = repo.head()?;
    let refname = head.name()?;
    remote.push(&[format!("{refname}:{refname}")], Some(&mut push_options))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use git2::Repository;
    use tempfile::TempDir;

    /// Creates a bare "remote" repo with one seeded commit (a file named
    /// `seed.txt`) on its default branch, so a clone of it has a HEAD to
    /// work from.
    fn seeded_bare_remote() -> (TempDir, std::path::PathBuf) {
        let remote_dir = tempfile::tempdir().expect("failed to create temp dir");
        let remote_path = remote_dir.path().join("remote.git");
        Repository::init_bare(&remote_path).expect("failed to init bare remote");

        // Seed it via a throwaway working clone, since a fresh bare repo has
        // no branches to clone from yet.
        let seed_dir = tempfile::tempdir().expect("failed to create temp dir");
        let seed_repo = Repository::init(seed_dir.path()).expect("failed to init seed repo");
        std::fs::write(seed_dir.path().join("seed.txt"), "seed\n").expect("failed to write seed");
        commit_all(&seed_repo, "seed");
        let mut remote = seed_repo
            .remote("origin", remote_path.to_str().expect("path is utf8"))
            .expect("failed to add remote");
        let head = seed_repo.head().expect("seed repo should have a HEAD");
        let refname = head.name().expect("HEAD should be named").to_string();
        remote
            .push(&[format!("{refname}:{refname}")], None)
            .expect("failed to push seed commit");

        (remote_dir, remote_path)
    }

    /// Clones `remote_path` into a fresh temp dir, returning the clone.
    fn clone_local(remote_path: &Path) -> (TempDir, std::path::PathBuf) {
        let local_dir = tempfile::tempdir().expect("failed to create temp dir");
        Repository::clone(
            remote_path.to_str().expect("path is utf8"),
            local_dir.path(),
        )
        .expect("failed to clone local repo");
        let local_path = local_dir.path().to_path_buf();
        (local_dir, local_path)
    }

    /// Stages and commits every change in `repo`'s working tree with a
    /// throwaway signature, on top of the current HEAD (or as a root commit
    /// if there is none yet). Used only to set up test fixtures.
    fn commit_all(repo: &Repository, message: &str) {
        let mut index = repo.index().expect("failed to get index");
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .expect("failed to stage files");
        index.write().expect("failed to write index");
        let tree_id = index.write_tree().expect("failed to write tree");
        let tree = repo.find_tree(tree_id).expect("failed to find tree");
        let signature =
            git2::Signature::now("Test", "test@example.com").expect("failed to build signature");
        let parents = match repo.head() {
            Ok(head) => vec![head.peel_to_commit().expect("HEAD should be a commit")],
            Err(_) => Vec::new(),
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parent_refs,
        )
        .expect("failed to commit");
    }

    /// Reads a file's contents from a bare repo's current branch tip, or
    /// `None` if it's not present there. Used to check what actually made
    /// it to the remote, as opposed to just the local working tree.
    fn read_file_from_remote_tip(remote_path: &Path, filename: &str) -> Option<String> {
        let repo = Repository::open_bare(remote_path).expect("failed to open bare remote");
        let head = repo.head().expect("remote should have a HEAD");
        let commit = head.peel_to_commit().expect("HEAD should be a commit");
        let tree = commit.tree().expect("commit should have a tree");
        let entry = tree.get_name(filename)?;
        let blob = entry
            .to_object(&repo)
            .expect("entry should resolve to an object")
            .peel_to_blob()
            .expect("entry should be a blob");
        Some(String::from_utf8_lossy(blob.content()).into_owned())
    }

    /// The commit messages on `repo_path`'s current branch, oldest first.
    fn commit_messages(repo_path: &Path) -> Vec<String> {
        let repo = Repository::open(repo_path).expect("failed to open repo");
        let mut revwalk = repo.revwalk().expect("failed to create revwalk");
        revwalk.push_head().expect("failed to push HEAD");
        revwalk
            .set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::REVERSE)
            .expect("failed to set sorting");
        revwalk
            .map(|oid| {
                let oid = oid.expect("revwalk should yield a valid oid");
                repo.find_commit(oid)
                    .expect("oid should resolve to a commit")
                    .message()
                    .expect("commit message should be utf8")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn sync_write_commits_the_write_with_the_given_message_and_pushes_it_to_the_remote() {
        let (_remote_dir, remote_path) = seeded_bare_remote();
        let (_local_dir, local_path) = clone_local(&remote_path);

        crate::git_sync::sync_write(&local_path, "add hello", || {
            std::fs::write(local_path.join("hello.txt"), "hello\n")
        })
        .expect("sync_write should succeed");

        assert!(local_path.join("hello.txt").is_file());
        assert_eq!(
            commit_messages(&local_path),
            vec!["seed".to_string(), "add hello".to_string()]
        );
        assert_eq!(
            read_file_from_remote_tip(&remote_path, "hello.txt"),
            Some("hello\n".to_string()),
            "the commit should have been pushed to the remote"
        );
    }

    #[test]
    fn sync_write_does_not_commit_or_push_when_the_write_fails() {
        let (_remote_dir, remote_path) = seeded_bare_remote();
        let (_local_dir, local_path) = clone_local(&remote_path);

        let result: Result<(), crate::git_sync::SyncError<&str>> =
            crate::git_sync::sync_write(&local_path, "should never land", || Err("write failed"));

        assert!(matches!(result, Err(crate::git_sync::SyncError::Write(_))));
        assert_eq!(commit_messages(&local_path), vec!["seed".to_string()]);
        assert_eq!(read_file_from_remote_tip(&remote_path, "hello.txt"), None);
    }

    #[test]
    fn sync_write_pulls_a_concurrent_remote_change_before_writing_and_committing() {
        let (_remote_dir, remote_path) = seeded_bare_remote();
        // Clone "local" first, before the other machine's change lands, so
        // it starts out stale relative to the remote.
        let (_local_dir, local_path) = clone_local(&remote_path);

        // A second machine pulls, writes its own file, and pushes — all
        // before "local" does anything.
        let (_other_dir, other_path) = clone_local(&remote_path);
        std::fs::write(other_path.join("other.txt"), "other machine\n")
            .expect("failed to write other machine's file");
        let other_repo = Repository::open(&other_path).expect("failed to open other repo");
        commit_all(&other_repo, "other machine's change");
        let mut other_remote = other_repo
            .find_remote("origin")
            .expect("other repo should have a remote");
        let other_refname = other_repo
            .head()
            .expect("other repo should have a HEAD")
            .name()
            .expect("HEAD should be named")
            .to_string();
        other_remote
            .push(&[format!("{other_refname}:{other_refname}")], None)
            .expect("other machine's push should succeed");

        // "local" is still stale at this point: it hasn't seen other.txt.
        assert!(!local_path.join("other.txt").is_file());

        // Without a pull first, this commit's parent would be "seed", not
        // "other machine's change" — so pushing it would be rejected as a
        // non-fast-forward. A successful push here proves the pull happened
        // and was folded in before the new commit was made.
        crate::git_sync::sync_write(&local_path, "local machine's change", || {
            std::fs::write(local_path.join("hello.txt"), "hello\n")
        })
        .expect("sync_write should succeed");

        assert!(
            local_path.join("other.txt").is_file(),
            "the pull should have brought in the other machine's file"
        );
        assert_eq!(
            commit_messages(&local_path),
            vec![
                "seed".to_string(),
                "other machine's change".to_string(),
                "local machine's change".to_string(),
            ]
        );
        assert_eq!(
            read_file_from_remote_tip(&remote_path, "other.txt"),
            Some("other machine\n".to_string())
        );
        assert_eq!(
            read_file_from_remote_tip(&remote_path, "hello.txt"),
            Some("hello\n".to_string())
        );
    }

    #[test]
    fn sync_write_does_not_write_when_the_pull_fails() {
        let (remote_dir, remote_path) = seeded_bare_remote();
        let (_local_dir, local_path) = clone_local(&remote_path);
        // Break the remote so the pull can't succeed, without touching the
        // local working tree otherwise.
        drop(remote_dir);

        let write_ran = std::cell::Cell::new(false);
        let result = crate::git_sync::sync_write(&local_path, "should never land", || {
            write_ran.set(true);
            std::fs::write(local_path.join("hello.txt"), "hello\n")
        });

        assert!(matches!(result, Err(crate::git_sync::SyncError::Pull(_))));
        assert!(!write_ran.get(), "write should not run when the pull fails");
        assert!(!local_path.join("hello.txt").is_file());
        assert_eq!(commit_messages(&local_path), vec!["seed".to_string()]);
    }

    #[test]
    fn sync_write_fails_on_a_genuine_divergence_between_local_and_remote_history() {
        let (_remote_dir, remote_path) = seeded_bare_remote();
        let (_local_dir, local_path) = clone_local(&remote_path);

        // A second machine pulls, writes its own file, and pushes — the
        // remote now has a commit "local" hasn't seen.
        let (_other_dir, other_path) = clone_local(&remote_path);
        std::fs::write(other_path.join("other.txt"), "other machine\n")
            .expect("failed to write other machine's file");
        let other_repo = Repository::open(&other_path).expect("failed to open other repo");
        commit_all(&other_repo, "other machine's change");
        let mut other_remote = other_repo
            .find_remote("origin")
            .expect("other repo should have a remote");
        let other_refname = other_repo
            .head()
            .expect("other repo should have a HEAD")
            .name()
            .expect("HEAD should be named")
            .to_string();
        other_remote
            .push(&[format!("{other_refname}:{other_refname}")], None)
            .expect("other machine's push should succeed");

        // "local" also makes its own commit, without ever pulling the other
        // machine's change first — so local and remote both moved on from
        // "seed" independently. Neither is an ancestor of the other.
        std::fs::write(local_path.join("local-only.txt"), "local machine\n")
            .expect("failed to write local-only file");
        let local_repo = Repository::open(&local_path).expect("failed to open local repo");
        commit_all(&local_repo, "local machine's unpushed change");

        let write_ran = std::cell::Cell::new(false);
        let result = crate::git_sync::sync_write(&local_path, "should never land", || {
            write_ran.set(true);
            std::fs::write(local_path.join("hello.txt"), "hello\n")
        });

        assert!(
            matches!(result, Err(crate::git_sync::SyncError::Pull(_))),
            "a genuine divergence should surface as a pull error, got {result:?}"
        );
        assert!(!write_ran.get(), "write should not run when the pull fails");
        assert!(!local_path.join("hello.txt").is_file());
        assert!(
            !local_path.join("other.txt").is_file(),
            "a failed pull should not partially apply the remote's change"
        );
        assert_eq!(
            commit_messages(&local_path),
            vec![
                "seed".to_string(),
                "local machine's unpushed change".to_string(),
            ]
        );
        assert_eq!(
            read_file_from_remote_tip(&remote_path, "local-only.txt"),
            None,
            "the diverged local commit should not have been pushed"
        );
    }

    #[test]
    fn sync_write_does_not_push_when_write_leaves_the_working_tree_unchanged() {
        let (_remote_dir, remote_path) = seeded_bare_remote();
        let (_local_dir, local_path) = clone_local(&remote_path);

        crate::git_sync::sync_write(&local_path, "no-op write", || {
            // Writes and then removes the same file, so the working tree
            // ends up byte-for-byte identical to HEAD's tree.
            std::fs::write(local_path.join("scratch.txt"), "temporary\n")?;
            std::fs::remove_file(local_path.join("scratch.txt"))
        })
        .expect("sync_write should succeed even when there is nothing to commit");

        assert_eq!(
            commit_messages(&local_path),
            vec!["seed".to_string()],
            "no new commit should have been made"
        );
        assert_eq!(
            read_file_from_remote_tip(&remote_path, "scratch.txt"),
            None,
            "nothing should have been pushed"
        );
    }
}
