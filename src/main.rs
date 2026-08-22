//! husmo binary entry point.
//!
//! With no arguments: locates and loads the config file, then serves the
//! MCP server (`husmo::mcp_server::HusmoServer`) over stdio for the
//! lifetime of the process, per `docs/ARCHITECTURE.md` ("MCP server":
//! "Transport: stdio, spawned per session by the MCP client.").
//!
//! With `init` as the first argument: runs the `husmo init` CLI
//! subcommand instead (see `husmo::init` and `docs/ARCHITECTURE.md`,
//! "Bootstrapping a data repo: `husmo init`"). This is a one-shot,
//! synchronous CLI command, not an MCP tool — it never touches the MCP
//! server.

use std::io::Write as _;

use husmo::config;
use husmo::mcp_server::HusmoServer;
use rmcp::ServiceExt;
use rmcp::transport::io::stdio;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next() {
        Some(subcommand) if subcommand == "init" => run_init(&args.collect::<Vec<_>>()),
        Some(flag) if flag == "--version" || flag == "-V" => println!("{}", version_line()),
        Some(other) => {
            eprintln!("husmo: unknown argument '{other}'");
            std::process::exit(1);
        }
        None => {
            let runtime = tokio::runtime::Runtime::new()
                .expect("failed to build the tokio runtime for the MCP server");
            runtime.block_on(serve());
        }
    }
}

/// The `husmo --version` / `husmo -V` output — matched against by the
/// Homebrew formula's `test do` block (see `.github/workflows/release.yml`),
/// so it must stay a single line starting with the binary name followed by
/// its Cargo package version.
fn version_line() -> String {
    format!("husmo {}", env!("CARGO_PKG_VERSION"))
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

/// Runs the `husmo init` subcommand: parses `args` for an optional
/// `--repo <url>`, resolves the data repo's git URL (from that flag or an
/// interactive prompt), clones it into the current directory, and writes
/// the config file to point at it.
fn run_init(args: &[String]) {
    let repo_flag = match parse_init_args(args) {
        Ok(flag) => flag,
        Err(err) => {
            eprintln!("husmo init: {err}");
            std::process::exit(1);
        }
    };

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

/// Parses `husmo init`'s arguments for an optional `--repo <url>` flag.
/// Returns `Ok(None)` when no `--repo` flag was given, in which case the
/// caller falls back to prompting interactively.
fn parse_init_args(args: &[String]) -> Result<Option<String>, String> {
    let mut repo = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--repo" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "--repo requires a value".to_string())?;
                repo = Some(value.clone());
                i += 2;
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    Ok(repo)
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
    #[test]
    fn version_line_reports_the_binary_name_and_cargo_package_version() {
        assert_eq!(
            super::version_line(),
            format!("husmo {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn parse_init_args_returns_none_when_no_flag_is_given() {
        assert_eq!(super::parse_init_args(&[]), Ok(None));
    }

    #[test]
    fn parse_init_args_returns_the_repo_flags_value() {
        let args = vec!["--repo".to_string(), "git@example.com:data.git".to_string()];

        assert_eq!(
            super::parse_init_args(&args),
            Ok(Some("git@example.com:data.git".to_string()))
        );
    }

    #[test]
    fn parse_init_args_rejects_a_repo_flag_missing_its_value() {
        let args = vec!["--repo".to_string()];

        assert_eq!(
            super::parse_init_args(&args),
            Err("--repo requires a value".to_string())
        );
    }

    #[test]
    fn parse_init_args_rejects_unknown_arguments() {
        let args = vec!["--bogus".to_string()];

        assert_eq!(
            super::parse_init_args(&args),
            Err("unknown argument '--bogus'".to_string())
        );
    }
}
