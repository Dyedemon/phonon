//! CUE sheet parser.
//!
//! Parses CUE files and provides virtual track splitting.

use crate::types::*;

/// Parse a CUE file content and return the CUE sheet metadata.
pub fn parse_cue(content: &str) -> Result<CueSheet, CueError> {
    let mut sheet = CueSheet::default();
    let mut current_track: Option<CueTrack> = None;

    for line in content.lines() {
        let line = line.trim();

        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        // Split on any whitespace: some rippers emit tab-separated commands
        // (`INDEX\t01\t00:00:00`), which a space-only split mis-parses.
        let parts: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
        if parts.is_empty() {
            continue;
        }

        let command = parts[0].to_uppercase();
        let args = parts.get(1).unwrap_or(&"").trim();

        match command.as_str() {
            "TITLE" => {
                let value = unquote(args);
                if current_track.is_some() {
                    if let Some(ref mut t) = current_track {
                        t.title = Some(value);
                    }
                } else {
                    sheet.title = Some(value);
                }
            }
            "PERFORMER" => {
                let value = unquote(args);
                if current_track.is_some() {
                    if let Some(ref mut t) = current_track {
                        t.performer = Some(value);
                    }
                } else {
                    sheet.performer = Some(value);
                }
            }
            "REM" => {
                let rem_parts: Vec<&str> = args.splitn(2, ' ').collect();
                if rem_parts.len() >= 2 {
                    match rem_parts[0].to_uppercase().as_str() {
                        "DATE" => sheet.date = Some(rem_parts[1].trim_matches('"').to_string()),
                        "GENRE" => sheet.genre = Some(rem_parts[1].trim_matches('"').to_string()),
                        _ => {}
                    }
                }
            }
            "FILE" => {
                // FILE line format: FILE "filename" TYPE
                // Extract the quoted filename, ignoring the TYPE suffix
                let filename = args
                    .split('"')
                    .nth(1)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| args.to_string());
                sheet.file = Some(filename);
            }
            "TRACK" => {
                // Save previous track
                if let Some(track) = current_track.take() {
                    sheet.tracks.push(track);
                }

                let track_parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();
                let track_index: u32 = track_parts
                    .first()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);

                current_track = Some(CueTrack {
                    index: track_index,
                    title: None,
                    performer: None,
                    start_time: 0.0,
                    duration: None,
                });
            }
            "INDEX" => {
                    let index_parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();
                if index_parts.len() >= 2 {
                    let index_num: u32 = index_parts[0].parse().unwrap_or(0);
                    if index_num == 1 {
                        let time = parse_time(index_parts[1]);
                        if let Some(ref mut t) = current_track {
                            t.start_time = time;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Save last track
    if let Some(track) = current_track {
        sheet.tracks.push(track);
    }

    // Calculate durations
    let track_count = sheet.tracks.len();
    let start_times: Vec<f64> = sheet.tracks.iter().map(|t| t.start_time).collect();
    for (i, track) in sheet.tracks.iter_mut().enumerate() {
        let start = track.start_time;
        if i + 1 < track_count {
            track.duration = Some((start_times[i + 1] - start).max(0.0));
        } else {
            // Last track: the caller fills in the duration from the actual
            // audio file length (None here so that fallback can fire).
            track.duration = None;
        }
    }

    Ok(sheet)
}

/// Parse a time string like "MM:SS:FF" into seconds.
fn parse_time(time_str: &str) -> f64 {
    let parts: Vec<&str> = time_str.split(':').collect();
    match parts.len() {
        2 => {
            let minutes: f64 = parts[0].parse().unwrap_or(0.0);
            let seconds: f64 = parts[1].parse().unwrap_or(0.0);
            minutes * 60.0 + seconds
        }
        3 => {
            let minutes: f64 = parts[0].parse().unwrap_or(0.0);
            let seconds: f64 = parts[1].parse().unwrap_or(0.0);
            let frames: f64 = parts[2].parse().unwrap_or(0.0);
            minutes * 60.0 + seconds + frames / 75.0
        }
        _ => 0.0,
    }
}

/// Remove surrounding quotes from a string.
fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 {
        let first = s.chars().next().unwrap();
        let last = s.chars().last().unwrap();
        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

/// Errors that can occur during CUE parsing.
#[derive(Debug, thiserror::Error)]
pub enum CueError {
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_cue() {
        let cue_content = r#"
TITLE "Test Album"
PERFORMER "Test Artist"
FILE "test.flac" WAVE
  TRACK 01 AUDIO
    TITLE "Track One"
    PERFORMER "Artist One"
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    TITLE "Track Two"
    INDEX 01 03:45:00
  TRACK 03 AUDIO
    TITLE "Track Three"
    INDEX 01 08:12:50
"#;

        let sheet = parse_cue(cue_content).unwrap();
        assert_eq!(sheet.title.as_deref(), Some("Test Album"));
        assert_eq!(sheet.performer.as_deref(), Some("Test Artist"));
        assert_eq!(sheet.file.as_deref(), Some("test.flac"));
        assert_eq!(sheet.tracks.len(), 3);

        assert_eq!(sheet.tracks[0].title.as_deref(), Some("Track One"));
        assert_eq!(sheet.tracks[0].start_time, 0.0);

        assert_eq!(sheet.tracks[1].title.as_deref(), Some("Track Two"));
        assert!((sheet.tracks[1].start_time - 225.0).abs() < 0.01);

        assert_eq!(sheet.tracks[2].title.as_deref(), Some("Track Three"));
        assert!((sheet.tracks[2].start_time - 492.0 - 50.0 / 75.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_time() {
        assert!((parse_time("03:45:00") - 225.0).abs() < 0.01);
        assert!((parse_time("00:00:00") - 0.0).abs() < 0.01);
        assert!((parse_time("01:30") - 90.0).abs() < 0.01);
    }
}
