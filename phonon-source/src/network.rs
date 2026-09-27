//! Network stream audio source.
//!
//! Supports HTTP/HTTPS streaming, Icecast/SHOUTcast, and HLS.
//! Downloads stream content into a buffer, then decodes via symphonia.

use std::io::{Cursor, Read};
use std::time::Duration;

use crate::traits::*;
use phonon_codec::*;

/// Network stream audio source.
pub struct NetworkStreamSource {
    name: String,
    /// Timeout for HTTP requests.
    timeout: Duration,
    /// Maximum download size in bytes (default: 100MB).
    max_download_size: usize,
}

impl NetworkStreamSource {
    /// Create a new network stream source with default settings.
    pub fn new() -> Self {
        Self {
            name: "Network Stream".into(),
            timeout: Duration::from_secs(30),
            max_download_size: 100 * 1024 * 1024,
        }
    }

    /// Set the HTTP request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the maximum download size.
    pub fn with_max_size(mut self, size: usize) -> Self {
        self.max_download_size = size;
        self
    }

    /// Check if a URI is a network stream.
    pub fn is_stream_uri(uri: &str) -> bool {
        uri.starts_with("http://") || uri.starts_with("https://") || uri.starts_with("icecast://")
    }

    /// Download data from a URL into a buffer.
    /// Note: TLS is compiled out of this build, so HTTPS URLs are rejected
    /// with an explicit error instead of being silently downgraded to
    /// plaintext HTTP.
    fn download(&self, url: &str) -> Result<Vec<u8>, SourceError> {
        if url.starts_with("https://") {
            return Err(SourceError::Network(
                "HTTPS streams are not supported in this build (TLS disabled). \
                 Use an http:// stream URL instead."
                    .into(),
            ));
        }
        let normalized = url.replacen("icecast://", "http://", 1);

        let response = ureq::get(&normalized)
            .set("User-Agent", "Phonon/0.1.0")
            .set("Icy-MetaData", "1") // Request Icecast metadata
            .timeout(self.timeout)
            .call()
            .map_err(|e| SourceError::Network(format!("HTTP request failed: {}", e)))?;

        // Check content type
        let content_type = response.header("Content-Type").unwrap_or("unknown");
        log::info!("Stream content-type: {}", content_type);

        let content_length: Option<usize> = response
            .header("Content-Length")
            .and_then(|v| v.parse().ok());

        // Read response body
        let mut reader = response.into_reader();
        let mut buffer = Vec::new();

        if let Some(len) = content_length {
            if len > self.max_download_size {
                return Err(SourceError::Network(format!(
                    "Content too large: {} bytes (max: {})",
                    len, self.max_download_size
                )));
            }
            buffer.reserve(len);
        }

        // Read in chunks with progress logging
        let mut total = 0u64;
        let mut chunk = vec![0u8; 65536];
        loop {
            let n = reader
                .read(&mut chunk)
                .map_err(|e| SourceError::Network(format!("Read error: {}", e)))?;
            if n == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..n]);
            total += n as u64;

            if total > self.max_download_size as u64 {
                return Err(SourceError::Network(format!(
                    "Download exceeded max size: {} bytes",
                    self.max_download_size
                )));
            }
        }

        log::info!("Downloaded {} bytes from {}", total, url);
        Ok(buffer)
    }
}

impl Default for NetworkStreamSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Wraps a buffered network stream for symphonia decoding.
struct NetworkStreamReader {
    decoder: AudioDecoder,
    sample_rate: u32,
    channels: u16,
}

impl AudioSourceReader for NetworkStreamReader {
    fn next_frame(&mut self) -> Result<Option<AudioFrame>, SourceError> {
        self.decoder
            .next_frame()
            .map_err(|e| SourceError::Decode(e.to_string()))
    }

    fn seek(&mut self, _time_secs: f64) -> Result<(), SourceError> {
        // Seeking not supported for network streams
        Err(SourceError::Network(
            "Seek not supported for network streams".into(),
        ))
    }

    fn duration(&self) -> Option<f64> {
        None // Unknown for streams
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> u16 {
        self.channels
    }
}

impl IAudioSource for NetworkStreamSource {
    fn search(&self, query: &str) -> Result<Vec<SearchResult>, SourceError> {
        if !Self::is_stream_uri(query) {
            return Err(SourceError::NotFound(format!(
                "Not a stream URI: {}",
                query
            )));
        }

        Ok(vec![SearchResult {
            uri: query.to_string(),
            name: query.to_string(),
            metadata: TrackMetadata {
                title: Some(query.to_string()),
                format: Some(AudioFormat::Unknown),
                ..Default::default()
            },
            cue_track_index: None,
        }])
    }

    fn get_metadata(&self, uri: &str) -> Result<TrackMetadata, SourceError> {
        // For network streams, we attempt a HEAD request to get metadata
        if !Self::is_stream_uri(uri) {
            return Err(SourceError::NotFound(format!("Not a stream URI: {}", uri)));
        }
        if uri.starts_with("https://") {
            return Err(SourceError::Network(
                "HTTPS streams are not supported in this build (TLS disabled). \
                 Use an http:// stream URL instead."
                    .into(),
            ));
        }

        let normalized = uri.replacen("icecast://", "http://", 1);

        match ureq::head(&normalized)
            .set("User-Agent", "Phonon/0.1.0")
            .timeout(Duration::from_secs(10))
            .call()
        {
            Ok(resp) => {
                let content_type = resp.header("Content-Type").unwrap_or("unknown");
                let icy_name = resp.header("icy-name");
                let icy_genre = resp.header("icy-genre");

                Ok(TrackMetadata {
                    title: icy_name
                        .map(|s| s.to_string())
                        .or_else(|| Some(uri.to_string())),
                    artist: None,
                    album: None,
                    genre: icy_genre.map(|s| s.to_string()),
                    format: Some(AudioFormat::from_extension(content_type)),
                    ..Default::default()
                })
            }
            Err(_) => {
                // Fallback: return basic metadata
                Ok(TrackMetadata {
                    title: Some(uri.to_string()),
                    format: Some(AudioFormat::Unknown),
                    ..Default::default()
                })
            }
        }
    }

    fn open_decoder(&self, uri: &str) -> Result<Box<dyn AudioSourceReader>, SourceError> {
        if !Self::is_stream_uri(uri) {
            return Err(SourceError::NotFound(format!("Not a stream URI: {}", uri)));
        }

        log::info!("Opening network stream: {}", uri);

        // Download the stream into a buffer
        let data = self.download(uri)?;

        if data.is_empty() {
            return Err(SourceError::Network("Empty response body".into()));
        }

        // Create a cursor over the buffered data
        let cursor = Cursor::new(data);

        // Use symphonia to determine the format and decode
        let decoder = AudioDecoder::open_reader(Box::new(cursor), uri)
            .map_err(|e| SourceError::Decode(format!("Failed to decode stream: {}", e)))?;

        let sample_rate = decoder.sample_rate();
        let channels = decoder.channels();

        log::info!("Stream opened: {}Hz, {} channels", sample_rate, channels);

        Ok(Box::new(NetworkStreamReader {
            decoder,
            sample_rate,
            channels,
        }))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn source_type(&self) -> SourceType {
        SourceType::NetworkStream
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_stream_uri() {
        assert!(NetworkStreamSource::is_stream_uri(
            "http://example.com/stream"
        ));
        assert!(NetworkStreamSource::is_stream_uri(
            "https://example.com/stream.m3u8"
        ));
        assert!(NetworkStreamSource::is_stream_uri(
            "icecast://radio.example.com:8000/mount"
        ));
        assert!(!NetworkStreamSource::is_stream_uri("/local/file.flac"));
        assert!(!NetworkStreamSource::is_stream_uri("C:\\music\\file.flac"));
        assert!(!NetworkStreamSource::is_stream_uri(
            "file:///music/file.mp3"
        ));
    }

    #[test]
    fn test_network_source_default() {
        let source = NetworkStreamSource::default();
        assert_eq!(source.name(), "Network Stream");
        assert_eq!(source.source_type(), SourceType::NetworkStream);
        assert_eq!(source.max_download_size, 100 * 1024 * 1024);
    }

    #[test]
    fn test_network_source_custom() {
        let source = NetworkStreamSource::new()
            .with_timeout(Duration::from_secs(60))
            .with_max_size(50 * 1024 * 1024);
        assert_eq!(source.timeout, Duration::from_secs(60));
        assert_eq!(source.max_download_size, 50 * 1024 * 1024);
    }

    #[test]
    fn test_search_non_stream_uri() {
        let source = NetworkStreamSource::new();
        let result = source.search("/local/file.flac");
        assert!(result.is_err());
    }

    #[test]
    fn test_search_stream_uri() {
        let source = NetworkStreamSource::new();
        let result = source.search("http://example.com/stream.mp3").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].uri, "http://example.com/stream.mp3");
    }

    #[test]
    fn test_get_metadata_non_stream() {
        let source = NetworkStreamSource::new();
        let result = source.get_metadata("/local/file.flac");
        assert!(result.is_err());
    }

    #[test]
    fn test_open_decoder_non_stream() {
        let source = NetworkStreamSource::new();
        let result = source.open_decoder("/local/file.flac");
        assert!(result.is_err());
    }
}
