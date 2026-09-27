//! Local file source provider.
//!
//! Scans directories for audio files, extracts metadata,
//! and associates CUE sheets with their parent audio files.

use phonon_codec::*;
use std::fs;
use std::path::Path;

use crate::traits::*;

/// Supported audio file extensions.
const SUPPORTED_EXTENSIONS: &[&str] = &[
    "flac", "alac", "m4a", "wav", "wave", "mp3", "aac", "dsf", "dff",
];

/// Local file audio source.
pub struct LocalFileSource {
    name: String,
}

impl LocalFileSource {
    /// Create a new local file source.
    pub fn new() -> Self {
        Self {
            name: "Local Files".into(),
        }
    }

    /// Check if a file extension is supported.
    pub fn is_supported(ext: &str) -> bool {
        SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str())
    }
}

impl Default for LocalFileSource {
    fn default() -> Self {
        Self::new()
    }
}

impl IAudioSource for LocalFileSource {
    fn search(&self, query: &str) -> Result<Vec<SearchResult>, SourceError> {
        let path = Path::new(query);
        if !path.exists() {
            return Err(SourceError::NotFound(query.to_string()));
        }

        let mut results = Vec::new();

        if path.is_file() {
            if let Some(result) = self.file_to_result(path) {
                results.push(result);
            }
        } else if path.is_dir() {
            // Scan directory recursively
            self.scan_directory(path, &mut results)?;
        }

        // Final global deduplication pass — required because:
        // 1. CUE-referenced audio files could still slip through if the
        //    CUE FILE entry and the filesystem entry differ in
        //    representation (relative/absolute, short/long paths, UNC, etc.)
        // 2. Two CUE sheets in the same folder tree can reference the same
        //    audio file and produce duplicate CUE-track stubs.
        //
        // Key by (normalised_uri, cue_track_index) so a standalone audio
        // track (cue_track_index = None) is considered distinct from the
        // same uri used as a CUE source (cue_track_index = Some), while
        // duplicate (uri, cue_track_index) pairs collapse into one.
        let mut seen = std::collections::HashSet::with_capacity(results.len());
        results.retain(|r| {
            let key = (Self::norm_path(Path::new(&r.uri)), r.cue_track_index);
            seen.insert(key)
        });

        Ok(results)
    }

    fn get_metadata(&self, uri: &str) -> Result<TrackMetadata, SourceError> {
        let path = Path::new(uri);
        if !path.exists() {
            return Err(SourceError::NotFound(uri.to_string()));
        }

        let decoder = phonon_codec::AudioDecoder::open(uri)
            .map_err(|e| SourceError::Decode(e.to_string()))?;

        Ok(decoder.metadata().clone())
    }

    fn open_decoder(&self, uri: &str) -> Result<Box<dyn AudioSourceReader>, SourceError> {
        let decoder = phonon_codec::AudioDecoder::open(uri)
            .map_err(|e| SourceError::Decode(e.to_string()))?;

        Ok(Box::new(LocalFileReader {
            decoder,
            _uri: uri.to_string(),
        }))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn source_type(&self) -> SourceType {
        SourceType::LocalFile
    }
}

impl LocalFileSource {
    /// Convert a file path to a SearchResult.
    fn file_to_result(&self, path: &Path) -> Option<SearchResult> {
        let ext = path.extension()?.to_str()?;
        if !Self::is_supported(ext) {
            return None;
        }

        let metadata = phonon_codec::AudioDecoder::open(path)
            .map(|d| d.metadata().clone())
            .unwrap_or_default();

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string();

        Some(SearchResult {
            uri: path.to_string_lossy().to_string(),
            name,
            metadata,
            cue_track_index: None,
        })
    }

    /// Recursively scan a directory for audio files.
    fn scan_directory(
        &self,
        dir: &Path,
        results: &mut Vec<SearchResult>,
    ) -> Result<(), SourceError> {
        let entries = fs::read_dir(dir)?;

        // First pass: collect CUE files and the audio files they reference,
        // so we can skip those as standalone tracks (they exist via CUE tracks).
        // Use lowercased path strings for comparison — Windows is case-insensitive.
        let mut cue_refs: Vec<String> = Vec::new();
        let mut cue_paths: Vec<String> = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("cue"))
            {
                // Try to extract the referenced audio file name from the CUE sheet.
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(sheet) = phonon_codec::cue::parse_cue(&content) {
                        if let Some(ref file) = sheet.file {
                            let audio_path = path.parent().unwrap_or(Path::new(".")).join(file);
                            if audio_path.exists() {
                                cue_refs.push(Self::norm_path(&audio_path));
                            } else {
                                for ext in SUPPORTED_EXTENSIONS {
                                    let alt = audio_path.with_extension(ext);
                                    if alt.exists() {
                                        cue_refs.push(Self::norm_path(&alt));
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                cue_paths.push(Self::norm_path(&path));
            }
        }

        // Second pass: collect audio files, skipping CUE-referenced ones.
        let entries = fs::read_dir(dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let path_norm = Self::norm_path(&path);

            if path.is_dir() {
                // Never follow directory symlinks/junctions — a cycle would
                // recurse until the stack overflows and abort the process.
                // Same policy as phonon-media's scanner (follow_links(false)).
                let is_link = fs::symlink_metadata(&path)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                if !is_link {
                    self.scan_directory(&path, results)?;
                }
            } else if path.is_file() {
                if cue_paths.contains(&path_norm) {
                    if let Ok(cue_results) = self.parse_cue_and_associate(&path) {
                        results.extend(cue_results);
                    }
                } else if !cue_refs.contains(&path_norm) {
                    if let Some(result) = self.file_to_result(&path) {
                        results.push(result);
                    }
                }
            }
        }

        Ok(())
    }

    /// Normalise a path for case-insensitive comparison (Windows).
    fn norm_path(p: &Path) -> String {
        p.to_string_lossy().to_lowercase().replace('/', "\\")
    }

    /// Parse a CUE file and generate virtual track results.
    fn parse_cue_and_associate(&self, cue_path: &Path) -> Result<Vec<SearchResult>, SourceError> {
        let content = fs::read_to_string(cue_path)?;
        let sheet = phonon_codec::cue::parse_cue(&content)
            .map_err(|e| SourceError::Decode(e.to_string()))?;

        // Find the associated audio file — if it cannot be located on
        // disk, produce zero tracks (there's nothing playable anyway).
        let audio_file: Option<std::path::PathBuf> = if let Some(ref file) = sheet.file {
            let audio_path = cue_path.parent().unwrap_or(Path::new(".")).join(file);
            if audio_path.exists() {
                Some(audio_path)
            } else {
                // Try common extensions
                SUPPORTED_EXTENSIONS
                    .iter()
                    .map(|ext| audio_path.with_extension(ext))
                    .find(|alt| alt.exists())
            }
        } else {
            None
        };

        let Some(audio_file) = audio_file else {
            return Ok(Vec::new());
        };
        let audio_uri = audio_file.to_string_lossy().to_string();

        let mut results = Vec::new();
        for track in &sheet.tracks {
            let metadata = TrackMetadata {
                title: track.title.clone(),
                artist: track.performer.clone().or_else(|| sheet.performer.clone()),
                album: sheet.title.clone(),
                track_number: Some(track.index),
                duration: track.duration,
                ..Default::default()
            };

            let name = format!(
                "{:02}. {}",
                track.index,
                track.title.as_deref().unwrap_or("Unknown")
            );

            results.push(SearchResult {
                uri: audio_uri.clone(),
                name,
                metadata,
                cue_track_index: Some(track.index),
            });
        }

        Ok(results)
    }
}

/// Reader for local file audio sources.
struct LocalFileReader {
    decoder: phonon_codec::AudioDecoder,
    _uri: String,
}

impl AudioSourceReader for LocalFileReader {
    fn next_frame(&mut self) -> Result<Option<AudioFrame>, SourceError> {
        self.decoder
            .next_frame()
            .map_err(|e| SourceError::Decode(e.to_string()))
    }

    fn seek(&mut self, time_secs: f64) -> Result<(), SourceError> {
        self.decoder
            .seek(time_secs)
            .map_err(|e| SourceError::Decode(e.to_string()))
    }

    fn duration(&self) -> Option<f64> {
        self.decoder.metadata().duration
    }

    fn sample_rate(&self) -> u32 {
        self.decoder.sample_rate()
    }

    fn channels(&self) -> u16 {
        self.decoder.channels()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supported_extensions() {
        assert!(LocalFileSource::is_supported("flac"));
        assert!(LocalFileSource::is_supported("FLAC"));
        assert!(LocalFileSource::is_supported("mp3"));
        assert!(LocalFileSource::is_supported("dsf"));
        assert!(!LocalFileSource::is_supported("txt"));
        assert!(!LocalFileSource::is_supported("ogg"));
    }
}
