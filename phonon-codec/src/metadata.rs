//! Metadata extraction for audio files.
//!
//! Extracts tags, album art, ReplayGain, and other metadata from audio files.

use std::fs::File;

use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::types::*;

/// Extract metadata from an audio file.
pub fn extract_metadata(path: &str) -> Result<TrackMetadata, crate::decoder::CodecError> {
    let decoder = crate::AudioDecoder::open(path)?;
    Ok(decoder.metadata().clone())
}

/// Extract album art from an audio file using Symphonia's metadata system.
///
/// Accesses the format reader's metadata to find embedded pictures
/// (FLAC Picture blocks, ID3v2 APIC frames, etc.).
pub fn extract_album_art(path: &str) -> Option<AlbumArt> {
    let src = File::open(path).ok()?;

    let mss = MediaSourceStream::new(Box::new(src), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
    {
        hint.with_extension(ext);
    }

    let format_opts = FormatOptions::default();
    let metadata_opts = MetadataOptions::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &metadata_opts)
        .ok()?;

    let mut format = probed.format;

    // Check current metadata log for embedded visuals (album art)
    let metadata = format.metadata();
    if let Some(log) = metadata.current() {
        if let Some(visual) = log.visuals().first() {
            return Some(AlbumArt {
                data: visual.data.to_vec(),
                mime_type: visual.media_type.clone(),
            });
        }
    }

    None
}

/// Extract lyrics from an audio file's metadata.
///
/// Checks for:
/// - ID3v2 USLT (unsynced lyrics) frames
/// - Vorbis Comments LYRICS tag
/// - Other tag-based lyrics keys
///
/// If the text looks like LRC format (starts with `[mm:ss`), it will be parsed
/// into synced lyrics. Otherwise, it's returned as unsynced text.
pub fn extract_lyrics(path: &str) -> LyricData {
    let mut result = LyricData::None;

    // Try to extract lyrics from audio file metadata
    if let Ok(src) = File::open(path) {
        let mss = MediaSourceStream::new(Box::new(src), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
        {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();

        if let Ok(probed) =
            symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)
        {
            let mut format = probed.format;
            let metadata = format.metadata();

            // Try tag-based lyrics first (Vorbis Comments, some ID3v2)
            let tag_keys = &["LYRICS", "UNSYNCED_LYRICS", "UNSYNCEDLYRICS", "USLT"];
            if let Some(current) = metadata.current() {
                for tag in current.tags() {
                    let key_upper = tag.key.to_uppercase();
                    if tag_keys.iter().any(|k| key_upper.contains(k)) {
                        let text = tag.value.to_string();
                        result = parse_lyrics(&text);
                        break;
                    }
                }
                // Also check visuals for unsynced lyrics (ID3v2 USLT frames)
                if result.is_none() {
                    for visual in current.visuals() {
                        if let Some(tag) = visual.tags.iter().find(|t| {
                            let key = t.key.to_uppercase();
                            key.contains("UNSYNCED")
                                || key.contains("USLT")
                                || key.contains("LYRICS")
                        }) {
                            let text = tag.value.to_string();
                            result = parse_lyrics(&text);
                            break;
                        }
                        if !visual.media_type.is_empty() {
                            let mt = visual.media_type.to_lowercase();
                            if mt.contains("text") || mt.contains("lyric") {
                                if let Ok(text) = String::from_utf8(visual.data.to_vec()) {
                                    if !text.trim().is_empty() {
                                        result = parse_lyrics(&text);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback: if nothing found in metadata, try an external .lrc file
    if result.is_none() {
        result = try_load_lrc_file(path);
    }

    result
}

/// Try to load lyrics from an external .lrc file next to the audio file.
/// Looks for `song.lrc` in the same directory as `song.mp3`.
/// Handles UTF-8 and GBK encodings (common for Chinese LRC files).
fn try_load_lrc_file(audio_path: &str) -> LyricData {
    let audio = std::path::Path::new(audio_path);
    let lrc_path = audio.with_extension("lrc");
    read_lrc_file(&lrc_path)
}

/// Directly read and parse an LRC file at the given path.
/// Supports UTF-8 and GBK encodings.
pub fn load_lrc_file(path: &str) -> LyricData {
    read_lrc_file(std::path::Path::new(path))
}

/// Parse raw LRC text from an external source (e.g., online search).
/// Directly parses the text without file I/O.
pub fn parse_lyrics_for_search(text: &str) -> LyricData {
    parse_lyrics(text)
}

fn read_lrc_file(path: &std::path::Path) -> LyricData {
    use std::io::Read;

    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return LyricData::None,
    };

    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() || bytes.is_empty() {
        return LyricData::None;
    }

    // Try UTF-8 first
    if let Ok(text) = String::from_utf8(bytes.clone()) {
        let trimmed = text.trim().to_string();
        if !trimmed.is_empty() {
            return parse_lyrics(&trimmed);
        }
    }

    // Fallback: GBK (common for Chinese LRC files)
    let (text, _, had_errors) = encoding_rs::GBK.decode(&bytes);
    let trimmed = text.trim().to_string();
    if !had_errors && !trimmed.is_empty() {
        return parse_lyrics(&trimmed);
    }

    LyricData::None
}

/// Parse lyrics text: auto-detect LRC format vs plain text.
/// Supports word-level timestamps in enhanced LRC format:
///   [mm:ss.xx] <mm:ss.xx>word1 <mm:ss.xx>word2 ...
fn parse_lyrics(text: &str) -> LyricData {
    // Check if it looks like LRC — first non-empty line should start with a timestamp
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return LyricData::None;
    }

    // Try to parse as LRC
    let mut lines: Vec<LyricLine> = Vec::new();
    let mut has_timestamps = false;

    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Parse LRC timestamp: [mm:ss.xx] or [mm:ss]
        if let Some(rest) = line.strip_prefix('[') {
            if let Some(bracket_pos) = rest.find(']') {
                let time_str = &rest[..bracket_pos];
                let after = rest[bracket_pos + 1..].trim();

                if let Some(time_ms) = parse_lrc_timestamp(time_str) {
                    if !after.is_empty() {
                        has_timestamps = true;
                        // Try to parse word-level timestamps: <mm:ss.xx>word
                        let words = parse_word_timestamps(after, time_ms);
                        lines.push(LyricLine {
                            time_ms,
                            text: if words.is_empty() {
                                after.to_string()
                            } else {
                                words
                                    .iter()
                                    .map(|w| w.text.as_str())
                                    .collect::<Vec<_>>()
                                    .join("")
                            },
                            words,
                        });
                    }
                }
            }
        }
    }

    if has_timestamps && !lines.is_empty() {
        // Sort by timestamp
        lines.sort_by_key(|l| l.time_ms);
        // Auto-distribute word timings for lines without word-level data
        auto_distribute_words(&mut lines);
        LyricData::Synced(lines)
    } else {
        // Return as unsynced plain text
        LyricData::Unsynced(trimmed.to_string())
    }
}

/// Parse word-level timestamps from enhanced LRC text.
/// Format: `<mm:ss.xx>word1 <mm:ss.xx>word2 ...` or plain text without word timestamps.
/// Returns empty Vec if no word timestamps found.
fn parse_word_timestamps(after: &str, _line_time_ms: u64) -> Vec<LyricWord> {
    let mut words = Vec::new();
    let mut remaining = after;
    let mut found_word_ts = false;

    while let Some(rest) = remaining.strip_prefix('<') {
        if let Some(bracket_pos) = rest.find('>') {
            let time_str = &rest[..bracket_pos];
            let after_bracket = &rest[bracket_pos + 1..];
            if let Some(word_time_ms) = parse_lrc_timestamp(time_str) {
                found_word_ts = true;
                // Find the next '<' to get the word text
                if let Some(next_lt) = after_bracket.find('<') {
                    let word_text = after_bracket[..next_lt].trim().to_string();
                    if !word_text.is_empty() {
                        words.push(LyricWord {
                            time_ms: word_time_ms,
                            text: word_text,
                        });
                    }
                    remaining = &after_bracket[next_lt..];
                } else {
                    // Last word — takes the rest
                    let word_text = after_bracket.trim().to_string();
                    if !word_text.is_empty() {
                        words.push(LyricWord {
                            time_ms: word_time_ms,
                            text: word_text,
                        });
                    }
                    break;
                }
            } else {
                break;
            }
        } else {
            break;
        }
    }

    if found_word_ts {
        words
    } else {
        Vec::new()
    }
}

/// Auto-distribute word timings for lines that don't have word-level timestamps.
/// Splits text by CJK characters and spaces, distributing time evenly within the line duration.
fn auto_distribute_words(lines: &mut [LyricLine]) {
    let total = lines.len();
    // Pre-collect time_ms for next-line lookups to avoid borrow conflicts.
    let next_times: Vec<u64> = lines.iter().map(|l| l.time_ms).collect();
    for (i, line) in lines.iter_mut().enumerate() {
        if !line.words.is_empty() {
            continue; // Already has word timestamps
        }

        let line_time = line.time_ms;
        // Estimate duration: gap to next line, or default 3000ms
        let duration = if i + 1 < total {
            let next_time = next_times[i + 1];
            if next_time > line_time {
                next_time - line_time
            } else {
                3000
            }
        } else {
            3000
        };

        let text = &line.text;
        let word_texts = split_words(text);

        if word_texts.len() <= 1 {
            continue; // Single word or empty — no need to split
        }

        // Even distribution: each character/segment gets the same time slice.
        let step = duration / word_texts.len() as u64;
        line.words = word_texts
            .into_iter()
            .enumerate()
            .map(|(j, t)| LyricWord {
                time_ms: line_time + (j as u64 * step),
                text: t,
            })
            .collect();
    }
}

/// Split text into word segments for karaoke highlighting.
/// Splits on CJK characters (one per word), spaces, and groups Latin words.
fn split_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_latin = false;

    for ch in text.chars() {
        let is_cjk = ('\u{4e00}'..='\u{9fff}').contains(&ch)
            || ('\u{3040}'..='\u{309f}').contains(&ch)  // Hiragana
            || ('\u{30a0}'..='\u{30ff}').contains(&ch)  // Katakana
            || ('\u{ac00}'..='\u{d7af}').contains(&ch); // Hangul
        let is_space = ch.is_whitespace();
        let is_latin = ch.is_ascii_alphabetic() || ch.is_ascii_digit() || ch == '\'';

        if is_cjk {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            words.push(ch.to_string());
            in_latin = false;
        } else if is_space {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            in_latin = false;
        } else if is_latin {
            if !in_latin && !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            current.push(ch);
            in_latin = true;
        } else {
            // Punctuation, etc. — append to current or start new
            if in_latin {
                current.push(ch);
            } else {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
                current.push(ch);
                words.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Parse an LRC timestamp string like "01:23.45" or "01:23" into milliseconds.
fn parse_lrc_timestamp(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let minutes: u64 = parts[0].parse().ok()?;
    let seconds_part = parts[1];
    let sub_parts: Vec<&str> = seconds_part.split('.').collect();
    let seconds: u64 = sub_parts[0].parse().ok()?;
    let millis: u64 = if sub_parts.len() > 1 {
        let frac = sub_parts[1];
        // Handle 2-digit (centiseconds) or 3-digit (milliseconds)
        let parsed: u64 = frac.parse().ok()?;
        if frac.len() == 2 {
            parsed * 10
        } else {
            parsed
        }
    } else {
        0
    };
    Some(minutes * 60_000 + seconds * 1000 + millis)
}

/// Extract ReplayGain tags from metadata.
pub fn extract_replay_gain(tags: &[(String, String)]) -> ReplayGain {
    let mut rg = ReplayGain::default();

    for (key, value) in tags {
        match key.to_uppercase().as_str() {
            "REPLAYGAIN_TRACK_GAIN" => {
                rg.track_gain = value.trim_end_matches(" dB").parse().ok();
            }
            "REPLAYGAIN_TRACK_PEAK" => {
                rg.track_peak = value.parse().ok();
            }
            "REPLAYGAIN_ALBUM_GAIN" => {
                rg.album_gain = value.trim_end_matches(" dB").parse().ok();
            }
            "REPLAYGAIN_ALBUM_PEAK" => {
                rg.album_peak = value.parse().ok();
            }
            _ => {}
        }
    }

    rg
}

// ─────────────────────────────────────────────────────────────────
// Plan A Task A3: Partial metadata patch + file write-back stub (RC1).
// ─────────────────────────────────────────────────────────────────

/// Partial metadata patch from the UI edit modal. All fields optional;
/// `Some` = overwrite, `None` = leave unchanged.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PartialMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub composer: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<i32>,
    pub disc_number: Option<i32>,
    /// 0..5 (0 = unrated).
    pub rating: Option<u8>,
    pub cover_bytes: Option<Vec<u8>>,
}

/// Writes `partial` metadata patch back into the actual audio file.
///
/// RC 1 stub: returns `Err` with a human-readable message so the frontend
/// can show the correct banner ("已仅保存到媒体库，文件回写将在 RC2 支持").
/// The database column always succeeds first (caller writes the library
/// BEFORE attempting this); this stub only handles the filesystem side.
pub fn write_metadata_to_file(
    _path: &str,
    _partial: &PartialMeta,
) -> Result<(), crate::decoder::CodecError> {
    Err(crate::decoder::CodecError::NotImplemented(
        "Metadata file write-back is not implemented for RC1. Changes saved to Phonon media library database only. File on-disk will be updated in RC2.".to_string()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replay_gain_extraction() {
        let tags = vec![
            ("REPLAYGAIN_TRACK_GAIN".into(), "-7.50 dB".into()),
            ("REPLAYGAIN_TRACK_PEAK".into(), "0.891251".into()),
            ("REPLAYGAIN_ALBUM_GAIN".into(), "-8.20 dB".into()),
            ("REPLAYGAIN_ALBUM_PEAK".into(), "0.944061".into()),
        ];

        let rg = extract_replay_gain(&tags);
        assert!((rg.track_gain.unwrap() - (-7.50)).abs() < 0.01);
        assert!((rg.track_peak.unwrap() - 0.891251).abs() < 0.0001);
        assert!((rg.album_gain.unwrap() - (-8.20)).abs() < 0.01);
        assert!((rg.album_peak.unwrap() - 0.944061).abs() < 0.0001);
    }

    #[test]
    fn test_replay_gain_empty() {
        let rg = extract_replay_gain(&[]);
        assert!(rg.track_gain.is_none());
        assert!(rg.track_peak.is_none());
    }

    #[test]
    fn test_split_words_cjk() {
        let words = split_words("你好世界");
        assert_eq!(words, vec!["你", "好", "世", "界"]);
    }

    #[test]
    fn test_split_words_mixed() {
        let words = split_words("你好 world 测试");
        assert_eq!(words, vec!["你", "好", "world", "测", "试"]);
    }

    #[test]
    fn test_parse_lyrics_with_words() {
        let lrc = "[00:01.00]你好世界\n[00:05.00]测试 test\n";
        let result = parse_lyrics(lrc);
        match result {
            LyricData::Synced(lines) => {
                assert_eq!(lines.len(), 2);
                // First line: "你好世界" should have 4 words
                assert!(!lines[0].words.is_empty(), "First line should have words");
                assert_eq!(lines[0].words.len(), 4);
                // Second line: "测试 test" should have 3 words
                assert!(!lines[1].words.is_empty(), "Second line should have words");
                assert_eq!(lines[1].words.len(), 3);
            }
            _ => panic!("Expected Synced lyrics"),
        }
    }
}
