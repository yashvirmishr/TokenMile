//! Community UI theme packs.
//!
//! A theme is a JSON document in `<data-dir>/freetokenmeter/themes/`. The
//! frontend owns the schema and the visual meaning of every token. This module
//! only guarantees three things:
//!
//! 1. Only regular `.json` files are read, with a bounded size.
//! 2. A malformed file never breaks the picker — it is reported as a problem
//!    while every valid theme still loads.
//! 3. Saving a copy can never escape the themes directory or overwrite an
//!    existing community theme by accident.
//!
//! Keeping validation here deliberately thin means a community theme can add a
//! new token without a backend change; the frontend ignores what it does not
//! understand and falls back for anything malformed.

use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Largest theme file accepted. A theme is a token document, not an asset
/// bundle; 512 KiB is already absurdly generous.
pub const MAX_THEME_BYTES: u64 = 512 * 1024;

/// Directory name under the application data directory.
pub const THEMES_DIR_NAME: &str = "themes";

/// A theme file as handed to the frontend. The frontend parses `contents` and
/// reports schema-level problems in its own picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserThemeFile {
    pub id: String,
    pub file: String,
    pub path: String,
    pub contents: String,
}

/// A file that looked like a theme but could not be read safely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThemeProblem {
    pub file: String,
    pub message: String,
}

/// Everything the theme picker needs from disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThemeLibrary {
    pub directory: String,
    pub themes: Vec<UserThemeFile>,
    pub problems: Vec<ThemeProblem>,
}

/// `<data-dir>/freetokenmeter/themes`
pub fn themes_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(THEMES_DIR_NAME)
}

pub fn ensure_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("Failed to create {}: {}", dir.display(), e))
}

/// Read every valid theme file in `dir`.
///
/// Missing directories are an empty library, not an error: a user who has never
/// installed a theme should see the built-ins with an empty community shelf.
pub fn discover(dir: &Path) -> ThemeLibrary {
    let mut themes = Vec::new();
    let mut problems = Vec::new();

    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ThemeLibrary {
                directory: dir.display().to_string(),
                themes,
                problems,
            };
        }
        Err(e) => {
            problems.push(ThemeProblem {
                file: dir.display().to_string(),
                message: format!("Could not read the themes directory: {e}"),
            });
            return ThemeLibrary {
                directory: dir.display().to_string(),
                themes,
                problems,
            };
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Only plain files with a .json extension. `file_type()` does not follow
        // symlinks, which keeps a community theme from pointing outside the
        // folder we opened for the user.
        let is_json = path
            .extension()
            .map(|ext| ext.eq_ignore_ascii_case("json"))
            .unwrap_or(false);
        if !is_json {
            continue;
        }
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            problems.push(ThemeProblem {
                file: file_name,
                message: "Not a regular file; skipped".to_string(),
            });
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(e) => {
                problems.push(ThemeProblem {
                    file: file_name,
                    message: format!("Could not inspect the file: {e}"),
                });
                continue;
            }
        };

        if metadata.len() > MAX_THEME_BYTES {
            problems.push(ThemeProblem {
                file: file_name,
                message: format!(
                    "File is {} KiB; themes must be under {} KiB",
                    metadata.len() / 1024,
                    MAX_THEME_BYTES / 1024
                ),
            });
            continue;
        }

        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(e) => {
                problems.push(ThemeProblem {
                    file: file_name,
                    message: format!("Could not read the file as UTF-8: {e}"),
                });
                continue;
            }
        };

        // A UTF-8 BOM from a Windows editor is not part of JSON. Strip it so a
        // hand-edited theme opens rather than being reported as invalid.
        let contents = contents
            .strip_prefix('\u{feff}')
            .unwrap_or(&contents)
            .to_string();

        if let Err(message) = validate_json_object(&contents) {
            problems.push(ThemeProblem {
                file: file_name,
                message,
            });
            continue;
        }

        themes.push(UserThemeFile {
            id: theme_id_from_file(&file_name),
            file: file_name,
            path: path.display().to_string(),
            contents,
        });
    }

    // Non-ASCII or punctuation-heavy file names can normalize to the same id.
    // Give later files a deterministic suffix so the frontend never gets two
    // themes with one key.
    themes.sort_by_key(|a| a.file.to_lowercase());
    let mut used_ids = HashSet::new();
    for theme in &mut themes {
        let base = theme.id.clone();
        let mut candidate = base.clone();
        let mut suffix = 2;
        while !used_ids.insert(candidate.clone()) {
            candidate = format!("{base}-{suffix}");
            suffix += 1;
        }
        theme.id = candidate;
    }

    themes.sort_by_key(|a| a.id.to_lowercase());
    problems.sort_by_key(|a| a.file.to_lowercase());

    ThemeLibrary {
        directory: dir.display().to_string(),
        themes,
        problems,
    }
}

/// Save a copy of a theme. Returns the file that was written.
///
/// An existing name is never overwritten: `my-theme.json`, `my-theme-2.json`,
/// and so on. That makes "start from this built-in" safe to click twice and
/// prevents a community theme from silently replacing another one.
pub fn save(dir: &Path, name: &str, contents: &str) -> Result<UserThemeFile, String> {
    ensure_dir(dir)?;

    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);

    if contents.len() as u64 > MAX_THEME_BYTES {
        return Err(format!(
            "Theme is too large ({} KiB); the limit is {} KiB",
            contents.len() / 1024,
            MAX_THEME_BYTES / 1024
        ));
    }
    validate_json_object(contents)?;

    let slug = slugify(name);
    let path = unique_path(dir, &slug);
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| "Theme path has no file name".to_string())?;

    // Write to a sibling temp file and rename, so a crash cannot leave a
    // half-written theme that the picker then reports as broken.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, contents.as_bytes())
        .map_err(|e| format!("Failed to write {}: {}", tmp.display(), e))?;
    if let Err(e) = fs::rename(&tmp, &path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Failed to save {}: {}", path.display(), e));
    }

    Ok(UserThemeFile {
        id: theme_id_from_file(&file_name),
        file: file_name,
        path: path.display().to_string(),
        contents: contents.to_string(),
    })
}

/// Open the themes directory in the platform file manager.
pub fn open_folder(dir: &Path) -> Result<(), String> {
    ensure_dir(dir)?;

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer");
        command.arg(dir);
        command
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(dir);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(dir);
        command
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open {}: {}", dir.display(), e))
}

/// A theme must be a JSON object. Schema validation belongs to the frontend.
fn validate_json_object(contents: &str) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_str(contents).map_err(|e| format!("Invalid JSON: {e}"))?;
    if !value.is_object() {
        return Err("A theme must be a JSON object".to_string());
    }
    Ok(())
}

/// File stem reduced to an id that is safe to use as a preference key.
fn theme_id_from_file(file_name: &str) -> String {
    let stem = Path::new(file_name)
        .file_stem()
        .map(|stem| stem.to_string_lossy())
        .unwrap_or_default();

    let mut id: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();

    while id.contains("--") {
        id = id.replace("--", "-");
    }
    let id = id.trim_matches(|c| c == '-' || c == '.').to_string();

    if id.is_empty() {
        "community-theme".to_string()
    } else {
        id
    }
}

/// ASCII slug for a new file. Unicode names fall back to a readable generic name
/// rather than producing an unusable file name on a different platform.
fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;

    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "community-theme".to_string()
    } else {
        slug.truncate(48);
        while slug.ends_with('-') {
            slug.pop();
        }
        slug
    }
}

fn unique_path(dir: &Path, slug: &str) -> PathBuf {
    let mut path = dir.join(format!("{slug}.json"));
    if !path.exists() {
        return path;
    }

    for suffix in 2..=999 {
        path = dir.join(format!("{slug}-{suffix}.json"));
        if !path.exists() {
            return path;
        }
    }

    // Practically unreachable; still deterministic and inside `dir`.
    dir.join(format!("{slug}-{}.json", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "ftm-themes-test-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn discover_loads_valid_themes_and_reports_bad_ones() {
        let dir = temp_dir("discover");
        fs::write(dir.join("a.json"), br#"{"name":"Alpha"}"#).unwrap();
        fs::write(dir.join("b.JSON"), br#"{"name":"Bravo"}"#).unwrap();
        fs::write(dir.join("broken.json"), b"{ not json").unwrap();
        fs::write(dir.join("notes.txt"), b"ignore me").unwrap();
        fs::write(dir.join("array.json"), b"[]").unwrap();

        let library = discover(&dir);

        assert_eq!(library.themes.len(), 2);
        assert_eq!(library.themes[0].id, "a");
        assert_eq!(library.themes[1].id, "b");
        assert_eq!(library.problems.len(), 2);
        assert!(library.problems.iter().any(|p| p.file == "broken.json"));
        assert!(library.problems.iter().any(|p| p.file == "array.json"));
    }

    #[test]
    fn discover_accepts_a_utf8_bom_from_a_windows_editor() {
        let dir = temp_dir("bom");
        fs::write(dir.join("bom.json"), "\u{feff}{\"name\":\"BOM\"}").unwrap();

        let library = discover(&dir);
        assert!(library.problems.is_empty(), "{:?}", library.problems);
        assert_eq!(library.themes.len(), 1);
        assert!(!library.themes[0].contents.starts_with('\u{feff}'));
    }

    #[test]
    fn discover_on_missing_directory_is_empty_not_an_error() {
        let dir = temp_dir("missing").join("never-created");
        let library = discover(&dir);
        assert!(library.themes.is_empty());
        assert!(library.problems.is_empty());
    }

    #[test]
    fn duplicate_theme_ids_get_deterministic_suffixes() {
        let dir = temp_dir("duplicate-ids");
        fs::write(dir.join("a b.json"), br#"{"name":"One"}"#).unwrap();
        fs::write(dir.join("a  b.json"), br#"{"name":"Two"}"#).unwrap();
        fs::write(dir.join("日本語.json"), br#"{"name":"Three"}"#).unwrap();
        fs::write(dir.join("中文.json"), br#"{"name":"Four"}"#).unwrap();

        let library = discover(&dir);
        let ids: HashSet<&str> = library.themes.iter().map(|t| t.id.as_str()).collect();

        assert_eq!(library.themes.len(), 4);
        assert_eq!(ids.len(), 4, "theme ids must be unique: {ids:?}");
        assert!(ids.contains("a-b"));
        assert!(ids.contains("a-b-2"));
        assert!(ids.contains("community-theme"));
        assert!(ids.contains("community-theme-2"));
    }

    #[test]
    fn save_never_overwrites_and_reports_the_new_id() {
        let dir = temp_dir("save");
        let first = save(&dir, "My Theme", r#"{"name":"One"}"#).unwrap();
        let second = save(&dir, "My Theme", r#"{"name":"Two"}"#).unwrap();

        assert_eq!(first.file, "my-theme.json");
        assert_eq!(first.id, "my-theme");
        assert_eq!(second.file, "my-theme-2.json");
        assert_eq!(second.id, "my-theme-2");
        assert_ne!(first.path, second.path);
    }

    #[test]
    fn save_rejects_invalid_or_non_object_json() {
        let dir = temp_dir("invalid");
        assert!(save(&dir, "bad", "{").unwrap_err().contains("Invalid JSON"));
        assert!(save(&dir, "array", "[]")
            .unwrap_err()
            .contains("JSON object"));
    }

    #[test]
    fn save_rejects_oversized_files() {
        let dir = temp_dir("large");
        let huge = format!(r#"{{"pad":"{}"}}"#, "x".repeat(MAX_THEME_BYTES as usize));
        assert!(save(&dir, "huge", &huge).unwrap_err().contains("too large"));
    }

    #[test]
    fn unsafe_names_are_slugified_and_unicode_falls_back() {
        assert_eq!(slugify("Neon  Nights!"), "neon-nights");
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert_eq!(slugify("日本語"), "community-theme");
        assert_eq!(slugify(""), "community-theme");
    }

    #[test]
    fn bundled_themes_are_valid_json_objects() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/themes");
        let library = discover(&dir);

        assert!(
            library.problems.is_empty(),
            "bundled theme files must stay valid: {:?}",
            library.problems
        );
        let ids: Vec<&str> = library
            .themes
            .iter()
            .map(|theme| theme.id.as_str())
            .collect();
        for expected in ["instrument", "midnight", "paper", "focus"] {
            assert!(ids.contains(&expected), "missing bundled theme {expected}");
        }
    }

    #[test]
    fn theme_ids_cannot_escape_the_namespace() {
        assert_eq!(theme_id_from_file("my theme!.json"), "my-theme");
        assert_eq!(theme_id_from_file(".hidden.json"), "hidden");
        assert_eq!(theme_id_from_file("日本.json"), "community-theme");
    }
}
