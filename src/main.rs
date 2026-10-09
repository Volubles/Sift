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
    #[arg(long, global = true)]
    database: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Open the desktop metadata browser.
    #[cfg(feature = "desktop")]
    Browse {
        #[arg(long)]
        folder: Option<PathBuf>,
        /// Capture one native frame to a PPM file and close (developer smoke test).
        #[arg(long, hide = true)]
        smoke_screenshot: Option<PathBuf>,
    },
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
    let database = match arguments.database {
        Some(database) => database,
        None => sift::config::default_database()?,
    };
    let command = arguments.command.unwrap_or({
        #[cfg(feature = "desktop")]
        {
            Command::Browse {
                folder: None,
                smoke_screenshot: None,
            }
        }
        #[cfg(not(feature = "desktop"))]
        {
            Command::Search {
                query: String::new(),
                offset: 0,
            }
        }
    });
    match command {
        #[cfg(feature = "desktop")]
        Command::Browse {
            folder,
            smoke_screenshot,
        } => {
            run_desktop(database, folder, smoke_screenshot)?;
        }
        Command::Index { folder } => {
            let mut catalog = Catalog::open(&database)?;
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
            let page = Catalog::open_reader(&database)?.search(&query, offset)?;
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

#[cfg(feature = "desktop")]
fn run_desktop(
    database: PathBuf,
    folder: Option<PathBuf>,
    smoke_screenshot: Option<PathBuf>,
) -> sift::Result<()> {
    sift::desktop::run(sift::desktop::Options {
        database,
        folder,
        smoke_screenshot,
    })
    .map_err(|error| sift::Error::Desktop(error.to_string()))
}
