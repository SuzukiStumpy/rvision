//! Layered resource loading: app and user override layers as raw text
//! (ADR 0024).
//!
//! Locates and reads the **app** and **user** layers of a named resource so
//! a resource kind (theme, help content, ...) can overlay them onto its own
//! framework-embedded default. This module knows nothing about any kind's
//! file format, parsing, or merge rules — only where its bytes live. The
//! framework layer isn't modelled here at all: it's whatever the calling
//! kind already embeds at compile time, since a library crate has no
//! runtime install location of its own to discover.
//!
//! See `docs/specs/resource.md`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The app and user override layers of a resource, as raw text. A missing
/// file is not an error — the corresponding field is simply `None`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceLayers {
    /// The application-defaults layer, present when `app_dir` was supplied
    /// and the file exists.
    pub app: Option<String>,
    /// The user-customisation layer, present when a config directory
    /// resolves on this platform and the file exists.
    pub user: Option<String>,
}

/// Reads the app and user layers of `file_name` for `app_name`.
///
/// `app_dir` is the application's own resources directory, supplied by the
/// calling app (never guessed — ADR 0024); `None` means the app has no such
/// directory and the `app` layer is unconditionally `None`, without
/// attempting any path. The user layer is located via [`user_resource_path`].
pub fn load_layers(
    app_name: &str,
    file_name: &str,
    app_dir: Option<&Path>,
) -> io::Result<ResourceLayers> {
    let app_path = app_dir.map(|dir| dir.join(file_name));
    let user_path = user_resource_path(app_name, file_name);
    load_layers_from(app_path.as_deref(), user_path.as_deref())
}

/// Resolves the user-customisation layer's path for `file_name` under
/// `app_name`, following the host OS's config-directory convention. `None`
/// if no such directory resolves from the environment (e.g. neither
/// `XDG_CONFIG_HOME` nor `HOME` is set on Linux).
pub fn user_resource_path(app_name: &str, file_name: &str) -> Option<PathBuf> {
    user_app_dir(app_name).map(|dir| dir.join(file_name))
}

/// Writes `contents` to the user-customisation layer for `file_name` under
/// `app_name`, creating its directory if necessary. A silent no-op returning
/// `Ok(())` when no config directory resolves — mirroring
/// [`user_resource_path`]'s own leniency, not an error.
pub fn write_user_resource(app_name: &str, file_name: &str, contents: &str) -> io::Result<()> {
    write_user_resource_to(user_app_dir(app_name).as_deref(), file_name, contents)
}

/// Reads `path`, treating a missing file as `Ok(None)`; any other I/O
/// failure propagates.
fn read_optional(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// The core of [`load_layers`], taking both layers' already-resolved full
/// file paths directly — testable against arbitrary paths (a temp
/// directory) without needing to fake the real per-OS environment.
fn load_layers_from(
    app_path: Option<&Path>,
    user_path: Option<&Path>,
) -> io::Result<ResourceLayers> {
    let app = match app_path {
        Some(path) => read_optional(path)?,
        None => None,
    };
    let user = match user_path {
        Some(path) => read_optional(path)?,
        None => None,
    };
    Ok(ResourceLayers { app, user })
}

/// Creates `dir` if needed, then writes `file_name` under it.
fn write_to(dir: &Path, file_name: &str, contents: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(dir.join(file_name), contents)
}

/// The core of [`write_user_resource`], taking the already-resolved user
/// app-directory directly — testable against an arbitrary directory (or
/// `None`) without needing to fake the real per-OS environment.
fn write_user_resource_to(
    user_dir: Option<&Path>,
    file_name: &str,
    contents: &str,
) -> io::Result<()> {
    match user_dir {
        Some(dir) => write_to(dir, file_name, contents),
        None => Ok(()),
    }
}

/// `app_name`'s directory under the host OS's config-directory convention,
/// resolved from the real process environment. The one impure entry point
/// in this module — the per-OS rules themselves are the pure, unit-tested
/// `unix_config_dir`/`macos_config_dir`/`windows_config_dir` below.
fn user_app_dir(app_name: &str) -> Option<PathBuf> {
    user_config_dir().map(|dir| dir.join(app_name))
}

fn user_config_dir() -> Option<PathBuf> {
    let env = |key: &str| std::env::var(key).ok();
    #[cfg(target_os = "macos")]
    {
        macos_config_dir(env("HOME").as_deref())
    }
    #[cfg(target_os = "windows")]
    {
        windows_config_dir(env("APPDATA").as_deref())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        unix_config_dir(env("XDG_CONFIG_HOME").as_deref(), env("HOME").as_deref())
    }
}

/// An empty environment variable counts as unset, matching
/// `edit::settings::nonempty`.
fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

/// Linux/BSD: `$XDG_CONFIG_HOME`, falling back to `$HOME/.config`.
#[cfg_attr(any(target_os = "macos", target_os = "windows"), allow(dead_code))]
fn unix_config_dir(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    nonempty(xdg_config_home)
        .map(PathBuf::from)
        .or_else(|| nonempty(home).map(|home| Path::new(home).join(".config")))
}

/// macOS: `$HOME/Library/Application Support`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_config_dir(home: Option<&str>) -> Option<PathBuf> {
    nonempty(home).map(|home| Path::new(home).join("Library/Application Support"))
}

/// Windows: `%APPDATA%`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn windows_config_dir(appdata: Option<&str>) -> Option<PathBuf> {
    nonempty(appdata).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A directory under the OS temp dir, unique per call, removed on drop —
    /// hand-rolled since the crate budget (ADR 0001) has no `tempfile`.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rvision-resource-test-{label}-{}-{n}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // --- per-OS user config dir resolution: pure, tested on every host ---

    #[test]
    fn unix_config_dir_prefers_xdg_config_home() {
        assert_eq!(
            unix_config_dir(Some("/xdg"), Some("/home/u")),
            Some(PathBuf::from("/xdg"))
        );
    }

    #[test]
    fn unix_config_dir_falls_back_to_home_dot_config() {
        assert_eq!(
            unix_config_dir(None, Some("/home/u")),
            Some(PathBuf::from("/home/u/.config"))
        );
    }

    #[test]
    fn unix_config_dir_empty_xdg_counts_as_unset() {
        assert_eq!(
            unix_config_dir(Some(""), Some("/home/u")),
            Some(PathBuf::from("/home/u/.config"))
        );
    }

    #[test]
    fn unix_config_dir_none_when_neither_set() {
        assert_eq!(unix_config_dir(None, None), None);
        assert_eq!(unix_config_dir(Some(""), Some("")), None);
    }

    #[test]
    fn macos_config_dir_uses_home_application_support() {
        assert_eq!(
            macos_config_dir(Some("/Users/u")),
            Some(PathBuf::from("/Users/u/Library/Application Support"))
        );
    }

    #[test]
    fn macos_config_dir_empty_home_counts_as_unset() {
        assert_eq!(macos_config_dir(Some("")), None);
    }

    #[test]
    fn macos_config_dir_none_when_home_unset() {
        assert_eq!(macos_config_dir(None), None);
    }

    #[test]
    fn windows_config_dir_uses_appdata() {
        assert_eq!(
            windows_config_dir(Some(r"C:\Users\u\AppData\Roaming")),
            Some(PathBuf::from(r"C:\Users\u\AppData\Roaming"))
        );
    }

    #[test]
    fn windows_config_dir_empty_appdata_counts_as_unset() {
        assert_eq!(windows_config_dir(Some("")), None);
    }

    #[test]
    fn windows_config_dir_none_when_unset() {
        assert_eq!(windows_config_dir(None), None);
    }

    // --- load_layers ---

    #[test]
    fn load_layers_app_dir_none_never_touches_app_layer() {
        let layers = load_layers("rvision-test-app", "does-not-matter", None).unwrap();
        assert_eq!(layers.app, None);
    }

    #[test]
    fn load_layers_from_neither_exists_returns_none_none() {
        let dir = TempDir::new("neither");
        let app_path = dir.path().join("app-theme");
        let user_path = dir.path().join("user-theme");

        let layers = load_layers_from(Some(&app_path), Some(&user_path)).unwrap();
        assert_eq!(
            layers,
            ResourceLayers {
                app: None,
                user: None
            }
        );
    }

    #[test]
    fn load_layers_from_reads_both_layers_when_present() {
        let dir = TempDir::new("both");
        let app_path = dir.path().join("app-theme");
        let user_path = dir.path().join("user-theme");
        fs::write(&app_path, "app contents").unwrap();
        fs::write(&user_path, "user contents").unwrap();

        let layers = load_layers_from(Some(&app_path), Some(&user_path)).unwrap();
        assert_eq!(
            layers,
            ResourceLayers {
                app: Some("app contents".to_string()),
                user: Some("user contents".to_string()),
            }
        );
    }

    #[test]
    fn load_layers_from_propagates_non_notfound_error_from_app_layer() {
        let dir = TempDir::new("app-error");
        // A directory where a file is expected fails to read, but not with
        // `NotFound` — that distinction is exactly what must propagate.
        let bogus_app_path = dir.path().join("subdir");
        fs::create_dir_all(&bogus_app_path).unwrap();

        let err = load_layers_from(Some(&bogus_app_path), None).unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn load_layers_from_propagates_non_notfound_error_from_user_layer() {
        let dir = TempDir::new("user-error");
        let bogus_user_path = dir.path().join("subdir");
        fs::create_dir_all(&bogus_user_path).unwrap();

        let err = load_layers_from(None, Some(&bogus_user_path)).unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::NotFound);
    }

    // --- write_user_resource ---

    #[test]
    fn write_user_resource_to_then_load_layers_from_round_trips() {
        let dir = TempDir::new("roundtrip");
        write_user_resource_to(Some(dir.path()), "theme", "role.fg = red").unwrap();

        let layers = load_layers_from(None, Some(&dir.path().join("theme"))).unwrap();
        assert_eq!(layers.user.as_deref(), Some("role.fg = red"));
    }

    #[test]
    fn write_user_resource_to_creates_missing_directory() {
        let dir = TempDir::new("mkdir");
        let nested = dir.path().join("nested").join("app-name");
        assert!(!nested.exists());

        write_user_resource_to(Some(&nested), "theme", "contents").unwrap();

        assert_eq!(
            fs::read_to_string(nested.join("theme")).unwrap(),
            "contents"
        );
    }

    #[test]
    fn write_user_resource_to_is_silent_noop_when_no_dir_resolves() {
        write_user_resource_to(None, "theme", "contents").unwrap();
    }
}
