mod cli;
mod client;
mod config;
mod errors;
mod parser;
mod print;
mod processor;
mod utils;

use {
    cli::app,
    errors::CliError,
    processor::process,
};

fn main() -> Result<(), CliError> {
    process(&app().get_matches()).map_err(|e| {
        print_error!("{}", e);
        e
    })
}
