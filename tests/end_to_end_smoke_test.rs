//! End-to-end smoke test for the roadmap task
//! `end-to-end-smoke-test-and-readme-finalization`.
//!
//! Every module exercised here already has its own focused unit/integration
//! tests elsewhere in the crate (`src/save.rs`, `src/archive.rs`,
//! `src/related.rs`, `src/mcp_server.rs`, ...). This test exercises the
//! wiring between them instead, as one continuous story: `save` a URL,
//! discover an outgoing link, archive that link as its own Document, relate
//! the two, confirm `get` surfaces the Related list, confirm
//! `search-semantic` finds a Document by meaning, then `delete` it — the
//! same flow described in `docs/ARCHITECTURE.md` end to end, driven through
//! the real MCP tool surface wherever a tool exists for the step, against a
//! real (git-backed) data repo and a real local HTTP fixture server.
//!
//! Lives in `tests/` (a Cargo integration test) rather than alongside a
//! `src/` module, since it's specifically about the seams between modules
//! rather than any one module's own behavior.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

use husmo::extract::OutgoingLink;
use rmcp::model::CallToolRequestParams;
use rmcp::{ClientHandler, ServiceExt, serde_json};
use tempfile::TempDir;

/// A minimal MCP client used only to drive `HusmoServer` in this test. It
/// forwards nothing of its own — it exists so the real MCP initialize/
/// call-tool handshake runs over the wire, the same way a real MCP client
/// would exercise the server. Mirrors the equivalent fixture in
/// `src/mcp_server.rs`'s own tests.
#[derive(Debug, Clone, Default)]
struct TestClient;

impl ClientHandler for TestClient {
    fn get_info(&self) -> rmcp::model::ClientInfo {
        rmcp::model::ClientInfo::default()
    }
}

/// Creates a bare "remote" repo with one seeded commit, then clones it into
/// a fresh temp dir. The `save`/`relate`/`delete` tools' git pull/commit/push
/// cycle (`crate::git_sync::sync_write`) needs a real `origin` remote to push
/// to. Mirrors the equivalent fixture in `src/mcp_server.rs`'s own tests.
/// Returns both temp dirs (kept alive for the test's duration) and the local
/// clone's path.
fn seeded_data_repo() -> (TempDir, TempDir, PathBuf) {
    let remote_dir = tempfile::tempdir().expect("failed to create temp dir");
    let remote_path = remote_dir.path().join("remote.git");
    git2::Repository::init_bare(&remote_path).expect("failed to init bare remote");

    let seed_dir = tempfile::tempdir().expect("failed to create temp dir");
    let seed_repo = git2::Repository::init(seed_dir.path()).expect("failed to init seed repo");
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

    let local_dir = tempfile::tempdir().expect("failed to create temp dir");
    git2::Repository::clone(
        remote_path.to_str().expect("path is utf8"),
        local_dir.path(),
    )
    .expect("failed to clone local repo");
    let local_path = local_dir.path().to_path_buf();

    (remote_dir, local_dir, local_path)
}

/// Stages and commits every change in `repo`'s working tree with a
/// throwaway signature. Used only to seed test fixtures.
fn commit_all(repo: &git2::Repository, message: &str) {
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

/// Starts a one-shot HTTP server on an OS-assigned localhost port that
/// replies to a single connection with an HTML page whose body is `body`,
/// then shuts down. Mirrors the equivalent fixture duplicated across
/// `src/fetch.rs`, `src/save.rs`, `src/archive.rs`, and `src/mcp_server.rs`.
fn one_shot_page_server(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test listener");
    let addr = listener.local_addr().expect("failed to read local addr");
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("failed to accept connection");
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("failed to write test response");
    });
    format!("http://{addr}/")
}

/// Serves a fresh `HusmoServer` rooted at `data_repo_path` and calls its
/// tool named `tool_name` with `arguments`, over a real (in-memory) MCP
/// client/server connection — end to end through the protocol, not just a
/// direct function call. Mirrors the equivalent fixture in
/// `src/mcp_server.rs`'s own tests.
async fn call_tool(
    data_repo_path: PathBuf,
    tool_name: &'static str,
    arguments: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ServiceError> {
    let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);

    let server = husmo::mcp_server::HusmoServer::new(data_repo_path);
    let server_handle = tokio::spawn(async move {
        server
            .serve(server_transport)
            .await
            .expect("server should start")
            .waiting()
            .await
            .expect("server should shut down cleanly");
    });

    let client = TestClient
        .serve(client_transport)
        .await
        .expect("client should connect");

    let result = client
        .call_tool(
            CallToolRequestParams::new(tool_name).with_arguments(
                arguments
                    .as_object()
                    .expect("arguments should be a JSON object")
                    .clone(),
            ),
        )
        .await;

    client
        .cancel()
        .await
        .expect("client should shut down cleanly");
    server_handle.await.expect("server task should not panic");

    result
}

/// Calls `call_tool` and unwraps its structured content, panicking with
/// `context` on either failure — keeps the flow below readable as a
/// sequence of steps rather than a wall of `expect` boilerplate.
async fn call_tool_ok(
    data_repo_path: PathBuf,
    tool_name: &'static str,
    arguments: serde_json::Value,
    context: &str,
) -> serde_json::Value {
    call_tool(data_repo_path, tool_name, arguments)
        .await
        .unwrap_or_else(|error| panic!("{context}: tool call failed: {error}"))
        .structured_content
        .unwrap_or_else(|| panic!("{context}: tool call returned no structured content"))
}

/// Step 1: saves `main_url` via the `save` tool and returns the saved
/// Document's id, plus the single outgoing link it should have discovered
/// (asserting there is exactly one, pointing at `discovered_url`).
async fn save_step(
    data_repo_path: PathBuf,
    main_url: String,
    discovered_url: &str,
) -> (String, OutgoingLink) {
    let save_result = call_tool_ok(
        data_repo_path,
        "save",
        serde_json::json!({ "url": main_url }),
        "save",
    )
    .await;

    let main_id = save_result["document"]["id"]
        .as_str()
        .expect("save should return the saved document's id")
        .to_string();
    let outgoing_links = save_result["outgoing_links"]
        .as_array()
        .expect("save should return the discovered outgoing links as an array");
    assert_eq!(
        outgoing_links.len(),
        1,
        "expected exactly one outgoing link to be discovered, got {outgoing_links:?}"
    );
    assert_eq!(outgoing_links[0]["url"], discovered_url);
    let link = OutgoingLink {
        text: outgoing_links[0]["text"]
            .as_str()
            .expect("the outgoing link should carry its visible text")
            .to_string(),
        url: discovered_url.to_string(),
    };
    (main_id, link)
}

/// Step 2: archives `link` as its own Document. There is no dedicated MCP
/// tool for this yet (see `docs/ARCHITECTURE.md`, "Content extraction": the
/// "ask a human which links look worth archiving" behavior is left to a
/// Skill layered on top, not this server), so this calls the library
/// function directly, exactly like `crate::archive`'s own tests do. Returns
/// the archived Document's id, asserting it's distinct from `main_id` and
/// that its `canonical_url` is the link's own url.
async fn archive_step(data_repo_path: PathBuf, link: OutgoingLink, main_id: &str) -> String {
    let link_url = link.url.clone();
    // Run on the blocking thread pool, not directly on this async task: like
    // `save`'s own tool handler (see `src/mcp_server.rs`), fetching the
    // link's page is blocking I/O via `reqwest`'s blocking client, which
    // builds (and tears down) its own inner Tokio runtime — doing that
    // directly from within an async task's own poll panics.
    let archived = tokio::task::spawn_blocking(move || {
        husmo::archive::archive_outgoing_link(&data_repo_path, &link)
    })
    .await
    .expect("archiving task should not panic")
    .expect("archiving the discovered link should succeed");

    assert_eq!(
        archived.document.canonical_url,
        Some(link_url),
        "the archived Document's canonical_url should be the discovered link's url"
    );
    assert_ne!(
        archived.document.id, main_id,
        "archiving should create a second, distinct Document"
    );
    archived.document.id
}

/// Step 5: fetches `main_id` via the `get` tool and asserts its Related list
/// names `archived_id` by reference.
async fn get_step(data_repo_path: PathBuf, main_id: &str, archived_id: &str) {
    let main_via_get = call_tool_ok(
        data_repo_path,
        "get",
        serde_json::json!({ "id": main_id }),
        "get",
    )
    .await;
    assert_eq!(
        main_via_get["related"],
        serde_json::json!([{ "id": archived_id, "title": "Discovered Page" }]),
        "get should list the archived Document as Related, by reference"
    );
}

/// Step 6: runs a `search-semantic` query expected to rank `main_id` first
/// among the (only two) Documents in the data repo.
async fn search_step(data_repo_path: PathBuf, main_id: &str) {
    let search_result = call_tool_ok(
        data_repo_path,
        "search-semantic",
        serde_json::json!({
            "query": "systems programming in a strongly typed language",
            "top_k": 1,
        }),
        "search-semantic",
    )
    .await;
    let hits = search_result["hits"]
        .as_array()
        .expect("search-semantic should return its hits as an array");
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0]["document"]["id"], main_id,
        "search-semantic should rank the Rust-programming page above the unrelated \
         baking page for a query about programming"
    );
}

/// Step 7: deletes `main_id` via the `delete` tool and asserts it's gone
/// from the data repo's current state while `archived_id` survives.
async fn delete_step(data_repo_path: PathBuf, main_id: &str, archived_id: &str) {
    let deleted = call_tool_ok(
        data_repo_path.clone(),
        "delete",
        serde_json::json!({ "id": main_id }),
        "delete",
    )
    .await;
    assert_eq!(deleted["id"], main_id);

    let remaining = husmo::store::load_all(&data_repo_path).expect("load_all should succeed");
    assert!(
        remaining.iter().all(|document| document.id != main_id),
        "the deleted Document should no longer be present in the data repo's current state"
    );
    assert!(
        remaining.iter().any(|document| document.id == archived_id),
        "deleting the main Document should not also remove the archived one"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn save_discover_archive_relate_get_search_and_delete_flow_end_to_end() {
    // The link `save` will discover is served by its own one-shot listener,
    // started first so its address can be embedded as an `<a href>` in the
    // main page below.
    let discovered_url = one_shot_page_server(
        "<html><head><title>Discovered Page</title></head>\
         <body><article><p>Bake a sourdough loaf for forty minutes at high heat.</p>\
         </article></body></html>",
    );
    let main_page = format!(
        "<html><head><title>Main Page</title></head>\
         <body><article><p>Rust is a systems programming language with strong static \
         typing. See <a href=\"{discovered_url}\">a related page</a> for more.</p>\
         </article></body></html>"
    );
    let main_url = one_shot_page_server(Box::leak(main_page.into_boxed_str()));

    let (_remote_dir, _local_dir, data_repo_path) = seeded_data_repo();

    let (main_id, link) = save_step(data_repo_path.clone(), main_url, &discovered_url).await;
    let archived_id = archive_step(data_repo_path.clone(), link, &main_id).await;

    call_tool_ok(
        data_repo_path.clone(),
        "relate",
        serde_json::json!({ "id_a": main_id, "id_b": archived_id }),
        "relate",
    )
    .await;

    get_step(data_repo_path.clone(), &main_id, &archived_id).await;
    search_step(data_repo_path.clone(), &main_id).await;
    delete_step(data_repo_path, &main_id, &archived_id).await;
}
