//! Playlist management and persistence.
//!
//! Supports M3U, M3U8, PLS, and XSPF playlist formats.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// A single playlist entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistEntry {
    pub path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub duration: Option<f64>,
}

/// Playlist format types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaylistFormat {
    M3u,
    M3u8,
    Pls,
    Xspf,
}

impl PlaylistFormat {
    /// Detect format from file extension.
    pub fn from_extension(path: &str) -> Option<Self> {
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match ext.as_str() {
            "m3u" => Some(PlaylistFormat::M3u),
            "m3u8" => Some(PlaylistFormat::M3u8),
            "pls" => Some(PlaylistFormat::Pls),
            "xspf" => Some(PlaylistFormat::Xspf),
            _ => None,
        }
    }
}

/// A playlist containing ordered entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub name: String,
    pub entries: Vec<PlaylistEntry>,
    pub format: PlaylistFormat,
}

impl Playlist {
    /// Create a new empty playlist.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            entries: Vec::new(),
            format: PlaylistFormat::M3u8,
        }
    }

    /// Add an entry to the playlist.
    pub fn add(&mut self, entry: PlaylistEntry) {
        self.entries.push(entry);
    }

    /// Remove an entry by index.
    pub fn remove(&mut self, index: usize) -> Option<PlaylistEntry> {
        if index < self.entries.len() {
            Some(self.entries.remove(index))
        } else {
            None
        }
    }

    /// Move an entry from one position to another.
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from < self.entries.len() && to < self.entries.len() {
            let entry = self.entries.remove(from);
            self.entries.insert(to, entry);
        }
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the playlist is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Load a playlist from a file.
    pub fn load(path: &str) -> Result<Self, PlaylistError> {
        let format = PlaylistFormat::from_extension(path)
            .ok_or_else(|| PlaylistError::UnsupportedFormat(path.to_string()))?;

        let content = fs::read_to_string(path)?;

        let entries = match format {
            PlaylistFormat::M3u | PlaylistFormat::M3u8 => parse_m3u(&content)?,
            PlaylistFormat::Pls => parse_pls(&content)?,
            PlaylistFormat::Xspf => parse_xspf(&content)?,
        };

        // Real-world playlists reference tracks relative to the playlist's
        // own directory (`song.flac`, `..\Album\x.flac`) or as `file://` URIs.
        // Resolve every entry against the playlist file's folder.
        let base_dir = Path::new(path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let entries = entries
            .into_iter()
            .map(|e| PlaylistEntry {
                path: resolve_entry_path(&base_dir, &e.path),
                ..e
            })
            .collect();

        let name = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        Ok(Self {
            name,
            entries,
            format,
        })
    }

    /// Save the playlist to a file.
    pub fn save(&self, path: &str) -> Result<(), PlaylistError> {
        let format = PlaylistFormat::from_extension(path).unwrap_or(PlaylistFormat::M3u8);

        let content = match format {
            PlaylistFormat::M3u | PlaylistFormat::M3u8 => self.to_m3u(),
            PlaylistFormat::Pls => self.to_pls(),
            PlaylistFormat::Xspf => self.to_xspf(),
        };

        fs::write(path, content)?;
        Ok(())
    }

    /// Serialize to M3U format.
    pub fn to_m3u(&self) -> String {
        let mut output = String::from("#EXTM3U\n");
        for entry in &self.entries {
            if let Some(duration) = entry.duration {
                let title = entry.title.as_deref().unwrap_or("");
                output.push_str(&format!("#EXTINF:{},{}\n", duration as i64, title));
            }
            output.push_str(&entry.path);
            output.push('\n');
        }
        output
    }

    /// Serialize to PLS format.
    pub fn to_pls(&self) -> String {
        let mut output = String::from("[playlist]\n");
        for (i, entry) in self.entries.iter().enumerate() {
            let idx = i + 1;
            output.push_str(&format!("File{}={}\n", idx, entry.path));
            output.push_str(&format!(
                "Title{}={}\n",
                idx,
                entry.title.as_deref().unwrap_or("")
            ));
            if let Some(duration) = entry.duration {
                output.push_str(&format!("Length{}={}\n", idx, duration as i64));
            }
        }
        output.push_str(&format!("NumberOfEntries={}\n", self.entries.len()));
        output.push_str("Version=2\n");
        output
    }

    /// Serialize to XSPF format.
    pub fn to_xspf(&self) -> String {
        let mut output = String::from(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<playlist version="1" xmlns="http://xspf.org/ns/0/">
  <trackList>
"#,
        );

        for entry in &self.entries {
            output.push_str("    <track>\n");
            output.push_str(&format!(
                "      <location>{}</location>\n",
                xml_escape(&entry.path)
            ));
            if let Some(ref title) = entry.title {
                output.push_str(&format!("      <title>{}</title>\n", xml_escape(title)));
            }
            if let Some(ref artist) = entry.artist {
                output.push_str(&format!(
                    "      <creator>{}</creator>\n",
                    xml_escape(artist)
                ));
            }
            if let Some(duration) = entry.duration {
                output.push_str(&format!(
                    "      <duration>{}</duration>\n",
                    (duration * 1000.0) as i64
                ));
            }
            output.push_str("    </track>\n");
        }

        output.push_str("  </trackList>\n</playlist>\n");
        output
    }
}

/// Resolve a playlist entry against the playlist file's directory.
///
/// Remote URLs are kept as-is; `file://` URIs are percent-decoded into local
/// paths; relative paths are joined onto `base_dir`. Plain absolute paths
/// pass through untouched (they are NOT percent-decoded — a literal `%20`
/// in a file name must survive).
fn resolve_entry_path(base_dir: &Path, raw: &str) -> String {
    let entry = raw.trim();
    if entry.is_empty() {
        return raw.to_string();
    }
    let lower = entry.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mms://")
        || lower.starts_with("icecast://")
    {
        return raw.to_string();
    }
    if let Some(rest) = lower.starts_with("file://").then(|| &entry[7..]) {
        let mut p = percent_decode(rest);
        // file:///C:/Music/... → /C:/... on Windows; drop the leading slash
        // that precedes the drive letter.
        #[cfg(windows)]
        if p.starts_with('/') && p.as_bytes().get(2) == Some(&b':') {
            p.remove(0);
        }
        return p;
    }
    if Path::new(entry).is_absolute() {
        return entry.to_string();
    }
    base_dir.join(entry).to_string_lossy().into_owned()
}

/// Minimal percent-decoding for URI-embedded paths.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse M3U/M3U8 playlist content.
pub fn parse_m3u(content: &str) -> Result<Vec<PlaylistEntry>, PlaylistError> {
    let mut entries = Vec::new();
    let mut current_title: Option<String> = None;
    let mut current_duration: Option<f64> = None;

    for line in content.lines() {
        let line = line.trim();

        if line.is_empty() || line == "#EXTM3U" {
            continue;
        }

        if line.starts_with("#EXTINF:") {
            let info = line.trim_start_matches("#EXTINF:");
            if let Some(comma_pos) = info.find(',') {
                let dur_str = &info[..comma_pos];
                current_duration = dur_str.parse::<f64>().ok();
                current_title = Some(info[comma_pos + 1..].to_string());
            }
        } else if !line.starts_with('#') {
            entries.push(PlaylistEntry {
                path: line.to_string(),
                title: current_title.take(),
                artist: None,
                duration: current_duration.take(),
            });
        }
    }

    Ok(entries)
}

/// Parse PLS playlist content.
pub fn parse_pls(content: &str) -> Result<Vec<PlaylistEntry>, PlaylistError> {
    let mut entries = Vec::new();
    let mut file_map: std::collections::HashMap<usize, PlaylistEntry> =
        std::collections::HashMap::new();

    for line in content.lines() {
        let line = line.trim();

        if line.is_empty() || line == "[playlist]" {
            continue;
        }

        if let Some(eq_pos) = line.find('=') {
            let key = &line[..eq_pos];
            let value = &line[eq_pos + 1..];

            if let Some(idx) = extract_number(key, "File") {
                file_map
                    .entry(idx)
                    .or_insert_with(|| PlaylistEntry {
                        path: String::new(),
                        title: None,
                        artist: None,
                        duration: None,
                    })
                    .path = value.to_string();
            } else if let Some(idx) = extract_number(key, "Title") {
                file_map
                    .entry(idx)
                    .or_insert_with(|| PlaylistEntry {
                        path: String::new(),
                        title: None,
                        artist: None,
                        duration: None,
                    })
                    .title = Some(value.to_string());
            } else if let Some(idx) = extract_number(key, "Length") {
                file_map
                    .entry(idx)
                    .or_insert_with(|| PlaylistEntry {
                        path: String::new(),
                        title: None,
                        artist: None,
                        duration: None,
                    })
                    .duration = value.parse::<f64>().ok();
            }
        }
    }

    for i in 1..=file_map.len() {
        if let Some(entry) = file_map.remove(&i) {
            if !entry.path.is_empty() {
                entries.push(entry);
            }
        }
    }

    Ok(entries)
}

/// Parse XSPF playlist content.
pub fn parse_xspf(content: &str) -> Result<Vec<PlaylistEntry>, PlaylistError> {
    let mut entries = Vec::new();

    // Simple XML parsing for XSPF playlists
    let track_sections: Vec<&str> = content.split("<track>").skip(1).collect();

    for section in track_sections {
        let section = section.split("</track>").next().unwrap_or("");

        let path = extract_xml_tag(section, "location").unwrap_or_default();
        if path.is_empty() {
            continue;
        }

        let title = extract_xml_tag(section, "title").map(|s| xml_unescape(&s));
        let artist = extract_xml_tag(section, "creator").map(|s| xml_unescape(&s));
        let duration = extract_xml_tag(section, "duration")
            .and_then(|d| d.parse::<f64>().ok())
            .map(|d| d / 1000.0);

        entries.push(PlaylistEntry {
            path,
            title,
            artist,
            duration,
        });
    }

    Ok(entries)
}

/// Extract a number from a key prefix (e.g., "File3" -> Some(3)).
fn extract_number(key: &str, prefix: &str) -> Option<usize> {
    if let Some(stripped) = key.strip_prefix(prefix) {
        stripped.parse::<usize>().ok()
    } else {
        None
    }
}

/// Extract the content of an XML tag.
fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);

    if let Some(start) = xml.find(&open) {
        let start = start + open.len();
        if let Some(end) = xml[start..].find(&close) {
            return Some(xml[start..start + end].to_string());
        }
    }
    None
}

/// Escape special XML characters.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Unescape XML entities.
fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Errors for playlist operations.
#[derive(Debug, thiserror::Error)]
pub enum PlaylistError {
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Parse error: {0}")]
    Parse(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_m3u() {
        let content = "#EXTM3U\n#EXTINF:300,Test Song\n/path/to/song.flac\n";
        let entries = parse_m3u(content).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "/path/to/song.flac");
        assert_eq!(entries[0].title.as_deref(), Some("Test Song"));
        assert_eq!(entries[0].duration, Some(300.0));
    }

    #[test]
    fn test_parse_pls() {
        let content = "[playlist]\nFile1=/music/song.flac\nTitle1=Test Song\nLength1=300\nNumberOfEntries=1\nVersion=2\n";
        let entries = parse_pls(content).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "/music/song.flac");
        assert_eq!(entries[0].title.as_deref(), Some("Test Song"));
        assert_eq!(entries[0].duration, Some(300.0));
    }

    #[test]
    fn test_playlist_roundtrip_m3u() {
        let mut playlist = Playlist::new("Test");
        playlist.add(PlaylistEntry {
            path: "/music/song.flac".into(),
            title: Some("Test Song".into()),
            artist: None,
            duration: Some(300.0),
        });

        let m3u = playlist.to_m3u();
        let parsed = parse_m3u(&m3u).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, "/music/song.flac");
    }
}
