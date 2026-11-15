//! The `synth` and `eval` subcommands (specification §6.9, §10): batch operations of the tool, not
//! user commands, so they call `scancraft-synth` directly. Arguments are parsed by hand; a usage
//! error exits with 64 and a message, a failed job with 1, and nothing panics on junk.

use scancraft_synth::SceneSpec;
use scancraft_synth::datasets::{Dataset, reference, smartdoc, synthetic, uvdoc};
use scancraft_synth::eval::{read_found, score, write_report};
use scancraft_synth::generate::MAX_COUNT;
use scancraft_synth::scene_io::generate_set;
use std::path::PathBuf;
use std::process::ExitCode;

/// Exit code for a command line that doesn't parse.
const USAGE_ERROR: u8 = 64;

pub const SYNTH_USAGE: &str = "usage: scancraft-cli synth [--curved] --count N --seed S --out DIR";
pub const EVAL_USAGE: &str = "usage: scancraft-cli eval --set smartdoc2015|uvdoc|reference|synthetic --root DIR --report FILE [--found FILE]";

fn usage(message: &str, usage: &str) -> ExitCode {
    eprintln!("scancraft-cli: {message}\n{usage}");
    ExitCode::from(USAGE_ERROR)
}

/// The flags of one command: `--name value` pairs and bare switches, each at most once.
struct Flags {
    values: Vec<(String, String)>,
    switches: Vec<String>,
}

/// Parses `args` against the flags that take a value and the switches that don't.
fn parse_flags(args: &[String], with_value: &[&str], switches: &[&str]) -> Result<Flags, String> {
    let mut flags = Flags { values: Vec::new(), switches: Vec::new() };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let seen = |f: &Flags| f.values.iter().any(|(n, _)| n == arg) || f.switches.contains(arg);
        if switches.contains(&arg.as_str()) {
            if seen(&flags) {
                return Err(format!("{arg} is given twice"));
            }
            flags.switches.push(arg.clone());
        } else if with_value.contains(&arg.as_str()) {
            if seen(&flags) {
                return Err(format!("{arg} is given twice"));
            }
            let value = it.next().filter(|v| !v.starts_with("--")).ok_or_else(|| format!("{arg} needs a value"))?;
            flags.values.push((arg.clone(), value.clone()));
        } else {
            return Err(format!("unknown argument {arg:?}"));
        }
    }
    Ok(flags)
}

impl Flags {
    fn value(&self, name: &str) -> Option<&str> {
        self.values.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    fn required(&self, name: &str) -> Result<&str, String> {
        self.value(name).ok_or_else(|| format!("{name} is required"))
    }
}

/// `synth [--curved] --count N --seed S --out DIR`
pub fn synth(args: &[String]) -> ExitCode {
    let spec = (|| -> Result<(SceneSpec, PathBuf), String> {
        let flags = parse_flags(args, &["--count", "--seed", "--out"], &["--curved"])?;
        let count: u32 = flags.required("--count")?.parse().map_err(|_| "--count must be a whole number".to_string())?;
        if count == 0 || count > MAX_COUNT {
            return Err(format!("--count must be from 1 to {MAX_COUNT}"));
        }
        let seed: u64 = flags.required("--seed")?.parse().map_err(|_| "--seed must be a whole number".to_string())?;
        let out = PathBuf::from(flags.required("--out")?);
        Ok((SceneSpec { count, seed, curved: flags.switches.iter().any(|s| s == "--curved") }, out))
    })();
    let (spec, out) = match spec {
        Ok(v) => v,
        Err(message) => return usage(&message, SYNTH_USAGE),
    };
    match generate_set(&spec, &out) {
        Ok(n) => {
            println!("wrote {n} {} scenes to {}", if spec.curved { "bent" } else { "flat" }, out.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("scancraft-cli synth: {e}");
            ExitCode::from(1)
        }
    }
}

fn load_set(name: &str, root: &std::path::Path) -> Result<Dataset, String> {
    match name {
        "smartdoc2015" => smartdoc::load(root),
        "uvdoc" => uvdoc::load(root),
        "reference" => reference::load(root),
        "synthetic" => synthetic::load(root),
        other => return Err(format!("unknown set {other:?}; the sets are smartdoc2015, uvdoc, reference and synthetic")),
    }
    .map_err(|e| e.to_string())
}

/// `eval --set NAME --root DIR --report FILE [--found FILE]`
pub fn eval(args: &[String]) -> ExitCode {
    let parsed = (|| -> Result<(String, PathBuf, PathBuf, Option<PathBuf>), String> {
        let flags = parse_flags(args, &["--set", "--root", "--report", "--found"], &[])?;
        let set = flags.required("--set")?.to_string();
        if !["smartdoc2015", "uvdoc", "reference", "synthetic"].contains(&set.as_str()) {
            return Err(format!("unknown set {set:?}"));
        }
        Ok((set, PathBuf::from(flags.required("--root")?), PathBuf::from(flags.required("--report")?), flags.value("--found").map(PathBuf::from)))
    })();
    let (set, root, report_path, found_path) = match parsed {
        Ok(v) => v,
        Err(message) => return usage(&message, EVAL_USAGE),
    };
    let run = || -> Result<String, String> {
        let dataset = load_set(&set, &root)?;
        let found = found_path.as_deref().map(read_found).transpose().map_err(|e| e.to_string())?;
        let report = score(&dataset, found.as_deref());
        write_report(&report, &report_path).map_err(|e| e.to_string())?;
        Ok(format!(
            "{}: {} photos, {} scored, {} skipped; report written to {}",
            report.set,
            report.photos,
            report.scored,
            report.skipped_count,
            report_path.display()
        ))
    };
    match run() {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("scancraft-cli eval: {message}");
            ExitCode::from(1)
        }
    }
}
