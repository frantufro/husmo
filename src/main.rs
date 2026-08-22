//! husmo binary entry point.
//!
//! With no subcommand: locates and loads the config file, then serves the
//! MCP server (`husmo::mcp_server::HusmoServer`) over stdio for the
//! lifetime of the process, per `docs/ARCHITECTURE.md` ("MCP server":
//! "Transport: stdio, spawned per session by the MCP client.").
//!
//! With `init`: runs the `husmo init` CLI subcommand instead (see
//! `husmo::init` and `docs/ARCHITECTURE.md`, "Bootstrapping a data repo:
//! `husmo init`"). This is a one-shot, synchronous CLI command, not an MCP
//! tool — it never touches the MCP server.
//!
//! `--version`/`-V` comes from clap's `#[command(version)]`, which prints
//! `husmo <CARGO_PKG_VERSION>` — matched against by the Homebrew formula's
//! `test do` block (see `.github/workflows/release.yml`).

use std::io::Write as _;

use clap::{Parser, Subcommand};
use husmo::config;
use husmo::mcp_server::HusmoServer;
use rmcp::ServiceExt;
use rmcp::transport::io::stdio;

#[derive(Parser)]
#[command(
    name = "husmo",
    version,
    about = "Local-first, git-backed document/link database with a Rust MCP server"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Bootstrap a data repo: clone it and write the config file
    Init {
        /// Data repo git URL; prompted for interactively if omitted
        #[arg(long, value_name = "URL")]
        repo: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Init { repo }) => run_init(repo),
        None => {
            let runtime = tokio::runtime::Runtime::new()
                .expect("failed to build the tokio runtime for the MCP server");
            runtime.block_on(serve());
        }
    }
}

/// Serves the MCP server over stdio for the lifetime of the process.
async fn serve() {
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

/// Runs the `husmo init` subcommand: resolves the data repo's git URL (from
/// `--repo` or an interactive prompt), clones it into the current
/// directory, and writes the config file to point at it.
fn run_init(repo_flag: Option<String>) {
    let repo_url = match husmo::init::resolve_repo_url(repo_flag, prompt_for_repo_url) {
        Ok(url) => url,
        Err(err) => {
            eprintln!("husmo init: {err}");
            std::process::exit(1);
        }
    };

    let dest_dir = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("husmo init: failed to determine the current directory: {err}");
            std::process::exit(1);
        }
    };

    let Some(config_path) = config::default_path() else {
        eprintln!(
            "husmo init: could not determine a config file location (set HUSMO_CONFIG, \
             XDG_CONFIG_HOME, or HOME)"
        );
        std::process::exit(1);
    };

    match husmo::init::run(&repo_url, &dest_dir, &config_path) {
        Ok(()) => println!(
            "husmo: cloned data repo into {} and wrote config at {}",
            dest_dir.display(),
            config_path.display()
        ),
        Err(err) => {
            eprintln!("husmo init: {err}");
            std::process::exit(1);
        }
    }
}

/// Prompts the user interactively, on stdout/stdin, for the data repo's
/// git URL.
fn prompt_for_repo_url() -> std::io::Result<String> {
    print!("Data repo git URL: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::{Cli, Commands};

    #[test]
    fn no_arguments_selects_no_subcommand() {
        let cli = Cli::parse_from(["husmo"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn init_with_no_repo_flag_leaves_it_none() {
        let cli = Cli::parse_from(["husmo", "init"]);
        assert!(matches!(cli.command, Some(Commands::Init { repo: None })));
    }

    #[test]
    fn init_with_repo_flag_captures_its_value() {
        let cli = Cli::parse_from(["husmo", "init", "--repo", "git@example.com:data.git"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Init { repo: Some(url) }) if url == "git@example.com:data.git"
        ));
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["husmo", "--bogus"]).is_err());
    }
}
