//! `cargo xtask secrets`: looks for committed secrets with gitleaks (house rule 10), in the files
//! of the working tree and in every commit of the history.
//!
//! The history goes through `git log -p`, piped into `gitleaks stdin`, so the check does not depend
//! on how gitleaks starts git itself (a gitleaks built for another processor than the Mac's fails
//! to, and then reports "0 commits scanned" as if it had found nothing).

use std::io::Write;
use std::process::{Command, Stdio};

const CONFIG: &str = ".gitleaks.toml";

/// Whether gitleaks is installed.
pub fn available() -> bool {
    Command::new("gitleaks").arg("version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Runs both scans; an error names what was found.
pub fn run() -> Result<(), String> {
    if !available() {
        return Err("gitleaks is not installed (https://github.com/gitleaks/gitleaks); `cargo xtask secrets` needs it".to_string());
    }
    eprintln!("$ gitleaks dir .");
    let status =
        Command::new("gitleaks").args(["dir", "--redact", "--no-banner", "--config", CONFIG, "."]).status().map_err(|e| format!("gitleaks: {e}"))?;
    if !status.success() {
        return Err("gitleaks found secrets in the working tree".to_string());
    }
    eprintln!("$ git log -p --all | gitleaks stdin");
    let mut log = Command::new("git")
        .args(["log", "-p", "--all", "--full-history"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("git: {e}"))?;
    let mut scan = Command::new("gitleaks")
        .args(["stdin", "--redact", "--no-banner", "--config", CONFIG])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("gitleaks: {e}"))?;
    if let (Some(mut from), Some(mut to)) = (log.stdout.take(), scan.stdin.take()) {
        // gitleaks may stop reading once it has what it needs; that is not an error here.
        let _ = std::io::copy(&mut from, &mut to);
        let _ = to.flush();
    }
    let _ = log.wait();
    let status = scan.wait().map_err(|e| format!("gitleaks: {e}"))?;
    if status.success() { Ok(()) } else { Err("gitleaks found secrets in the history".to_string()) }
}
