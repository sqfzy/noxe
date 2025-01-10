#![feature(let_chains)]

mod cli;
mod process;

fn main() {
    use clap::Parser;

    let args = cli::Cli::parse();

    if let Err(e) = process::process_command(args) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
