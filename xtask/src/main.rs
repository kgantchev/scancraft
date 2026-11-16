//! `cargo xtask <command>`: the repository's checks and generators.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod fixtures;
mod layers;
mod secrets;

use std::process::{Command, ExitCode};

const USAGE: &str = "usage: cargo xtask <command>

commands:
  ci [-p PACKAGE]   fmt check, clippy -D warnings, tests and the layer check; with -p, one
                    package's fmt, clippy and tests (what a work package runs before its PR)
  layers            check the crate layering (specification §6.1)
  secrets           gitleaks over the working tree and the whole history (house rule 10)
  fixtures          redraw fixtures/ (build plan WP0 step 7)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("ci") => match (args.get(1).map(String::as_str), args.get(2)) {
            (None, _) => ci_all(),
            (Some("-p"), Some(package)) => ci_package(package),
            _ => Err(USAGE.to_string()),
        },
        Some("layers") => layers::run(),
        Some("secrets") => secrets::run(),
        Some("fixtures") => fixtures::run(),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn cargo(args: &[&str]) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    eprintln!("$ cargo {}", args.join(" "));
    let status = Command::new(cargo).args(args).status().map_err(|e| format!("cargo: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("cargo {} failed", args.join(" "))) }
}

fn ci_all() -> Result<(), String> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
    cargo(&["test", "--workspace", "--no-fail-fast"])?;
    layers::run()?;
    secrets_if_installed()
}

/// The secret scan runs wherever gitleaks is installed. The shared CI runners may not have it, and
/// the gate there must not fail for that; the warning says so instead of passing silently.
fn secrets_if_installed() -> Result<(), String> {
    if secrets::available() {
        secrets::run()
    } else {
        eprintln!("warning: gitleaks is not installed, so the secret scan was skipped (`cargo xtask secrets`)");
        Ok(())
    }
}

fn ci_package(package: &str) -> Result<(), String> {
    cargo(&["fmt", "-p", package, "--check"])?;
    cargo(&["clippy", "-p", package, "--all-targets", "--", "-D", "warnings"])?;
    cargo(&["test", "-p", package, "--no-fail-fast"])?;
    layers::run()
}
