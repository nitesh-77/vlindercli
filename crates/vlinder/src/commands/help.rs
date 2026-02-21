use std::path::PathBuf;

use crate::config::{logs_dir, vlinder_dir};

use super::fleet;

/// Run the bundled support fleet.
///
/// Syntactic sugar for `vlinder fleet run -p <bundled-support-fleet>`.
/// Ensures the logs directory exists so the log-analyst mount succeeds.
pub fn execute() {
    // Ensure logs directory exists for the log-analyst mount
    let logs = logs_dir();
    if !logs.exists() {
        std::fs::create_dir_all(&logs).unwrap_or_else(|e| {
            eprintln!("Failed to create logs directory {}: {}", logs.display(), e);
            std::process::exit(1);
        });
    }

    // Locate the bundled support fleet relative to the binary's source tree.
    // In development: CARGO_MANIFEST_DIR/fleets/support
    // In production: this would be an installed path (deferred).
    let fleet_path = bundled_fleet_path();

    if !fleet_path.exists() {
        eprintln!(
            "Support fleet not found at {}. Run from the vlinder project root.",
            fleet_path.display()
        );
        std::process::exit(1);
    }

    fleet::run(Some(fleet_path));
}

fn bundled_fleet_path() -> PathBuf {
    // Production: ~/.vlinder/support-fleet (written by installer)
    let installed = vlinder_dir().join("support-fleet");
    if installed.exists() {
        return installed;
    }
    // Development: source tree
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fleets").join("support")
}
