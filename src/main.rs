use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{Args, Parser, Subcommand};
use std::process::ExitCode;

mod bundled;
mod cache;
mod commands;
mod generate;
mod github;
mod packages;
mod search;
mod sources;
mod symbols;
mod tree;
mod ui;

use commands::{cmd_install, cmd_local, cmd_search, cmd_symbols_dir};

const STYLES: Styles = Styles::styled()
    .header(AnsiColor::White.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::White.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Cyan.on_default())
    .placeholder(AnsiColor::White.on_default().effects(Effects::DIMMED))
    .valid(AnsiColor::Cyan.on_default())
    .invalid(AnsiColor::Yellow.on_default())
    .error(AnsiColor::Red.on_default().effects(Effects::BOLD));

/// Search, install and generate Volatility 3 symbols
#[derive(Parser)]
#[command(name = "vol3sm", version, styles = STYLES, disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Find symbols by banner, distro, kernel or file name
    Search {
        query: String,
        /// Results to show
        #[arg(short, long, default_value_t = 20, value_name = "N")]
        limit: usize,
    },

    /// Install a symbol by search id, source:path or banner
    Install(InstallArgs),

    /// Generate Linux symbols from a kernel debug package
    Generate(Box<generate::GenerateArgs>),

    /// List installed symbols
    Local,

    /// Print the symbols directory
    SymbolsDir,

    /// Print licenses for the bundled dwarf2json
    Licenses,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct InstallArgs {
    /// Search id or source:path
    #[arg(value_name = "ID|PATH")]
    path: Option<String>,
    /// Exact banner from `vol.py -f <dump> banners`
    #[arg(short, long)]
    banner: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Search { query, limit } => cmd_search(&query, limit),
        Commands::Install(args) => cmd_install(args.path, args.banner),
        Commands::Generate(args) => generate::cmd_generate(*args),
        Commands::Local => cmd_local(),
        Commands::SymbolsDir => cmd_symbols_dir(),
        Commands::Licenses => {
            bundled::print_licenses();
            Ok(())
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            ui::print_error(&err);
            ExitCode::FAILURE
        }
    }
}
