//! husmo binary entry point.
//!
//! Locates and loads the config file, then serves the MCP server
//! (`husmo::mcp_server::HusmoServer`) over stdio for the lifetime of the
//! process, per `docs/ARCHITECTURE.md` ("MCP server": "Transport: stdio,
//! spawned per session by the MCP client.").

use husmo::config;
use husmo::mcp_server::HusmoServer;
use rmcp::ServiceExt;
use rmcp::transport::io::stdio;

#[tokio::main]
async fn main() {
    let Some(path) = config::default_path() else {
        eprintln!(
            "husmo: could not determine a config file location (set HUSMO_CONFIG, \
             XDG_CONFIG_HOME, or HOME)"
        );
        std::process::exit(1);
    };

    let cfg = match config::load(&path) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("husmo: {err}");
            std::process::exit(1);
        }
    };

    let server = HusmoServer::new(cfg.data_repo_path);
    let running = match server.serve(stdio()).await {
        Ok(running) => running,
        Err(err) => {
            eprintln!("husmo: failed to start MCP server: {err}");
            std::process::exit(1);
        }
    };

    if let Err(err) = running.waiting().await {
        eprintln!("husmo: MCP server exited with an error: {err}");
        std::process::exit(1);
    }
}
