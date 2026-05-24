use std::env;
use std::process;
use tke::{
    AppError, Config, Dispatch, benchmark_commands, capture_interactive, compare_e2e_command,
    compare_rollout, install_self, parse_dispatch, print_activate, print_deactivate, run_shim,
    run_wrapped, usage,
};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        process::exit(1);
    }
}

fn run() -> Result<(), AppError> {
    let args = env::args().collect::<Vec<_>>();
    let argv0 = args.first().cloned().unwrap_or_else(|| "tke".to_owned());
    let dispatch = parse_dispatch(&argv0, args)?;
    let config = Config::load()?;

    match dispatch {
        Dispatch::Help => {
            println!("{}", usage());
            Ok(())
        }
        Dispatch::Install { bin_dir } => install_self(bin_dir),
        Dispatch::Activate {
            agents,
            shim_dir,
            shell,
        } => print_activate(&agents, shim_dir, shell, &config),
        Dispatch::Run {
            name,
            args,
            shim_dir,
        } => {
            let code = run_wrapped(&name, &args, shim_dir, &config)?;
            process::exit(code);
        }
        Dispatch::Deactivate => {
            print_deactivate();
            Ok(())
        }
        Dispatch::CaptureInteractive { source, output } => {
            capture_interactive(source, output, &config)
        }
        Dispatch::CompareRollout { source } => compare_rollout(source, &config),
        Dispatch::CompareE2e { sources, agent } => compare_e2e_command(sources, agent, &config),
        Dispatch::BenchmarkCommands { check } => benchmark_commands(&config, check),
        Dispatch::Shim { name, args } | Dispatch::ShimExec { name, args } => {
            let code = run_shim(&name, &args, &config)?;
            process::exit(code);
        }
    }
}
