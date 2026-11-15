//! The crate layering of specification §6.1, checked from `cargo metadata`.
//!
//! - A crate depends only on crates in lower layers, or earlier in its own layer's chain.
//! - Nothing below L4 depends on a UI crate.
//! - Only the apps depend on `scancraft-apple`.
//! - `scancraft-synth` depends on no workspace crate but `scancraft-core`, so it checks the
//!   pipeline independently, and only the apps and xtask depend on it.

use serde_json::Value;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Layer(u8),
    /// `scancraft-synth`.
    Tool,
    /// Apps and xtask.
    Exempt,
}

const TABLE: &[(&str, Class)] = &[
    ("scancraft-core", Class::Layer(0)),
    ("scancraft-geom", Class::Layer(0)),
    ("scancraft-raster", Class::Layer(1)),
    ("scancraft-detect", Class::Layer(2)),
    ("scancraft-enhance", Class::Layer(2)),
    ("scancraft-io", Class::Layer(2)),
    ("scancraft-engine", Class::Layer(3)),
    ("scancraft-apple", Class::Layer(3)),
    ("scancraft-ui-egui", Class::Layer(4)),
    ("scancraft-synth", Class::Tool),
    ("scancraft", Class::Exempt),
    ("scancraft-cli", Class::Exempt),
    ("xtask", Class::Exempt),
];

/// Within a layer, a crate may use those before it in its chain.
const CHAINS: &[&[&str]] = &[&["scancraft-core", "scancraft-geom"]];

/// Windowing and interface crates, allowed from L4.
const UI_CRATES: &[&str] = &["egui", "eframe", "egui_kittest", "winit", "craft-ui", "craft-menu", "craft-dialogs", "craft-keymap", "craft-quit"];

fn class(name: &str) -> Option<Class> {
    TABLE.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

fn chain_allows(from: &str, to: &str) -> bool {
    CHAINS.iter().any(|c| match (c.iter().position(|n| *n == from), c.iter().position(|n| *n == to)) {
        (Some(f), Some(t)) => t < f,
        _ => false,
    })
}

struct Dep {
    name: String,
    dev: bool,
}

/// Returns the violations for the given crates and their dependencies.
fn check(crates: &[(String, Vec<Dep>)]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, deps) in crates {
        let Some(cls) = class(name) else {
            out.push(format!("{name}: not in xtask/src/layers.rs TABLE"));
            continue;
        };
        for d in deps {
            let target = class(&d.name);
            match (cls, target) {
                (Class::Exempt, _) => {}
                (_, Some(Class::Exempt)) => out.push(format!("{name} -> {}: apps and xtask are not dependencies", d.name)),
                (Class::Tool, Some(_)) if d.name != "scancraft-core" => {
                    out.push(format!("{name} -> {}: the scene generator may use only scancraft-core", d.name))
                }
                (Class::Layer(_), Some(Class::Tool)) if !d.dev => {
                    out.push(format!("{name} -> {}: only the apps and xtask may use the scene generator", d.name))
                }
                (Class::Layer(_), Some(Class::Layer(_))) if d.name == "scancraft-apple" => {
                    out.push(format!("{name} -> scancraft-apple: only the apps may use it"))
                }
                (Class::Layer(l), Some(Class::Layer(t))) if t > l || (t == l && !chain_allows(name, &d.name)) => {
                    out.push(format!("{name} (L{l}) -> {} (L{t}): may only depend on lower layers", d.name))
                }
                (Class::Layer(l), None) if l < 4 && UI_CRATES.contains(&d.name.as_str()) => {
                    out.push(format!("{name} (L{l}) -> {}: interface crates start at L4", d.name))
                }
                _ => {}
            }
        }
    }
    out
}

fn metadata() -> Result<Vec<(String, Vec<Dep>)>, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo).args(["metadata", "--format-version", "1", "--no-deps"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    let packages = v.get("packages").and_then(Value::as_array).ok_or("cargo metadata: no packages")?;
    let mut crates = Vec::new();
    for p in packages {
        let name = p.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let deps = p
            .get("dependencies")
            .and_then(Value::as_array)
            .map(|ds| {
                ds.iter()
                    .map(|d| Dep {
                        name: d.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                        dev: d.get("kind").and_then(Value::as_str) == Some("dev"),
                    })
                    .collect()
            })
            .unwrap_or_default();
        crates.push((name, deps));
    }
    Ok(crates)
}

pub fn run() -> Result<(), String> {
    let crates = metadata()?;
    let violations = check(&crates);
    if violations.is_empty() {
        println!("layers: {} crates, no violations", crates.len());
        Ok(())
    } else {
        Err(format!("layer violations:\n  {}", violations.join("\n  ")))
    }
}

#[cfg(test)]
mod tests {
    use super::{Dep, check};

    fn c(name: &str, deps: &[&str]) -> (String, Vec<Dep>) {
        (name.to_string(), deps.iter().map(|d| Dep { name: d.to_string(), dev: false }).collect())
    }

    #[test]
    fn the_rules_catch_each_kind_of_violation() {
        assert!(check(&[c("scancraft-geom", &["scancraft-core"])]).is_empty());
        assert_eq!(check(&[c("scancraft-core", &["scancraft-geom"])]).len(), 1);
        assert_eq!(check(&[c("scancraft-detect", &["scancraft-enhance"])]).len(), 1);
        assert_eq!(check(&[c("scancraft-engine", &["egui"])]).len(), 1);
        assert_eq!(check(&[c("scancraft-engine", &["scancraft-apple"])]).len(), 1);
        assert_eq!(check(&[c("scancraft-synth", &["scancraft-geom"])]).len(), 1);
        assert_eq!(check(&[c("scancraft-engine", &["scancraft-synth"])]).len(), 1);
        assert_eq!(check(&[c("scancraft-newthing", &[])]).len(), 1);
        assert!(check(&[c("scancraft", &["scancraft-apple", "eframe"])]).is_empty());
    }
}
