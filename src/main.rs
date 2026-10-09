use std::path::PathBuf;

use clap::{Parser, Subcommand};
use sift::{
    catalog::Catalog,
    library::{self, Cancellation},
    search::SearchQuery,
};

#[derive(Parser)]
#[command(version, about = "Sift local sample catalog")]
struct Arguments {
    #[arg(long, global = true, default_value = ".local/sift.sqlite")]
    database: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Index metadata from a chosen folder without modifying any source files.
    Index { folder: PathBuf },
    /// Search indexed names and relative paths (up to 128 results per page).
    Search {
        #[arg(default_value = "")]
        query: String,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
}

fn main() -> std::process::ExitCode {
    match run(Arguments::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Sift operation failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(arguments: Arguments) -> sift::Result<()> {
    match arguments.command {
        Command::Index { folder } => {
            let mut catalog = Catalog::open(&arguments.database)?;
            let report = library::scan(&folder, &mut catalog, &Cancellation::default(), |_| {})?;
            println!(
                "Indexed {} files; {} metadata/traversal errors; {} links skipped",
                report.indexed, report.errors, report.skipped_links
            );
            for issue in report.issues {
                eprintln!(
                    "Index warning for {}: {}",
                    issue.path.display(),
                    issue.message
                );
            }
        }
        Command::Search { query, offset } => {
            let query = SearchQuery::parse(&query)?;
            let page = Catalog::open_reader(&arguments.database)?.search(&query, offset)?;
            println!("{} results; page offset {}", page.total, page.offset);
            for sample in page.rows {
                println!(
                    "{}\t{}\t{}",
                    sample.name, sample.format, sample.display_path
                );
            }
        }
    }
    Ok(())
}
