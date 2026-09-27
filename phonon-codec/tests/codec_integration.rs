//! Integration tests for the phonon-codec crate.
//!
//! Tests format decoding, metadata extraction, CUE parsing,
//! and DSD support.

use phonon_codec::*;

// ============================================================
// Test helpers
// ============================================================

/// Generate a sine wave tone for testing.
/// Frequency in Hz, sample_rate in Hz, duration in seconds.
fn generate_sine(frequency: f64, sample_rate: u32, duration: f64, channels: u16) -> Vec<f32> {
    let num_samples = (sample_rate as f64 * duration) as usize * channels as usize;
    let mut samples = Vec::with_capacity(num_samples);
    for i in 0..num_samples / channels as usize {
        let t = i as f64 / sample_rate as f64;
        let value = (2.0 * std::f64::consts::PI * frequency * t).sin() as f32;
        for _ in 0..channels {
            samples.push(value);
        }
    }
    samples
}

/// Generate a WAV file in memory (minimal 44-byte header + PCM data).
fn generate_wav_bytes(samples: &[f32], sample_rate: u32, channels: u16) -> Vec<u8> {
    let data_size = samples.len() * 2; // 16-bit PCM
    let file_size = 44 + data_size;

    let mut wav = Vec::with_capacity(file_size);

    // RIFF header
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(file_size as u32 - 8).to_le_bytes());
    wav.extend_from_slice(b"WAVE");

    // fmt chunk
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * channels as u32 * 2;
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = channels * 2;
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    // data chunk
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_size as u32).to_le_bytes());

    // PCM data (float -> i16)
    for sample in samples {
        let clamped = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
        wav.extend_from_slice(&clamped.to_le_bytes());
    }

    wav
}

// ============================================================
// AudioFormat tests
// ============================================================

#[test]
fn test_audio_format_from_extension_all() {
    assert_eq!(AudioFormat::from_extension("flac"), AudioFormat::Flac);
    assert_eq!(AudioFormat::from_extension("FLAC"), AudioFormat::Flac);
    assert_eq!(AudioFormat::from_extension("alac"), AudioFormat::Alac);
    assert_eq!(AudioFormat::from_extension("m4a"), AudioFormat::Alac);
    assert_eq!(AudioFormat::from_extension("wav"), AudioFormat::Wav);
    assert_eq!(AudioFormat::from_extension("wave"), AudioFormat::Wav);
    assert_eq!(AudioFormat::from_extension("mp3"), AudioFormat::Mp3);
    assert_eq!(AudioFormat::from_extension("aac"), AudioFormat::Aac);
    assert_eq!(AudioFormat::from_extension("dsf"), AudioFormat::Dsf);
    assert_eq!(AudioFormat::from_extension("dff"), AudioFormat::Dff);
    assert_eq!(AudioFormat::from_extension("ogg"), AudioFormat::Unknown);
    assert_eq!(AudioFormat::from_extension(""), AudioFormat::Unknown);
}

#[test]
fn test_audio_format_from_mime() {
    assert_eq!(AudioFormat::from_mime("audio/flac"), AudioFormat::Flac);
    assert_eq!(AudioFormat::from_mime("audio/mpeg"), AudioFormat::Mp3);
    assert_eq!(AudioFormat::from_mime("audio/wav"), AudioFormat::Wav);
    assert_eq!(AudioFormat::from_mime("audio/x-dsf"), AudioFormat::Dsf);
    assert_eq!(AudioFormat::from_mime("video/mp4"), AudioFormat::Unknown);
}

// ============================================================
// ChannelLayout tests
// ============================================================

#[test]
fn test_channel_layout_all() {
    let cases = vec![
        (ChannelLayout::Mono, 1),
        (ChannelLayout::Stereo, 2),
        (ChannelLayout::Layout2_1, 3),
        (ChannelLayout::Layout3_0, 3),
        (ChannelLayout::Layout3_1, 4),
        (ChannelLayout::Layout4_0, 4),
        (ChannelLayout::Layout4_1, 5),
        (ChannelLayout::Layout5_0, 5),
        (ChannelLayout::Layout5_1, 6),
        (ChannelLayout::Layout6_1, 7),
        (ChannelLayout::Layout7_1, 8),
    ];

    for (layout, expected_count) in cases {
        assert_eq!(
            layout.channel_count(),
            expected_count,
            "Wrong count for {:?}",
            layout
        );
    }
}

#[test]
fn test_channel_layout_from_count() {
    assert_eq!(ChannelLayout::from_count(1), ChannelLayout::Mono);
    assert_eq!(ChannelLayout::from_count(2), ChannelLayout::Stereo);
    assert_eq!(ChannelLayout::from_count(6), ChannelLayout::Layout5_1);
    assert_eq!(ChannelLayout::from_count(8), ChannelLayout::Layout7_1);
    // Custom for > 8
    let custom = ChannelLayout::from_count(10);
    assert_eq!(custom.channel_count(), 10);
}

#[test]
fn test_channel_layout_names() {
    let stereo = ChannelLayout::Stereo.channel_names();
    assert_eq!(stereo, vec!["FrontLeft", "FrontRight"]);

    let mono = ChannelLayout::Mono.channel_names();
    assert_eq!(mono, vec!["FrontCenter"]);

    let surround = ChannelLayout::Layout5_1.channel_names();
    assert_eq!(surround.len(), 6);
    assert!(surround.contains(&"LFE".to_string()));
}

// ============================================================
// CUE parsing tests
// ============================================================

#[test]
fn test_cue_parse_complete() {
    let cue = r#"
REM GENRE Classical
REM DATE 1995
PERFORMER "London Symphony Orchestra"
TITLE "Greatest Hits"
FILE "album.flac" WAVE
  TRACK 01 AUDIO
    TITLE "Symphony No.5 - I. Allegro"
    PERFORMER "Beethoven"
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    TITLE "Symphony No.5 - II. Andante"
    INDEX 01 07:25:33
  TRACK 03 AUDIO
    TITLE "Symphony No.5 - III. Scherzo"
    INDEX 01 15:48:00
  TRACK 04 AUDIO
    TITLE "Symphony No.5 - IV. Allegro"
    INDEX 01 20:30:45
"#;

    let sheet = cue::parse_cue(cue).unwrap();
    assert_eq!(sheet.title.as_deref(), Some("Greatest Hits"));
    assert_eq!(
        sheet.performer.as_deref(),
        Some("London Symphony Orchestra")
    );
    assert_eq!(sheet.date.as_deref(), Some("1995"));
    assert_eq!(sheet.genre.as_deref(), Some("Classical"));
    assert_eq!(sheet.file.as_deref(), Some("album.flac"));
    assert_eq!(sheet.tracks.len(), 4);

    // Verify track 1
    assert_eq!(sheet.tracks[0].index, 1);
    assert_eq!(
        sheet.tracks[0].title.as_deref(),
        Some("Symphony No.5 - I. Allegro")
    );
    assert_eq!(sheet.tracks[0].start_time, 0.0);
    assert!(sheet.tracks[0].duration.is_some());

    // Verify track 2 start time: 7:25 + 33/75 = 445.44 seconds
    assert!((sheet.tracks[1].start_time - 445.44).abs() < 0.01);

    // Verify track 3 start time: 15:48 = 948 seconds
    assert!((sheet.tracks[2].start_time - 948.0).abs() < 0.01);

    // Verify track 4 start time: 20:30 + 45/75 = 1230.6 seconds
    assert!((sheet.tracks[3].start_time - 1230.6).abs() < 0.01);
}

#[test]
fn test_cue_parse_minimal() {
    let cue = r#"
FILE "test.flac" WAVE
  TRACK 01 AUDIO
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    INDEX 01 05:00:00
"#;

    let sheet = cue::parse_cue(cue).unwrap();
    assert_eq!(sheet.tracks.len(), 2);
    assert!(sheet.tracks[0].title.is_none());
    assert_eq!(sheet.tracks[1].start_time, 300.0);
}

#[test]
fn test_cue_parse_empty() {
    let sheet = cue::parse_cue("").unwrap();
    assert_eq!(sheet.tracks.len(), 0);
}

#[test]
fn test_cue_parse_comments_ignored() {
    let cue = r#"
// This is a comment
FILE "test.flac" WAVE
  TRACK 01 AUDIO
    INDEX 01 00:00:00
"#;

    let sheet = cue::parse_cue(cue).unwrap();
    assert_eq!(sheet.tracks.len(), 1);
}

#[test]
fn test_cue_multiple_performers() {
    let cue = r#"
PERFORMER "Various Artists"
TITLE "Compilation"
FILE "comp.flac" WAVE
  TRACK 01 AUDIO
    TITLE "Track 1"
    PERFORMER "Artist A"
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    TITLE "Track 2"
    PERFORMER "Artist B"
    INDEX 01 04:00:00
"#;

    let sheet = cue::parse_cue(cue).unwrap();
    assert_eq!(sheet.tracks[0].performer.as_deref(), Some("Artist A"));
    assert_eq!(sheet.tracks[1].performer.as_deref(), Some("Artist B"));
}

#[test]
fn test_cue_parse_time_two_digit() {
    // "MM:SS" format (no frames)
    let cue = r#"
FILE "test.flac" WAVE
  TRACK 01 AUDIO
    INDEX 01 01:30
  TRACK 02 AUDIO
    INDEX 01 03:45
"#;

    let sheet = cue::parse_cue(cue).unwrap();
    assert!((sheet.tracks[0].start_time - 90.0).abs() < 0.01);
    assert!((sheet.tracks[1].start_time - 225.0).abs() < 0.01);
}

// ============================================================
// ReplayGain tests
// ============================================================

#[test]
fn test_replay_gain_extraction_full() {
    let tags = vec![
        ("REPLAYGAIN_TRACK_GAIN".to_string(), "-7.50 dB".to_string()),
        ("REPLAYGAIN_TRACK_PEAK".to_string(), "0.891251".to_string()),
        ("REPLAYGAIN_ALBUM_GAIN".to_string(), "-8.20 dB".to_string()),
        ("REPLAYGAIN_ALBUM_PEAK".to_string(), "0.944061".to_string()),
    ];

    let rg = metadata::extract_replay_gain(&tags);
    assert!((rg.track_gain.unwrap() - (-7.50)).abs() < 0.01);
    assert!((rg.track_peak.unwrap() - 0.891251).abs() < 0.0001);
    assert!((rg.album_gain.unwrap() - (-8.20)).abs() < 0.01);
    assert!((rg.album_peak.unwrap() - 0.944061).abs() < 0.0001);
}

#[test]
fn test_replay_gain_extraction_partial() {
    let tags = vec![("REPLAYGAIN_TRACK_GAIN".to_string(), "-3.00 dB".to_string())];

    let rg = metadata::extract_replay_gain(&tags);
    assert!((rg.track_gain.unwrap() - (-3.00)).abs() < 0.01);
    assert!(rg.track_peak.is_none());
    assert!(rg.album_gain.is_none());
}

#[test]
fn test_replay_gain_extraction_positive() {
    let tags = vec![("REPLAYGAIN_TRACK_GAIN".to_string(), "+2.50 dB".to_string())];

    let rg = metadata::extract_replay_gain(&tags);
    assert!((rg.track_gain.unwrap() - 2.50).abs() < 0.01);
}

#[test]
fn test_replay_gain_no_db_suffix() {
    let tags = vec![("REPLAYGAIN_TRACK_GAIN".to_string(), "-5.00".to_string())];

    let rg = metadata::extract_replay_gain(&tags);
    assert!((rg.track_gain.unwrap() - (-5.00)).abs() < 0.01);
}

// ============================================================
// TrackMetadata tests
// ============================================================

#[test]
fn test_track_metadata_default() {
    let meta = TrackMetadata::default();
    assert!(meta.title.is_none());
    assert!(meta.duration.is_none());
    assert!(meta.sample_rate.is_none());
}

#[test]
fn test_track_metadata_serialization() {
    let meta = TrackMetadata {
        title: Some("Test Song".into()),
        artist: Some("Test Artist".into()),
        album: Some("Test Album".into()),
        duration: Some(300.0),
        sample_rate: Some(44100),
        channels: Some(2),
        bit_depth: Some(24),
        ..Default::default()
    };

    let json = serde_json::to_string(&meta).unwrap();
    let parsed: TrackMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.title.as_deref(), Some("Test Song"));
    assert_eq!(parsed.sample_rate, Some(44100));
    assert_eq!(parsed.bit_depth, Some(24));
}

// ============================================================
// PcmBuffer / AudioFrame tests
// ============================================================

#[test]
fn test_audio_frame_creation() {
    let frame = AudioFrame {
        samples: vec![0.5f32, -0.5, 0.25, -0.25],
        sample_rate: 44100,
        channels: 2,
        channel_layout: ChannelLayout::Stereo,
        timestamp: 0.0,
    };

    assert_eq!(frame.samples.len(), 4);
    assert_eq!(frame.sample_rate, 44100);
    assert_eq!(frame.channels, 2);
}

#[test]
fn test_pcm_buffer_properties() {
    let buf = PcmBuffer {
        samples: vec![0.0f32; 1024],
        sample_rate: 48000,
        channels: 2,
        channel_layout: ChannelLayout::Stereo,
        frame_count: 512,
    };

    assert_eq!(buf.frame_count, 512);
    assert_eq!(buf.samples.len(), 1024);
    assert_eq!(buf.sample_rate, 48000);
}

// ============================================================
// Sine wave generation test (utility verification)
// ============================================================

#[test]
fn test_sine_generation() {
    let samples = generate_sine(440.0, 44100, 1.0, 1);
    assert_eq!(samples.len(), 44100);

    // First sample should be near 0
    assert!(samples[0].abs() < 0.01);

    // Check that we have a full cycle every 44100/440 ≈ 100.23 samples
    let period = 44100.0 / 440.0;
    let quarter_period = (period / 4.0) as usize;

    // At quarter period, sine should be near 1.0
    assert!(samples[quarter_period] > 0.9);

    // At half period, sine should be near 0 again
    let half_period = (period / 2.0) as usize;
    assert!(samples[half_period].abs() < 0.1);
}

#[test]
fn test_sine_stereo() {
    let samples = generate_sine(1000.0, 48000, 0.5, 2);
    // 0.5 sec * 48000 * 2 channels = 48000 samples
    assert_eq!(samples.len(), 48000);

    // Left and right should be identical (same tone)
    for i in (0..samples.len()).step_by(2) {
        assert_eq!(samples[i], samples[i + 1]);
    }
}

// ============================================================
// WAV generation test
// ============================================================

#[test]
fn test_wav_generation() {
    let samples = generate_sine(440.0, 44100, 0.1, 1);
    let wav = generate_wav_bytes(&samples, 44100, 1);

    // Check RIFF header
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(&wav[12..16], b"fmt ");
    assert_eq!(&wav[36..40], b"data");

    // Check sample rate
    let sr = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
    assert_eq!(sr, 44100);

    // Check channels
    let ch = u16::from_le_bytes([wav[22], wav[23]]);
    assert_eq!(ch, 1);
}

// ============================================================
// DSD error handling tests
// ============================================================

#[test]
fn test_dsd_error_display() {
    let err = dsd::DsdError::UnsupportedFormat;
    assert_eq!(err.to_string(), "Unsupported format");

    let err = dsd::DsdError::InvalidHeader("Bad DSF".into());
    assert!(err.to_string().contains("Bad DSF"));
}

// ============================================================
// CodecError tests
// ============================================================

#[test]
fn test_codec_error_display() {
    let err = decoder::CodecError::EndOfStream;
    assert_eq!(err.to_string(), "End of stream");

    let err = decoder::CodecError::UnsupportedFormat("ogg".into());
    assert!(err.to_string().contains("ogg"));
}
