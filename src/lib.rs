mod adapter;
mod app;
mod benchmark;
mod benchmark_data;
mod e2e_report;
mod file_profile;
mod log_profile;
mod path_profile;
mod rewrite;
mod rollout_io;
mod rollout_stats;
mod search_profile;
mod shim;
pub(crate) mod table_profile;
mod trim;

pub use app::{
    AppError, Config, Dispatch, benchmark_commands, compare_e2e_command, parse_dispatch,
    print_activate, print_deactivate, run_shim, run_wrapped, usage,
};
pub use rollout_io::{capture_interactive, compare_rollout, usage_stats};
pub use shim::run_tty_wrapped;
pub use trim::ShellKind;

#[cfg(test)]
mod tests;
