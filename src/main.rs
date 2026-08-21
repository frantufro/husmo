//! husmo binary entry point.
//!
//! For now this just locates and loads the config file and reports the data
//! repo path it points at. The MCP server itself is scaffolded in a later
//! roadmap task.

use husmo::config;

fn main() {
    let Some(path) = config::default_path() else {
        eprintln!(
            "husmo: could not determine a config file location (set HUSMO_CONFIG, \
             XDG_CONFIG_HOME, or HOME)"
        );
        std::process::exit(1);
    };

    match config::load(&path) {
        Ok(cfg) => {
            println!("husmo: using data repo at {}", cfg.data_repo_path.display());
        }
        Err(err) => {
            eprintln!("husmo: {err}");
            std::process::exit(1);
        }
    }
}
