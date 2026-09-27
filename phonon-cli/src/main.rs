//! Phonon CLI - Command-line audio player.
//!
//! Provides a CLI interface for testing the complete audio pipeline.

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

use phonon_core::*;

/// Phonon - High-fidelity audio framework
#[derive(Parser)]
#[command(name = "phonon")]
#[command(version = "0.1.0")]
#[command(about = "High-fidelity audio player and framework")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Play audio files
    Play {
        /// Audio file or directory to play
        file: PathBuf,

        /// Audio output device
        #[arg(short, long)]
        device: Option<String>,

        /// Use exclusive mode (Bit-Perfect output)
        #[arg(short, long)]
        exclusive: bool,

        /// Loop playback
        #[arg(short, long)]
        r#loop: bool,

        /// Playlist file to load
        #[arg(short, long)]
        playlist: Option<PathBuf>,

        /// CUE track index to play (1-based, default: 1)
        #[arg(short = 't', long)]
        track: Option<usize>,
    },

    /// List audio output devices
    ListDevices,
}

fn main() {
    // Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    let cli = Cli::parse();

    if cli.verbose {
        log::set_max_level(log::LevelFilter::Debug);
    }

    match &cli.command {
        Commands::Play {
            file,
            device,
            exclusive,
            r#loop,
            playlist,
            track,
        } => {
            if let Err(e) = cmd_play(
                file,
                device.as_deref(),
                *exclusive,
                *r#loop,
                playlist.as_deref(),
                *track,
            ) {
                log::error!("Playback error: {}", e);
                std::process::exit(1);
            }
        }
        Commands::ListDevices => {
            if let Err(e) = cmd_list_devices() {
                log::error!("Error listing devices: {}", e);
                std::process::exit(1);
            }
        }
    }
}

/// Play a file or directory.
fn cmd_play(
    file: &Path,
    device: Option<&str>,
    exclusive: bool,
    loop_play: bool,
    playlist: Option<&Path>,
    track: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Phonon CLI Player v0.1.0");

    // Load playlist if provided — its entries override the positional `file`.
    let playlist_paths: Option<Vec<String>> = match playlist {
        Some(pl_path) => {
            log::info!("Loading playlist: {}", pl_path.display());
            let pl = phonon_source::Playlist::load(&pl_path.to_string_lossy())?;
            log::info!("Playlist loaded: {} entries", pl.len());
            for (i, entry) in pl.entries.iter().enumerate() {
                log::info!(
                    "  {}. {} - {}",
                    i + 1,
                    entry.title.as_deref().unwrap_or("Unknown"),
                    entry.path
                );
            }
            Some(pl.entries.iter().map(|e| e.path.clone()).collect())
        }
        None => None,
    };

    // Configure engine
    let mut config = EngineConfig::default();
    if let Some(dev) = device {
        config.device_name = dev.to_string();
    }
    config.exclusive = exclusive;

    log::info!(
        "Device: {}",
        if config.device_name.is_empty() {
            "default"
        } else {
            &config.device_name
        }
    );
    log::info!("Exclusive mode: {}", exclusive);
    log::info!("Loop: {}", loop_play);

    // Create engine
    let engine = PlaybackEngine::new(config)?;

    // List devices
    let devices = engine.device_manager().lock().unwrap().list_devices()?;
    log::info!("Available devices:");
    for dev in &devices {
        let marker = if dev.is_default { " [default]" } else { "" };
        log::info!("  - {}{}", dev.name, marker);
    }

    // Play the file
    let path = file.to_string_lossy().to_string();
    log::info!("Playing: {}", path);

    let is_cue = file
        .extension()
        .map(|e| e.eq_ignore_ascii_case("cue"))
        .unwrap_or(false);

    if let Some(paths) = &playlist_paths {
        log::info!("--playlist given; ignoring positional file argument");
        for p in paths {
            let _ = engine.enqueue(p);
        }
        engine.set_queue_index(0);
        engine.start_playback()?;
        log::info!("Playing playlist ({} entries)", paths.len());
    } else if is_cue {
        let track_idx = track.unwrap_or(1).saturating_sub(1);
        engine.play_cue(&path, track_idx)?;
    } else {
        engine.play_file(&path)?;
    }

    // Wait for playback to finish
    loop {
        let state = engine.state();
        log::debug!(
            "State: {:?}, buffer fill: {:.1}%",
            state,
            engine.buffer_fill() * 100.0
        );

        match state {
            PlaybackState::Stopped => {
                if loop_play {
                    log::info!("Looping...");
                    if let Some(paths) = &playlist_paths {
                        for p in paths {
                            let _ = engine.enqueue(p);
                        }
                        engine.set_queue_index(0);
                        engine.start_playback()?;
                    } else if is_cue {
                        let track_idx = track.unwrap_or(1).saturating_sub(1);
                        engine.play_cue(&path, track_idx)?;
                    } else {
                        engine.play_file(&path)?;
                    }
                } else {
                    break;
                }
            }
            PlaybackState::Idle => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            _ => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }

    log::info!("Playback finished");
    Ok(())
}

/// List all audio output devices.
fn cmd_list_devices() -> Result<(), Box<dyn std::error::Error>> {
    let device_manager = DeviceManager::new()?;
    let devices = device_manager.list_devices()?;

    println!("Available audio output devices:");
    println!("{:-<60}", "");

    for dev in &devices {
        let default_marker = if dev.is_default { " [DEFAULT]" } else { "" };
        println!("Device: {}{}", dev.name, default_marker);
        println!("  Max channels: {}", dev.max_channels);
        println!(
            "  Exclusive mode: {}",
            if dev.supports_exclusive { "Yes" } else { "No" }
        );
        print!("  Supported sample rates: ");
        let rates: Vec<String> = dev
            .supported_sample_rates
            .iter()
            .map(|r| format!("{}Hz", r))
            .collect();
        println!("{}", rates.join(", "));
        println!();
    }

    Ok(())
}
