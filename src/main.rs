mod cli_args;
mod cli_exec;
mod cli_output;
mod cli_parse;
mod cli_schema;
mod env_vars;
mod profile;
mod tui;

use anyhow::Result;
pub(crate) use cli_schema::*;

fn main() -> Result<()> {
    cli_exec::run()
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
