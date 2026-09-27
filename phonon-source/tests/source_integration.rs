//! Integration tests for the phonon-source crate.
//!
//! Tests playlist persistence, local file source, and network source.

use phonon_source::*;

// ============================================================
// Playlist M3U/M3U8 tests
// ============================================================

#[test]
fn test_playlist_m3u_roundtrip() {
    let mut playlist = Playlist::new("Test M3U");
    playlist.add(PlaylistEntry {
        path: "/music/album/song1.flac".into(),
        title: Some("Song One".into()),
        artist: Some("Artist A".into()),
        duration: Some(245.0),
    });
    playlist.add(PlaylistEntry {
        path: "/music/album/song2.flac".into(),
        title: Some("Song Two".into()),
        artist: Some("Artist A".into()),
        duration: Some(198.5),
    });

    let m3u_content = playlist.to_m3u();
    let parsed = parse_m3u(&m3u_content).unwrap();

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].path, "/music/album/song1.flac");
    assert_eq!(parsed[0].title.as_deref(), Some("Song One"));
    assert_eq!(parsed[0].duration, Some(245.0));
    assert_eq!(parsed[1].path, "/music/album/song2.flac");
}

#[test]
fn test_playlist_m3u_no_metadata() {
    let mut playlist = Playlist::new("Simple");
    playlist.add(PlaylistEntry {
        path: "/music/track.flac".into(),
        title: None,
        artist: None,
        duration: None,
    });

    let m3u = playlist.to_m3u();
    let parsed = parse_m3u(&m3u).unwrap();

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].path, "/music/track.flac");
    assert!(parsed[0].title.is_none());
}

#[test]
fn test_playlist_m3u_relative_paths() {
    let content = "#EXTM3U\n#EXTINF:300,Song\n./relative/path/song.flac\n";
    let parsed = parse_m3u(content).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].path, "./relative/path/song.flac");
}

#[test]
fn test_playlist_m3u_absolute_paths() {
    let content = "#EXTM3U\nC:\\Music\\song.flac\n";
    let parsed = parse_m3u(content).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].path, "C:\\Music\\song.flac");
}

#[test]
fn test_playlist_m3u_comments_ignored() {
    let content = "#EXTM3U\n# This is a comment\n#EXTINF:200,Test\n/path/song.flac\n";
    let parsed = parse_m3u(content).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].title.as_deref(), Some("Test"));
}

// ============================================================
// Playlist PLS tests
// ============================================================

#[test]
fn test_playlist_pls_roundtrip() {
    let mut playlist = Playlist::new("Test PLS");
    playlist.add(PlaylistEntry {
        path: "/music/track1.flac".into(),
        title: Some("First Track".into()),
        artist: None,
        duration: Some(300.0),
    });

    let pls_content = playlist.to_pls();
    let parsed = parse_pls(&pls_content).unwrap();

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].path, "/music/track1.flac");
    assert_eq!(parsed[0].title.as_deref(), Some("First Track"));
    assert_eq!(parsed[0].duration, Some(300.0));
}

#[test]
fn test_playlist_pls_multiple_entries() {
    let content = r#"[playlist]
File1=/music/01.flac
Title1=Track 1
Length1=200
File2=/music/02.flac
Title2=Track 2
Length2=250
NumberOfEntries=2
Version=2
"#;

    let parsed = parse_pls(content).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].path, "/music/01.flac");
    assert_eq!(parsed[1].path, "/music/02.flac");
    assert_eq!(parsed[1].duration, Some(250.0));
}

#[test]
fn test_playlist_pls_empty() {
    let content = "[playlist]\nNumberOfEntries=0\nVersion=2\n";
    let parsed = parse_pls(content).unwrap();
    assert_eq!(parsed.len(), 0);
}

// ============================================================
// Playlist XSPF tests
// ============================================================

#[test]
fn test_playlist_xspf_roundtrip() {
    let mut playlist = Playlist::new("Test XSPF");
    playlist.add(PlaylistEntry {
        path: "/music/song.flac".into(),
        title: Some("Test & Song <awesome>".into()),
        artist: Some("Artist \"Name\"".into()),
        duration: Some(180.0),
    });

    let xspf = playlist.to_xspf();
    let parsed = parse_xspf(&xspf).unwrap();

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].path, "/music/song.flac");
    assert_eq!(parsed[0].title.as_deref(), Some("Test & Song <awesome>"));
    assert_eq!(parsed[0].artist.as_deref(), Some("Artist \"Name\""));
    assert_eq!(parsed[0].duration, Some(180.0));
}

#[test]
fn test_playlist_xspf_multiple_tracks() {
    let mut playlist = Playlist::new("Multi");
    for i in 1..=5 {
        playlist.add(PlaylistEntry {
            path: format!("/music/track{}.flac", i),
            title: Some(format!("Track {}", i)),
            artist: None,
            duration: Some(200.0),
        });
    }

    let xspf = playlist.to_xspf();
    let parsed = parse_xspf(&xspf).unwrap();

    assert_eq!(parsed.len(), 5);
    for (i, entry) in parsed.iter().enumerate() {
        assert_eq!(entry.path, format!("/music/track{}.flac", i + 1));
    }
}

#[test]
fn test_playlist_xspf_empty() {
    let playlist = Playlist::new("Empty");
    let xspf = playlist.to_xspf();
    let parsed = parse_xspf(&xspf).unwrap();
    assert_eq!(parsed.len(), 0);
}

// ============================================================
// Playlist format detection
// ============================================================

#[test]
fn test_playlist_format_detection() {
    assert_eq!(
        PlaylistFormat::from_extension("test.m3u"),
        Some(PlaylistFormat::M3u)
    );
    assert_eq!(
        PlaylistFormat::from_extension("test.m3u8"),
        Some(PlaylistFormat::M3u8)
    );
    assert_eq!(
        PlaylistFormat::from_extension("test.pls"),
        Some(PlaylistFormat::Pls)
    );
    assert_eq!(
        PlaylistFormat::from_extension("test.xspf"),
        Some(PlaylistFormat::Xspf)
    );
    assert_eq!(PlaylistFormat::from_extension("test.txt"), None);
    assert_eq!(PlaylistFormat::from_extension("test.flac"), None);
}

// ============================================================
// Playlist manipulation tests
// ============================================================

#[test]
fn test_playlist_add_remove() {
    let mut playlist = Playlist::new("Test");
    assert!(playlist.is_empty());

    playlist.add(PlaylistEntry {
        path: "/test.flac".into(),
        title: None,
        artist: None,
        duration: None,
    });
    assert_eq!(playlist.len(), 1);
    assert!(!playlist.is_empty());

    let removed = playlist.remove(0);
    assert!(removed.is_some());
    assert_eq!(removed.unwrap().path, "/test.flac");
    assert!(playlist.is_empty());
}

#[test]
fn test_playlist_remove_out_of_bounds() {
    let mut playlist = Playlist::new("Test");
    playlist.add(PlaylistEntry {
        path: "/test.flac".into(),
        title: None,
        artist: None,
        duration: None,
    });

    assert!(playlist.remove(5).is_none());
    assert_eq!(playlist.len(), 1);
}

#[test]
fn test_playlist_reorder() {
    let mut playlist = Playlist::new("Test");
    playlist.add(PlaylistEntry {
        path: "/first.flac".into(),
        title: None,
        artist: None,
        duration: None,
    });
    playlist.add(PlaylistEntry {
        path: "/second.flac".into(),
        title: None,
        artist: None,
        duration: None,
    });
    playlist.add(PlaylistEntry {
        path: "/third.flac".into(),
        title: None,
        artist: None,
        duration: None,
    });

    // Move first to last
    playlist.reorder(0, 2);
    assert_eq!(playlist.entries[0].path, "/second.flac");
    assert_eq!(playlist.entries[1].path, "/third.flac");
    assert_eq!(playlist.entries[2].path, "/first.flac");
}

// ============================================================
// LocalFileSource tests
// ============================================================

#[test]
fn test_local_file_source_supported_extensions() {
    assert!(LocalFileSource::is_supported("flac"));
    assert!(LocalFileSource::is_supported("FLAC"));
    assert!(LocalFileSource::is_supported("mp3"));
    assert!(LocalFileSource::is_supported("MP3"));
    assert!(LocalFileSource::is_supported("wav"));
    assert!(LocalFileSource::is_supported("WAVE"));
    assert!(LocalFileSource::is_supported("aac"));
    assert!(LocalFileSource::is_supported("m4a"));
    assert!(LocalFileSource::is_supported("dsf"));
    assert!(LocalFileSource::is_supported("dff"));
    assert!(!LocalFileSource::is_supported("txt"));
    assert!(!LocalFileSource::is_supported("ogg"));
    assert!(!LocalFileSource::is_supported(""));
}

#[test]
fn test_local_file_source_name() {
    let source = LocalFileSource::new();
    assert_eq!(source.name(), "Local Files");
    assert_eq!(source.source_type(), SourceType::LocalFile);
}

// ============================================================
// NetworkStreamSource tests
// ============================================================

#[test]
fn test_network_stream_is_stream_uri() {
    assert!(NetworkStreamSource::is_stream_uri(
        "http://example.com/stream"
    ));
    assert!(NetworkStreamSource::is_stream_uri(
        "https://icecast.example.com/mount"
    ));
    assert!(NetworkStreamSource::is_stream_uri(
        "http://localhost:8000/stream.mp3"
    ));
    assert!(!NetworkStreamSource::is_stream_uri("/local/file.flac"));
    assert!(!NetworkStreamSource::is_stream_uri("C:\\music\\file.mp3"));
    assert!(!NetworkStreamSource::is_stream_uri(""));
}

#[test]
fn test_network_stream_source_name() {
    let source = NetworkStreamSource::new();
    assert_eq!(source.name(), "Network Stream");
    assert_eq!(source.source_type(), SourceType::NetworkStream);
}

#[test]
fn test_network_stream_search() {
    let source = NetworkStreamSource::new();
    let result = source.search("http://example.com/stream").unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].uri, "http://example.com/stream");
}

#[test]
fn test_network_stream_invalid_uri() {
    let source = NetworkStreamSource::new();
    let result = source.search("/local/file.flac");
    assert!(result.is_err());
}

// ============================================================
// SourceType tests
// ============================================================

#[test]
fn test_source_type_values() {
    assert_ne!(SourceType::LocalFile, SourceType::NetworkStream);
    assert_ne!(SourceType::LocalFile, SourceType::Plugin);
    assert_ne!(SourceType::NetworkStream, SourceType::Plugin);
}

// ============================================================
// SourceError tests
// ============================================================

#[test]
fn test_source_error_display() {
    let err = SourceError::NotFound("file.flac".into());
    assert!(err.to_string().contains("file.flac"));

    let err = SourceError::UnsupportedFormat("ogg".into());
    assert!(err.to_string().contains("ogg"));
}

// ============================================================
// PlaylistError tests
// ============================================================

#[test]
fn test_playlist_error_display() {
    let err = PlaylistError::UnsupportedFormat("bad.ext".into());
    assert!(err.to_string().contains("bad.ext"));

    let err = PlaylistError::Parse("invalid syntax".into());
    assert!(err.to_string().contains("invalid syntax"));
}

// ============================================================
// Playlist serialization round-trip tests
// ============================================================

#[test]
fn test_playlist_save_load_roundtrip() {
    // Test M3U8 round-trip
    let mut original = Playlist::new("Roundtrip");
    original.add(PlaylistEntry {
        path: "/music/01 - Intro.flac".into(),
        title: Some("Intro".into()),
        artist: Some("Artist".into()),
        duration: Some(60.0),
    });
    original.add(PlaylistEntry {
        path: "/music/02 - Main.flac".into(),
        title: Some("Main Theme".into()),
        artist: Some("Artist".into()),
        duration: Some(300.0),
    });

    // Serialize
    let m3u = original.to_m3u();

    // Parse back
    let parsed = parse_m3u(&m3u).unwrap();

    assert_eq!(parsed.len(), original.len());
    for (orig, parsed) in original.entries.iter().zip(parsed.iter()) {
        assert_eq!(orig.path, parsed.path);
        assert_eq!(orig.title, parsed.title);
        assert_eq!(orig.duration, parsed.duration);
    }
}

// ============================================================
// Helper: parse_m3u (re-exported for testing)
// ============================================================

use phonon_source::playlist::parse_m3u;
use phonon_source::playlist::parse_pls;
use phonon_source::playlist::parse_xspf;

// ============================================================
// XML escape tests
// ============================================================

#[test]
fn test_xspf_escape_special_chars() {
    let mut playlist = Playlist::new("Escaped");
    playlist.add(PlaylistEntry {
        path: "/music/rock & roll.flac".into(),
        title: Some("Rock & Roll <Live>".into()),
        artist: Some("Artist \"The Best\"".into()),
        duration: Some(240.0),
    });

    let xspf = playlist.to_xspf();

    // Verify escaped characters
    assert!(xspf.contains("&amp;"));
    assert!(xspf.contains("&lt;"));
    assert!(xspf.contains("&gt;"));
    assert!(xspf.contains("&quot;"));
    assert!(!xspf.contains("Rock & Roll <Live>")); // Raw should be escaped
}

// ============================================================
// PlaylistEntry serialization
// ============================================================

#[test]
fn test_playlist_entry_serialization() {
    let entry = PlaylistEntry {
        path: "/test.flac".into(),
        title: Some("Test".into()),
        artist: Some("Artist".into()),
        duration: Some(100.0),
    };

    let json = serde_json::to_string(&entry).unwrap();
    let parsed: PlaylistEntry = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.path, entry.path);
    assert_eq!(parsed.title, entry.title);
    assert_eq!(parsed.artist, entry.artist);
    assert_eq!(parsed.duration, entry.duration);
}
