//! `scancraft-cli` (specification §6.9). Subcommands are filled in by WP4 (`synth`, `eval`) and
//! WP12 (`detect`, `page`, `flatten`, `pdf`).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod batch;
mod run;

use scancraft_core::command::Command;
use std::process::ExitCode;

const USAGE: &str = "usage: scancraft-cli <command> [args]

commands:
  detect   <photo> [--json]                   find the page (A1–A10, A25)
  page     <photo> [--look ..] [--flatten ..] render one page
  flatten  <photo> [--json]                   the sheet fits and the flatten decision (A25–A30)
  pdf      <photos..> [--book] [--order ..]   write a searchable PDF
  synth    [--curved] --count N --seed S --out DIR
  eval     --set NAME --root DIR --report FILE
  commands                                    list command ids";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("commands") => {
            for id in Command::IDS {
                println!("{id}");
            }
            ExitCode::SUCCESS
        }
        Some("synth") => batch::synth(&args[1..]),
        Some("eval") => batch::eval(&args[1..]),
        Some("detect") => run::detect_or_flatten(&args[1..], false),
        Some("flatten") => run::detect_or_flatten(&args[1..], true),
        Some("page") => run::page(&args[1..]),
        Some("pdf") => run::pdf(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(64)
        }
    }
}
