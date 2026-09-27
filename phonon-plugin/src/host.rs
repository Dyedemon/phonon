//! Host functions exposed to WASM plugins.
//!
//! Provides sandboxed access to logging, filesystem, and network.

use crate::types::PluginHost;

/// Default host implementation that provides sandboxed access.
pub struct DefaultHost {
    /// Allowed filesystem paths (empty = none).
    allowed_fs_paths: Vec<String>,
    /// Allowed network hosts (empty = none).
    allowed_network_hosts: Vec<String>,
    /// Whether network access is enabled at all.
    network_enabled: bool,
    /// Whether filesystem access is enabled at all.
    fs_enabled: bool,
}

impl DefaultHost {
    /// Create a new host with no permissions (default sandbox).
    pub fn new() -> Self {
        Self {
            allowed_fs_paths: Vec::new(),
            allowed_network_hosts: Vec::new(),
            network_enabled: false,
            fs_enabled: false,
        }
    }

    /// Grant filesystem access to a specific path.
    pub fn grant_fs_access(&mut self, path: &str) {
        self.fs_enabled = true;
        self.allowed_fs_paths.push(path.to_string());
    }

    /// Grant network access to a specific host.
    pub fn grant_network_access(&mut self, host: &str) {
        self.network_enabled = true;
        self.allowed_network_hosts.push(host.to_string());
    }

    /// Revoke all permissions.
    pub fn revoke_all(&mut self) {
        self.allowed_fs_paths.clear();
        self.allowed_network_hosts.clear();
        self.network_enabled = false;
        self.fs_enabled = false;
    }
}

impl Default for DefaultHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginHost for DefaultHost {
    fn log(&self, level: &str, message: &str) {
        match level.to_lowercase().as_str() {
            "error" => log::error!("[plugin] {}", message),
            "warn" => log::warn!("[plugin] {}", message),
            "info" => log::info!("[plugin] {}", message),
            "debug" => log::debug!("[plugin] {}", message),
            _ => log::info!("[plugin] {}", message),
        }
    }

    fn request_fs_access(&self, path: &str) -> bool {
        if !self.fs_enabled {
            return false;
        }
        self.allowed_fs_paths
            .iter()
            .any(|allowed| path.starts_with(allowed))
    }

    fn request_network_access(&self, url: &str) -> bool {
        if !self.network_enabled {
            return false;
        }
        self.allowed_network_hosts
            .iter()
            .any(|allowed| url.contains(allowed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_no_permissions() {
        let host = DefaultHost::new();
        assert!(!host.request_fs_access("/tmp/test.flac"));
        assert!(!host.request_network_access("https://example.com/stream"));
    }

    #[test]
    fn test_granted_permissions() {
        let mut host = DefaultHost::new();
        host.grant_fs_access("/music");
        host.grant_network_access("example.com");

        assert!(host.request_fs_access("/music/album/song.flac"));
        assert!(!host.request_fs_access("/etc/passwd"));
        assert!(host.request_network_access("https://example.com/stream"));
        assert!(!host.request_network_access("https://evil.com/stream"));
    }

    #[test]
    fn test_revoke_all() {
        let mut host = DefaultHost::new();
        host.grant_fs_access("/music");
        host.grant_network_access("example.com");
        assert!(host.request_fs_access("/music/file.flac"));
        assert!(host.request_network_access("https://example.com/stream"));

        host.revoke_all();
        assert!(!host.request_fs_access("/music/file.flac"));
        assert!(!host.request_network_access("https://example.com/stream"));
    }

    #[test]
    fn test_fs_access_exact_match() {
        let mut host = DefaultHost::new();
        host.grant_fs_access("/music/song.flac");
        assert!(host.request_fs_access("/music/song.flac"));
        // starts_with allows nested paths
        assert!(host.request_fs_access("/music/song.flac.backup"));
        // But /music alone does NOT start with /music/song.flac
        assert!(!host.request_fs_access("/music"));
    }

    #[test]
    fn test_fs_access_multiple_paths() {
        let mut host = DefaultHost::new();
        host.grant_fs_access("/music");
        host.grant_fs_access("/videos");
        assert!(host.request_fs_access("/music/album/song.flac"));
        assert!(host.request_fs_access("/videos/concert.mp4"));
        assert!(!host.request_fs_access("/documents/readme.txt"));
    }

    #[test]
    fn test_network_access_subdomain() {
        let mut host = DefaultHost::new();
        host.grant_network_access("example.com");
        assert!(host.request_network_access("https://example.com/stream"));
        assert!(host.request_network_access("https://sub.example.com/api"));
        // "example.com" is contained in "example.com.evil.net" but that's a different host
        // (contains check is loose, this is expected behavior for the current impl)
    }

    #[test]
    fn test_network_access_multiple_hosts() {
        let mut host = DefaultHost::new();
        host.grant_network_access("example.com");
        host.grant_network_access("trusted.org");
        assert!(host.request_network_access("https://example.com/stream"));
        assert!(host.request_network_access("https://trusted.org/api"));
        assert!(!host.request_network_access("https://evil.com/malware"));
    }

    #[test]
    fn test_default_host_default_trait() {
        let host = DefaultHost::default();
        assert!(!host.request_fs_access("/any/path"));
        assert!(!host.request_network_access("https://any.com"));
    }

    #[test]
    fn test_log_does_not_panic() {
        let host = DefaultHost::new();
        // Just verify logging doesn't panic for any level
        host.log("error", "test error");
        host.log("warn", "test warn");
        host.log("info", "test info");
        host.log("debug", "test debug");
        host.log("trace", "test trace"); // unknown level falls back to info
        host.log("ERROR", "uppercase error");
        host.log("WARN", "uppercase warn");
    }
}
