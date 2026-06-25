//! The `aether` CLI binary (U13; `watch` subcommand added in U16).
//!
//! `aether run --intent <f> [--input <f>] --ledger <db> [--cache <dir>] [--scratch <dir>]`

use aether_cli::{run, watch};

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") => match parse_run(&args[1..]) {
            Ok(opts) => dispatch_run(opts).await,
            Err(msg) => fail(&msg),
        },
        Some("watch") => match parse_run(&args[1..]) {
            Ok(opts) => dispatch_watch(opts).await,
            Err(msg) => fail(&msg),
        },
        _ => {
            eprintln!(
                "usage:\n  aether run   --intent <file> [--input <file>] --ledger <db> \
                 [--cache <dir>] [--scratch <dir>]\n  \
                 aether watch --intent <file> --source <dir> --ledger <db> \
                 [--cache <dir>] [--scratch <dir>]"
            );
            ExitCode::FAILURE
        }
    }
}

struct RunOpts {
    intent: PathBuf,
    input: Option<PathBuf>,
    ledger: PathBuf,
    cache: PathBuf,
    scratch: PathBuf,
}

fn parse_run(args: &[String]) -> std::result::Result<RunOpts, String> {
    let mut intent = None;
    let mut input = None;
    let mut ledger = None;
    let mut cache = PathBuf::from(".aether/cache");
    let mut scratch = PathBuf::from(".aether/scratch");

    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = args.get(i + 1).cloned();
        match flag {
            "--intent" => intent = value.map(PathBuf::from),
            // `--input` (run) and `--source` (watch) both populate `input`.
            "--input" | "--source" => input = value.map(PathBuf::from),
            "--ledger" => ledger = value.map(PathBuf::from),
            "--cache" => {
                if let Some(v) = value {
                    cache = PathBuf::from(v);
                }
            }
            "--scratch" => {
                if let Some(v) = value {
                    scratch = PathBuf::from(v);
                }
            }
            _ => {}
        }
        // Every recognized flag consumes a value; step two tokens at a time.
        i += 2;
    }

    Ok(RunOpts {
        intent: intent.ok_or("--intent is required")?,
        input,
        ledger: ledger.ok_or("--ledger is required")?,
        cache,
        scratch,
    })
}

async fn dispatch_run(opts: RunOpts) -> ExitCode {
    match run::execute(
        &opts.intent,
        opts.input.as_deref(),
        &opts.ledger,
        &opts.cache,
        &opts.scratch,
    )
    .await
    {
        Ok(outcome) => {
            println!(
                "ok: {} node(s) executed, net {} cents, ledger event {}",
                outcome.nodes_executed, outcome.result_cents, outcome.event_id
            );
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.to_string()),
    }
}

async fn dispatch_watch(opts: RunOpts) -> ExitCode {
    let source = match opts.input.as_deref() {
        Some(s) => s,
        None => return fail("--source is required for watch"),
    };
    match watch::watch(
        &opts.intent,
        source,
        &opts.ledger,
        &opts.cache,
        &opts.scratch,
        Duration::from_millis(500),
    )
    .await
    {
        Ok(n) => {
            println!("aether watch: exited after processing {n} file(s)");
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e.to_string()),
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("aether: {msg}");
    ExitCode::FAILURE
}
