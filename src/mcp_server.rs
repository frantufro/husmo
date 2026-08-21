//! The MCP server itself, over stdio transport, spawned per session (see
//! `docs/ARCHITECTURE.md`, "MCP server": "Transport: stdio, spawned per
//! session by the MCP client. No persistent background daemon, no
//! auto-start requirement, for now."). This module wires the tool surface
//! declared there onto the business logic already implemented elsewhere in
//! the crate (`crate::save` for the `save` tool), adding the one thing each
//! of those modules leaves to its caller: the git pull/commit/push cycle
//! (`crate::git_sync::sync_write`).
//!
//! The `save` and `get` tools are implemented here. The rest of the tool
//! surface (`search-*`, `relate`/`unrelate`, `list`, `delete`) is added by
//! later roadmap tasks onto this same [`HusmoServer`].

use std::path::PathBuf;

use rmcp::ErrorData;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::extract::OutgoingLink;
use crate::git_sync::{self, SyncError};
use crate::save::{self, SaveError, SaveInput};
use crate::store::{self, IdentifierError, ResolveError};

/// The husmo MCP server: holds the data repo path every tool operates
/// against. Constructed once per session and served over stdio (see
/// `crate::main`). Has no `ToolRouter` field of its own — per-call routing
/// is generated fresh by `#[tool_router]`/`#[tool_handler]` below.
#[derive(Debug, Clone)]
pub struct HusmoServer {
    data_repo_path: PathBuf,
}

impl HusmoServer {
    /// Creates a server whose tools all operate against the data repo at
    /// `data_repo_path`.
    #[must_use]
    pub fn new(data_repo_path: PathBuf) -> Self {
        Self { data_repo_path }
    }
}

/// Parameters for the `save` tool. Exactly one of `url`/`path`/`content` is
/// required, validated server-side by [`crate::save::save_input`]; `content`
/// additionally requires `title`, since pasted text has no page or filename
/// to derive one from.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveParams {
    /// A URL to fetch and save. Exactly one of `url`, `path`, or `content`
    /// is required.
    #[serde(default)]
    pub url: Option<String>,
    /// A local file path to ingest. Exactly one of `url`, `path`, or
    /// `content` is required.
    #[serde(default)]
    pub path: Option<String>,
    /// Pasted/typed content to save directly. Exactly one of `url`, `path`,
    /// or `content` is required; requires `title`.
    #[serde(default)]
    pub content: Option<String>,
    /// The Document's title. Required when `content` is supplied; ignored
    /// for `url`/`path`, which derive their own title during ingestion.
    #[serde(default)]
    pub title: Option<String>,
    /// Tags to apply to the saved Document.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// The wire shape of a Document returned from a tool call. Mirrors
/// [`crate::document::Document`], with `saved_at` rendered as an RFC 3339
/// string rather than relying on a `chrono` `JsonSchema` impl.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DocumentDto {
    /// See [`Document::id`].
    pub id: String,
    /// See [`Document::slug`].
    pub slug: String,
    /// See [`Document::canonical_url`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_url: Option<String>,
    /// See [`Document::title`].
    pub title: String,
    /// See [`Document::tags`].
    pub tags: Vec<String>,
    /// See [`Document::saved_at`], rendered as RFC 3339.
    pub saved_at: String,
    /// See [`Document::summary`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// See [`Document::author`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// See [`Document::content`].
    pub content: String,
    /// See [`Document::related`].
    pub related: Vec<String>,
}

impl From<Document> for DocumentDto {
    fn from(document: Document) -> Self {
        DocumentDto {
            id: document.id,
            slug: document.slug,
            canonical_url: document.canonical_url,
            title: document.title,
            tags: document.tags,
            saved_at: document.saved_at.to_rfc3339(),
            summary: document.summary,
            author: document.author,
            content: document.content,
            related: document.related,
        }
    }
}

/// Parameters for the `get` tool. Exactly one of `id`/`slug`/`url` is
/// required, validated server-side by [`crate::store::identifier`].
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetParams {
    /// The Document's stable internal id. Exactly one of `id`, `slug`, or
    /// `url` is required.
    #[serde(default)]
    pub id: Option<String>,
    /// The Document's slug. Exactly one of `id`, `slug`, or `url` is
    /// required.
    #[serde(default)]
    pub slug: Option<String>,
    /// The Document's canonical URL. Exactly one of `id`, `slug`, or `url`
    /// is required.
    #[serde(default)]
    pub url: Option<String>,
}

/// The wire shape of an outgoing link discovered during extraction. Mirrors
/// [`crate::extract::OutgoingLink`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct OutgoingLinkDto {
    /// See [`OutgoingLink::text`].
    pub text: String,
    /// See [`OutgoingLink::url`].
    pub url: String,
}

impl From<OutgoingLink> for OutgoingLinkDto {
    fn from(link: OutgoingLink) -> Self {
        OutgoingLinkDto {
            text: link.text,
            url: link.url,
        }
    }
}

/// The `save` tool's result: the saved Document plus any outgoing links
/// discovered in its content, reported as data only per
/// `docs/ARCHITECTURE.md` ("Content extraction") — never followed
/// automatically.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SaveResult {
    /// The Document that was written.
    pub document: DocumentDto,
    /// Outgoing links discovered in the Document's own content, in the
    /// order they appear.
    pub outgoing_links: Vec<OutgoingLinkDto>,
}

#[tool_router]
impl HusmoServer {
    /// Saves a URL, pasted text, or local file as a Document: dispatches
    /// to the matching ingestion path (`crate::save::save`), then runs the
    /// git pull/commit/push cycle (`crate::git_sync::sync_write`) around
    /// the write, per `docs/ARCHITECTURE.md` ("Git mechanics").
    #[tool(
        description = "Save a URL, pasted text, or local file as a Document. Exactly \
            one of `url`, `path`, or `content` is required; `content` additionally \
            requires `title`. Re-saving a `url` that was already saved overwrites its \
            Document in place instead of duplicating it. Returns the saved Document \
            plus any outgoing links discovered in its content, reported as data only \
            and never followed automatically."
    )]
    async fn save(
        &self,
        Parameters(params): Parameters<SaveParams>,
    ) -> Result<Json<SaveResult>, ErrorData> {
        let input = save::save_input(params.url, params.path, params.content, params.title)
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        let message = commit_message(&input);
        let sync_dir = self.data_repo_path.clone();
        let save_dir = self.data_repo_path.clone();
        let tags = params.tags;

        // Ingesting a URL performs blocking network I/O, and the git
        // pull/commit/push cycle is blocking file and process I/O, so the
        // whole write runs on tokio's blocking thread pool rather than
        // directly on an async worker thread: `reqwest`'s blocking client
        // builds (and later tears down) its own inner Tokio runtime, which
        // panics if attempted from within an async task's own poll.
        let output = tokio::task::spawn_blocking(move || {
            git_sync::sync_write(&sync_dir, &message, move || {
                save::save(&save_dir, input, tags)
            })
        })
        .await
        .map_err(|error| ErrorData::internal_error(format!("save task panicked: {error}"), None))?
        .map_err(|error| sync_error_to_mcp_error(&error))?;

        Ok(Json(SaveResult {
            document: output.document.into(),
            outgoing_links: output.outgoing_links.into_iter().map(Into::into).collect(),
        }))
    }

    /// Looks up exactly one Document by `id`, `slug`, or `url`
    /// (`crate::store::identifier` validates that exactly one was supplied,
    /// `crate::store::resolve` finds it), including its Related list by
    /// reference. Unlike `save`, this is a pure local read with no network
    /// I/O and no git pull/commit/push cycle, so it runs directly rather
    /// than on the blocking thread pool.
    #[tool(
        description = "Look up a Document by exactly one of `id`, `slug`, or `url`. Returns \
            the Document, including its Related list by reference (ids only; use `search-*` \
            with expansion to pull in their content)."
    )]
    async fn get(&self, Parameters(params): Parameters<GetParams>) -> Result<Json<DocumentDto>, ErrorData> {
        let identifier = store::identifier(params.id, params.slug, params.url)
            .map_err(identifier_error_to_mcp_error)?;
        let document = store::resolve(&self.data_repo_path, &identifier)
            .map_err(|error| resolve_error_to_mcp_error(&error))?;
        Ok(Json(document.into()))
    }
}

/// The git commit message for a `save` call, describing what was saved.
fn commit_message(input: &SaveInput) -> String {
    match input {
        SaveInput::Url(url) => format!("save: {url}"),
        SaveInput::LocalFile(path) => format!("save: {}", path.display()),
        SaveInput::PastedText { title, .. } => format!("save: {title}"),
    }
}

/// Maps a failure from the `save` tool's git pull/commit/push cycle to the
/// MCP error reported back to the client.
fn sync_error_to_mcp_error(error: &SyncError<SaveError>) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

/// Maps a failure from validating the `get` tool's `id`/`slug`/`url`
/// parameters to the MCP error reported back to the client.
fn identifier_error_to_mcp_error(error: IdentifierError) -> ErrorData {
    ErrorData::invalid_params(error.to_string(), None)
}

/// Maps a failure from resolving the `get` tool's identifier to a Document
/// to the MCP error reported back to the client.
fn resolve_error_to_mcp_error(error: &ResolveError) -> ErrorData {
    match error {
        ResolveError::NotFound(_) => ErrorData::resource_not_found(error.to_string(), None),
        ResolveError::Store(_) => ErrorData::internal_error(error.to_string(), None),
    }
}

#[tool_handler(
    instructions = "husmo: a local-first, git-backed document/link database. See \
        docs/ARCHITECTURE.md for the full tool surface."
)]
impl ServerHandler for HusmoServer {}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rmcp::model::CallToolRequestParams;
    use rmcp::{ClientHandler, ServiceExt, serde_json};

    use crate::mcp_server::HusmoServer;

    /// A minimal MCP client used only to drive `HusmoServer` in these
    /// tests. It forwards nothing of its own — it exists so the real MCP
    /// initialize/call-tool handshake runs over the wire, the same way a
    /// real MCP client would exercise the server.
    #[derive(Debug, Clone, Default)]
    struct TestClient;

    impl ClientHandler for TestClient {
        fn get_info(&self) -> rmcp::model::ClientInfo {
            rmcp::model::ClientInfo::default()
        }
    }

    /// Creates a bare "remote" repo with one seeded commit, then clones it
    /// into a fresh temp dir. The `save` tool's git pull/commit/push cycle
    /// (`crate::git_sync::sync_write`) needs a real `origin` remote to push
    /// to, the same fixture shape used in `crate::git_sync`'s own tests.
    /// Returns both temp dirs (kept alive for the test's duration) and the
    /// local clone's path.
    fn seeded_data_repo() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
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
        git2::Repository::clone(remote_path.to_str().expect("path is utf8"), local_dir.path())
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

    /// Serves a fresh `HusmoServer` rooted at `data_repo_path` and calls its
    /// `save` tool with `arguments`, over a real (in-memory) MCP
    /// client/server connection — end to end through the protocol, not just
    /// a direct function call.
    async fn call_save(
        data_repo_path: PathBuf,
        arguments: serde_json::Value,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ServiceError> {
        let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);

        let server = HusmoServer::new(data_repo_path);
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
                CallToolRequestParams::new("save").with_arguments(
                    arguments
                        .as_object()
                        .expect("arguments should be a JSON object")
                        .clone(),
                ),
            )
            .await;

        client.cancel().await.expect("client should shut down cleanly");
        server_handle.await.expect("server task should not panic");

        result
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn save_tool_saves_pasted_text_end_to_end() {
        let (_remote_dir, _local_dir, data_repo_path) = seeded_data_repo();

        let result = call_save(
            data_repo_path.clone(),
            serde_json::json!({
                "content": "Some pasted content.",
                "title": "My Pasted Note",
            }),
        )
        .await
        .expect("save tool call should succeed");

        let structured = result
            .structured_content
            .expect("save tool should return structured content");
        assert_eq!(structured["document"]["title"], "My Pasted Note");
        assert_eq!(structured["document"]["content"], "Some pasted content.");
        assert_eq!(structured["document"]["canonical_url"], serde_json::Value::Null);
        assert_eq!(structured["outgoing_links"], serde_json::json!([]));

        let on_disk = crate::store::load_all(&data_repo_path).expect("load_all should succeed");
        assert_eq!(on_disk.len(), 1, "the pasted note should have been written to the data repo");
        assert_eq!(on_disk[0].title, "My Pasted Note");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn save_tool_ingests_a_local_file_end_to_end() {
        let (_remote_dir, local_dir, data_repo_path) = seeded_data_repo();
        let file_path = local_dir.path().join("notes.txt");
        std::fs::write(&file_path, "Some file content.\n").expect("failed to write test file");

        let result = call_save(
            data_repo_path.clone(),
            serde_json::json!({ "path": file_path.to_string_lossy() }),
        )
        .await
        .expect("save tool call should succeed");

        let structured = result
            .structured_content
            .expect("save tool should return structured content");
        assert_eq!(structured["document"]["title"], "notes");
        assert_eq!(structured["document"]["content"], "Some file content.\n");
        assert_eq!(structured["document"]["canonical_url"], serde_json::Value::Null);

        let on_disk = crate::store::load_all(&data_repo_path).expect("load_all should succeed");
        assert_eq!(on_disk.len(), 1, "the ingested file should have been written to the data repo");
    }

    /// Starts a one-shot HTTP server on an OS-assigned localhost port that
    /// replies to a single connection with an HTML page whose body is
    /// `body`, then shuts down. Mirrors the helper in `crate::save`'s tests.
    fn one_shot_page_server(body: &'static str) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

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

    /// Like [`one_shot_page_server`], but binds once and answers two
    /// sequential connections in order, with `first_body` then
    /// `second_body`. Used to save the same URL twice against a server that
    /// stays alive for both fetches. Mirrors the helper in `crate::save`'s
    /// tests.
    fn two_shot_page_server(first_body: &'static str, second_body: &'static str) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test listener");
        let addr = listener.local_addr().expect("failed to read local addr");
        std::thread::spawn(move || {
            for body in [first_body, second_body] {
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
            }
        });
        format!("http://{addr}/")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn save_tool_fetches_a_url_end_to_end() {
        let url = one_shot_page_server(
            "<html><head><title>Fetched Page</title></head>\
             <body><article><p>Some content. See \
             <a href=\"https://further.example/\">more</a>.</p></article></body></html>",
        );
        let (_remote_dir, _local_dir, data_repo_path) = seeded_data_repo();

        let result = call_save(data_repo_path.clone(), serde_json::json!({ "url": url }))
            .await
            .expect("save tool call should succeed");

        let structured = result
            .structured_content
            .expect("save tool should return structured content");
        assert_eq!(structured["document"]["canonical_url"], url);
        assert_eq!(structured["document"]["title"], "Fetched Page");
        assert_eq!(
            structured["outgoing_links"],
            serde_json::json!([{ "text": "more", "url": "https://further.example/" }])
        );

        let on_disk = crate::store::load_all(&data_repo_path).expect("load_all should succeed");
        assert_eq!(on_disk.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn save_tool_overwrites_the_existing_document_when_the_same_url_is_saved_again() {
        let url = two_shot_page_server(
            "<html><head><title>Original Title</title></head>\
             <body><article><p>Original content.</p></article></body></html>",
            "<html><head><title>Original Title</title></head>\
             <body><article><p>Updated content.</p></article></body></html>",
        );
        let (_remote_dir, _local_dir, data_repo_path) = seeded_data_repo();

        let first = call_save(data_repo_path.clone(), serde_json::json!({ "url": url }))
            .await
            .expect("first save tool call should succeed")
            .structured_content
            .expect("save tool should return structured content");
        let second = call_save(data_repo_path.clone(), serde_json::json!({ "url": url }))
            .await
            .expect("second save tool call should succeed")
            .structured_content
            .expect("save tool should return structured content");

        assert_eq!(second["document"]["id"], first["document"]["id"]);
        assert_eq!(second["document"]["content"], "Updated content.");

        let on_disk = crate::store::load_all(&data_repo_path).expect("load_all should succeed");
        assert_eq!(
            on_disk.len(),
            1,
            "re-saving the same url should not create a second Document"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn save_tool_reports_a_validation_error_when_no_input_is_supplied() {
        let (_remote_dir, _local_dir, data_repo_path) = seeded_data_repo();

        let result = call_save(data_repo_path.clone(), serde_json::json!({})).await;

        assert!(
            result.is_err(),
            "calling save with none of url/path/content set should fail, got {result:?}"
        );
        let on_disk = crate::store::load_all(&data_repo_path).expect("load_all should succeed");
        assert!(
            on_disk.is_empty(),
            "no Document should have been written for an invalid call"
        );
    }

    /// Serves a fresh `HusmoServer` rooted at `data_repo_path` and calls its
    /// `get` tool with `arguments`, over a real (in-memory) MCP
    /// client/server connection. `get` is a read-only lookup, so unlike
    /// [`call_save`] this needs only a plain directory, not a git-backed
    /// data repo with a remote to push to.
    async fn call_get(
        data_repo_path: PathBuf,
        arguments: serde_json::Value,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ServiceError> {
        let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);

        let server = HusmoServer::new(data_repo_path);
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
                CallToolRequestParams::new("get").with_arguments(
                    arguments
                        .as_object()
                        .expect("arguments should be a JSON object")
                        .clone(),
                ),
            )
            .await;

        client.cancel().await.expect("client should shut down cleanly");
        server_handle.await.expect("server task should not panic");

        result
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_tool_resolves_the_same_document_by_id_slug_or_url() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let mut doc = crate::document::Document::new("My Title", "Some content.\n");
        doc.canonical_url = Some("https://example.com/post".to_string());
        crate::store::write(dir.path(), &doc).expect("write should succeed");

        let by_id = call_get(dir.path().to_path_buf(), serde_json::json!({ "id": doc.id }))
            .await
            .expect("get by id should succeed")
            .structured_content
            .expect("get tool should return structured content");
        let by_slug = call_get(dir.path().to_path_buf(), serde_json::json!({ "slug": doc.slug }))
            .await
            .expect("get by slug should succeed")
            .structured_content
            .expect("get tool should return structured content");
        let by_url = call_get(
            dir.path().to_path_buf(),
            serde_json::json!({ "url": "https://example.com/post" }),
        )
        .await
        .expect("get by url should succeed")
        .structured_content
        .expect("get tool should return structured content");

        for structured in [&by_id, &by_slug, &by_url] {
            assert_eq!(structured["id"], doc.id);
            assert_eq!(structured["title"], "My Title");
            assert_eq!(structured["content"], "Some content.\n");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_tool_includes_the_related_list_by_reference() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc_a = crate::document::Document::new("A", "content a");
        let doc_b = crate::document::Document::new("B", "content b");
        crate::store::write(dir.path(), &doc_a).expect("write should succeed");
        crate::store::write(dir.path(), &doc_b).expect("write should succeed");
        crate::related::relate(dir.path(), &doc_a.id, &doc_b.id).expect("relate should succeed");

        let structured = call_get(dir.path().to_path_buf(), serde_json::json!({ "id": doc_a.id }))
            .await
            .expect("get tool call should succeed")
            .structured_content
            .expect("get tool should return structured content");

        assert_eq!(structured["related"], serde_json::json!([doc_b.id]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_tool_reports_a_validation_error_when_zero_identifiers_are_supplied() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let result = call_get(dir.path().to_path_buf(), serde_json::json!({})).await;

        assert!(
            result.is_err(),
            "calling get with none of id/slug/url set should fail, got {result:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_tool_reports_a_validation_error_when_multiple_identifiers_are_supplied() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc = crate::document::Document::new("My Title", "content");
        crate::store::write(dir.path(), &doc).expect("write should succeed");

        let result = call_get(
            dir.path().to_path_buf(),
            serde_json::json!({ "id": doc.id, "slug": doc.slug }),
        )
        .await;

        assert!(
            result.is_err(),
            "calling get with both id and slug set should fail, got {result:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_tool_reports_a_not_found_error_for_an_unknown_identifier() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let result = call_get(
            dir.path().to_path_buf(),
            serde_json::json!({ "slug": "does-not-exist" }),
        )
        .await;

        assert!(
            result.is_err(),
            "calling get with an identifier matching no Document should fail, got {result:?}"
        );
    }
}
