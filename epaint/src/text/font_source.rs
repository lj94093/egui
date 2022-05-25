use std::path::{Path, PathBuf};

use crate::FontFamily;

// FIXME(pcwalton): These could expand to multiple fonts, and they could be language-specific.
#[cfg(any(target_family = "windows", target_os = "macos", target_os = "ios"))]
pub const DEFAULT_FONT_FAMILY_SERIF: &'static str = "Times New Roman";
#[cfg(any(target_family = "windows", target_os = "macos", target_os = "ios"))]
pub const DEFAULT_FONT_FAMILY_SANS_SERIF: &'static str = "Arial";
#[cfg(any(target_family = "windows", target_os = "macos", target_os = "ios"))]
pub const DEFAULT_FONT_FAMILY_MONOSPACE: &'static str = "Courier New";
#[cfg(any(target_family = "windows", target_os = "macos", target_os = "ios"))]
pub const DEFAULT_FONT_FAMILY_CURSIVE: &'static str = "Comic Sans MS";
#[cfg(target_family = "windows")]
pub const DEFAULT_FONT_FAMILY_FANTASY: &'static str = "Impact";
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub const DEFAULT_FONT_FAMILY_FANTASY: &'static str = "Papyrus";

#[cfg(not(any(target_family = "windows", target_os = "macos", target_os = "ios")))]
pub const DEFAULT_FONT_FAMILY_SERIF: &'static str = "serif";
#[cfg(not(any(target_family = "windows", target_os = "macos", target_os = "ios")))]
pub const DEFAULT_FONT_FAMILY_SANS_SERIF: &'static str = "sans-serif";
#[cfg(not(any(target_family = "windows", target_os = "macos", target_os = "ios")))]
pub const DEFAULT_FONT_FAMILY_MONOSPACE: &'static str = "monospace";
#[cfg(not(any(target_family = "windows", target_os = "macos", target_os = "ios")))]
pub const DEFAULT_FONT_FAMILY_CURSIVE: &'static str = "cursive";
#[cfg(not(any(target_family = "windows", target_os = "macos", target_os = "ios")))]
pub const DEFAULT_FONT_FAMILY_FANTASY: &'static str = "fantasy";

#[cfg(target_os = "android")]
fn default_font_directories() -> Vec<PathBuf> {
    vec![PathBuf::from("/system/fonts")]
}

#[cfg(target_family = "windows")]
fn default_font_directories() -> Vec<PathBuf> {
    // Because of the forbid of unsafe code,we can't call winapi GetWindowsDirectoryW to get the windows directory path.
    // So we hard code with C:\Windows which means we don't support windows directory in d~z.
    let fonts_path = PathBuf::from(r"C:\Windows\Fonts\");
    vec![fonts_path]
}

#[cfg(target_os = "macos")]
fn default_font_directories() -> Vec<PathBuf> {
    let mut directories = vec![
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
        PathBuf::from("/Network/Library/Fonts"),
    ];
    if let Some(mut path) = dirs_next::home_dir() {
        path.push("Library");
        path.push("Fonts");
        directories.push(path);
    }
    directories
}

#[cfg(not(any(target_os = "android", target_family = "windows", target_os = "macos")))]
fn default_font_directories() -> Vec<PathBuf> {
    let mut directories = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
        PathBuf::from("/var/run/host/usr/share/fonts"), // Flatpak specific
        PathBuf::from("/var/run/host/usr/local/share/fonts"),
    ];
    if let Some(path) = dirs_next::home_dir() {
        directories.push(path.join(".fonts")); // ~/.fonts is deprecated
        directories.push(path.join("local").join("share").join("fonts")); // Flatpak specific
    }
    if let Some(mut path) = dirs_next::data_dir() {
        path.push("fonts");
        directories.push(path);
    }
    directories
}

pub fn get_system_default_font_path(family: FontFamily) -> Option<PathBuf> {
    let mut font_directories: Vec<PathBuf> = default_font_directories();

    for mut font_path in font_directories {
        let font_name = match family {
            FontFamily::Monospace => DEFAULT_FONT_FAMILY_MONOSPACE.to_owned(),
            FontFamily::Proportional => DEFAULT_FONT_FAMILY_SERIF.to_owned(),
            FontFamily::Name(ref name) => name.as_ref().to_owned(),
        };
        font_path.set_file_name(font_name);
        if font_path.exists() {
            return Some(font_path);
        }
    }
    None
}
