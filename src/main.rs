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
    /// Exercise the real output device with a selected test file.
    #[cfg(feature = "playback")]
    AudioCheck {
        file: PathBuf,
        #[arg(long, default_value_t = false)]
        exercise: bool,
    },
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
        #[cfg(feature = "playback")]
        Command::AudioCheck { file, exercise } => audio_check(file, exercise)?,
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

#[cfg(feature = "playback")]
fn audio_check(file: PathBuf, exercise: bool) -> sift::Result<()> {
    use sift::audio::{player::Player, transport};
    use std::time::{Duration, Instant};
    let player = Player::start(|| {})?;
    player
        .play(file.clone(), 0.0)
        .map_err(|error| sift::Error::Desktop(error.to_string()))?;
    let started = Instant::now();
    let mut first_callback = None;
    let mut paused = false;
    let mut resumed = false;
    let mut reconnected = false;
    let mut reconnect_nonzero = 0;
    loop {
        while let Some(message) = player.poll() {
            if message.generation != player.generation() {
                continue;
            }
            if let Some(error) = message.error {
                return Err(sift::Error::Desktop(error));
            }
            if let Some(output) = message.output {
                println!("Output opened: {output}");
            }
        }
        let snapshot = player.snapshot();
        if snapshot.nonzero > 0 && first_callback.is_none() {
            first_callback = Some(started.elapsed());
        }
        if exercise && !paused && snapshot.seconds >= 0.08 {
            player
                .pause()
                .map_err(|error| sift::Error::Desktop(error.to_string()))?;
            std::thread::sleep(Duration::from_millis(150));
            let position = player.snapshot().seconds;
            std::thread::sleep(Duration::from_millis(100));
            if (player.snapshot().seconds - position).abs() > 1.0 / 48_000.0 {
                return Err(sift::Error::Desktop("Paused playhead advanced".into()));
            }
            player
                .resume()
                .map_err(|error| sift::Error::Desktop(error.to_string()))?;
            paused = true;
            resumed = true;
        }
        if exercise && resumed && !reconnected && snapshot.seconds >= 0.25 {
            reconnect_nonzero = snapshot.nonzero;
            player
                .reconnect(file.clone(), 0.5)
                .map_err(|error| sift::Error::Desktop(error.to_string()))?;
            reconnected = true;
        }
        if snapshot.phase == transport::ENDED && (!exercise || reconnected) {
            if exercise && snapshot.nonzero <= reconnect_nonzero {
                return Err(sift::Error::Desktop(
                    "Reopened output produced no nonzero PCM".into(),
                ));
            }
            if snapshot.nonzero == 0 {
                return Err(sift::Error::Desktop(
                    "No nonzero PCM reached the device callback".into(),
                ));
            }
            println!(
                "Device callback check passed: first nonzero callback {:?}, rendered {} frames, nonzero {}, underruns {}, final {:.3}s",
                first_callback,
                snapshot.rendered,
                snapshot.nonzero,
                snapshot.underruns,
                snapshot.seconds
            );
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(10) {
            return Err(sift::Error::Desktop(
                "Device callback check timed out".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
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
