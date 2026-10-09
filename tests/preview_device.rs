#![cfg(feature = "playback")]
mod common;

use std::sync::{Arc, Barrier, mpsc};
use std::time::{Duration, Instant};

use sift::audio::player::Player;
use sift::library::{self, Cancellation};

fn await_audio(player: &Player, previous: u64) {
    let started = Instant::now();
    while player.snapshot().rendered <= previous {
        while let Some(message) = player.poll() {
            if message.generation == player.generation()
                && let Some(error) = message.error
            {
                panic!("Device test failed: {error}");
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "Device callback did not progress"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "Requires an actual output device; run explicitly with --ignored --test-threads=1"]
fn switching_long_sources_remains_responsive_during_indexing() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("samples");
    std::fs::create_dir(&root).unwrap();
    let first = root.join("first.wav");
    let second = root.join("second.wav");
    common::wave(&first, 48_000, 48_000);
    common::wave(&second, 32_000, 32_000);
    for index in 0..140 {
        std::fs::copy(&first, root.join(format!("copy_{index:03}.wav"))).unwrap();
    }
    // A long declared PCM stream with a small real prefix exercises bounded
    // streaming without decoding the full two-minute file before playback.
    let long = root.join("long.wav");
    common::wave(&long, 48_000, 48_000);
    let mut header = std::fs::read(&long).unwrap();
    let data_bytes = 120u32 * 48_000 * 2;
    header[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
    header[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    std::fs::write(&long, header).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&long)
        .unwrap()
        .set_len(u64::from(44 + data_bytes))
        .unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let scanner_barrier = barrier.clone();
    let (progress_sender, progress_receiver) = mpsc::channel();
    let database = temporary.path().join("catalog.sqlite");
    let scanner = std::thread::spawn(move || {
        let mut catalog = sift::catalog::Catalog::open(&database).unwrap();
        let mut synchronized = false;
        library::scan(&root, &mut catalog, &Cancellation::default(), |_| {
            if !synchronized {
                progress_sender.send(()).unwrap();
                scanner_barrier.wait();
                synchronized = true;
            }
        })
        .unwrap()
    });
    progress_receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    let player = Player::start(|| {}).unwrap();
    player.volume(0.05).unwrap();
    player.play(first, 0.0).unwrap();
    await_audio(&player, 0);
    barrier.wait();
    let previous = player.snapshot().rendered;
    player.play(second, 0.0).unwrap();
    await_audio(&player, previous);
    let previous = player.snapshot().rendered;
    player.play(long, 0.0).unwrap();
    await_audio(&player, previous);
    player.stop().unwrap();
    assert_eq!(scanner.join().unwrap().indexed, 143);
    assert_eq!(player.snapshot().underruns, 0);
}
