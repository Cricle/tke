use std::env;
use std::path::PathBuf;
use std::process;
use tke::{AppError, Config, benchmark_commands, compare_e2e_command, compare_rollout};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        process::exit(1);
    }
}

fn run() -> Result<(), AppError> {
    let args: Vec<String> = env::args().collect();
    let sub = args.get(1).map(String::as_str);
    let config = Config::load()?;

    match sub {
        Some("compare-rollout") => {
            let mut source = None;
            let mut iter = args.into_iter().skip(2);
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "--source" => {
                        let value = iter.next().ok_or_else(|| {
                            AppError::Usage("missing value for --source".to_owned())
                        })?;
                        source = Some(PathBuf::from(value));
                    }
                    other => {
                        return Err(AppError::Usage(format!(
                            "unknown compare-rollout arg `{other}`"
                        )));
                    }
                }
            }
            compare_rollout(source, &config)
        }
        Some("compare-e2e") => {
            let mut sources = Vec::new();
            let mut agent = None;
            let mut iter = args.into_iter().skip(2);
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "--source" => {
                        let value = iter.next().ok_or_else(|| {
                            AppError::Usage("missing value for --source".to_owned())
                        })?;
                        sources.push(PathBuf::from(value));
                    }
                    "--agent" => {
                        let value = iter.next().ok_or_else(|| {
                            AppError::Usage("missing value for --agent".to_owned())
                        })?;
                        agent = Some(value);
                    }
                    other => {
                        return Err(AppError::Usage(format!(
                            "unknown compare-e2e arg `{other}`"
                        )));
                    }
                }
            }
            compare_e2e_command(sources, agent, &config)
        }
        Some("benchmark-commands") => {
            let mut check = false;
            for arg in args.into_iter().skip(2) {
                match arg.as_str() {
                    "--check" => check = true,
                    other => {
                        return Err(AppError::Usage(format!(
                            "unknown benchmark-commands arg `{other}`"
                        )));
                    }
                }
            }
            benchmark_commands(&config, check)
        }
        _ => {
            eprintln!("tke-bench - tke benchmarking and comparison tools");
            eprintln!();
            eprintln!("Usage:");
            eprintln!("  tke-bench compare-rollout [--source PATH]");
            eprintln!("  tke-bench compare-e2e [--source DIR]... [--agent codex|claude]");
            eprintln!("  tke-bench benchmark-commands [--check]");
            process::exit(1);
        }
    }
}
