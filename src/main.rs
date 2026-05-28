use std::env;
use std::process;
use tke::{
    AppError, Config, Dispatch, capture_interactive, parse_dispatch, print_activate,
    print_deactivate, run_shim, run_tty_wrapped, run_wrapped, usage, usage_stats,
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
        Dispatch::Tty {
            name,
            args,
            shim_dir,
        } => {
            let code = run_tty_wrapped(&name, &args, shim_dir, &config)?;
            process::exit(code);
        }
        Dispatch::Deactivate => {
            print_deactivate();
            Ok(())
        }
        Dispatch::CaptureInteractive { source, output } => {
            capture_interactive(source, output, &config)
        }
        Dispatch::Stats {
            sources,
            limit,
            filter,
            group_by,
            changed_only,
            refresh,
            top,
            sort_by,
            json,
        } => usage_stats(
            sources,
            limit,
            filter,
            group_by,
            changed_only,
            refresh,
            top,
            sort_by,
            json,
            &config,
        ),
        Dispatch::Shim { name, args } | Dispatch::ShimExec { name, args } => {
            let code = run_shim(&name, &args, &config)?;
            process::exit(code);
        }
    }
}
