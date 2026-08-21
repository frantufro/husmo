//! Test-only fixtures shared across modules' `#[cfg(test)]` suites. Not
//! part of the public API — gated behind `#[cfg(test)]` in `lib.rs`.

use std::path::PathBuf;

use git2::Repository;
use tempfile::TempDir;

/// Creates a bare "remote" repo with one seeded commit (a file named
/// `seed.txt`) on its default branch, so a clone of it has a HEAD to work
/// from. Used by both `crate::git_sync`'s and `crate::init`'s tests, which
/// each need a real, clonable git URL.
pub(crate) fn seeded_bare_remote() -> (TempDir, PathBuf) {
    let remote_dir = tempfile::tempdir().expect("failed to create temp dir");
    let remote_path = remote_dir.path().join("remote.git");
    Repository::init_bare(&remote_path).expect("failed to init bare remote");

    // Seed it via a throwaway working clone, since a fresh bare repo has no
    // branches to clone from yet.
    let seed_dir = tempfile::tempdir().expect("failed to create temp dir");
    let seed_repo = Repository::init(seed_dir.path()).expect("failed to init seed repo");
    std::fs::write(seed_dir.path().join("seed.txt"), "seed\n").expect("failed to write seed");

    let mut index = seed_repo.index().expect("failed to get index");
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .expect("failed to stage files");
    index.write().expect("failed to write index");
    let tree_id = index.write_tree().expect("failed to write tree");
    let tree = seed_repo.find_tree(tree_id).expect("failed to find tree");
    let signature =
        git2::Signature::now("Test", "test@example.com").expect("failed to build signature");
    seed_repo
        .commit(Some("HEAD"), &signature, &signature, "seed", &tree, &[])
        .expect("failed to commit seed");

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
