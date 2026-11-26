use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;
use serde::Serialize;

use crate::domain::{safe_component, BookDraft};
use crate::infrastructure::files::hash_file;
use crate::infrastructure::store::{self, StorePaths};

#[cfg(test)]
const SEND_FOLDER: &str = "Library Studio";
const UNKNOWN_AUTHOR: &str = "Nieznany autor";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderDevice {
    pub kind: String,
    pub name: String,
    pub root: String,
    pub formats: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderFile {
    pub relative_path: String,
    pub file_name: String,
    pub size: u64,
    pub format: String,
    pub already_in_catalog: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferNote {
    pub code: String,
    pub name: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferReport {
    pub copied: u32,
    pub skipped: u32,
    pub added: u32,
    pub notes: Vec<TransferNote>,
}

struct Profile {
    kind: &'static str,
    label: &'static str,
    scan: &'static str,
    send: &'static str,
    formats: &'static [&'static str],
}

const KINDLE: Profile = Profile {
    kind: "kindle",
    label: "Kindle",
    scan: "documents",
    send: "documents/Library Studio",
    formats: &["epub", "pdf", "mobi", "azw", "azw3", "azw4", "kfx", "txt", "doc", "docx", "html", "htm", "rtf", "prc"],
};
const KOBO: Profile = Profile {
    kind: "kobo",
    label: "Kobo",
    scan: "",
    send: "Library Studio",
    formats: &["epub", "kepub.epub", "pdf", "mobi", "cbz", "cbr"],
};
const POCKETBOOK: Profile = Profile {
    kind: "pocketbook",
    label: "PocketBook",
    scan: "",
    send: "Books/Library Studio",
    formats: &["epub", "pdf", "mobi", "azw", "azw3", "fb2", "fb2.zip", "txt", "djvu", "cbz", "cbr", "doc", "docx", "rtf", "html"],
};
const TOLINO: Profile = Profile {
    kind: "tolino",
    label: "Tolino",
    scan: "Books",
    send: "Books/Library Studio",
    formats: &["epub", "pdf"],
};
const BOOX: Profile = Profile {
    kind: "boox",
    label: "Onyx Boox",
    scan: "Books",
    send: "Books/Library Studio",
    formats: &["epub", "pdf", "mobi", "azw", "azw3", "fb2", "fb2.zip", "txt", "djvu", "cbz", "cbr", "doc", "docx"],
};

pub fn detect_readers() -> Vec<ReaderDevice> {
    drive_roots().into_iter().filter_map(|root| match_root(&root).map(|profile| device(&profile, &root))).collect()
}

pub fn device_from_path(path: &Path) -> Result<ReaderDevice, String> {
    if !path.is_dir() {
        return Err("Nie ma takiego folderu.".into());
    }
    if let Some(device) = find_reader(path) {
        return Ok(device);
    }
    if is_drive_root(path) {
        return Err("Ten dysk nie wygląda na czytnik. Wskaż folder z książkami.".into());
    }
    if blocked_path(path) {
        return Err("Tego folderu nie używam jako czytnika.".into());
    }
    Ok(ReaderDevice {
        kind: "folder".into(),
        name: path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| "Folder".into()),
        root: path.to_string_lossy().to_string(),
        formats: POCKETBOOK.formats.iter().map(|ext| (*ext).to_string()).collect(),
    })
}

pub fn list_books(conn: &Connection, root: &Path) -> Result<Vec<ReaderFile>, String> {
    let device = device_from_path(root)?;
    let profile = profile_for(&device);
    let scan = scan_dir(&device, profile);
    if !scan.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    // Skrót liczymy tylko dla plików, których rozmiar zgadza się z jakimś plikiem w katalogu.
    // Haszowanie wszystkiego na wolnym USB trwało minutami.
    let mut stmt = conn
        .prepare("SELECT DISTINCT d.file_size FROM digital_copies d JOIN book_editions e ON e.id = d.book_edition_id JOIN books b ON b.id = e.book_id WHERE b.deleted_at IS NULL")
        .map_err(crate::infrastructure::db::map_sql)?;
    let known_sizes: std::collections::HashSet<u64> = stmt
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(crate::infrastructure::db::map_sql)?
        .filter_map(|size| size.ok())
        .map(|size| size as u64)
        .collect();
    drop(stmt);
    let walk = walkdir::WalkDir::new(&scan).max_depth(8).into_iter().filter_entry(|entry| {
        entry.depth() == 0 || !entry.file_type().is_dir() || !skip_dir(&entry.file_name().to_string_lossy())
    });
    for entry in walk.filter_map(|entry| entry.ok()) {
        if files.len() >= 2000 || !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(format) = ebook_format(path) else { continue };
        if !known_ebook(&format) {
            continue;
        }
        let relative = path.strip_prefix(&scan).map_err(|e| e.to_string())?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        if relative.split('/').any(|part| part == ".." || part.starts_with('.')) {
            continue;
        }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        let already = known_sizes.contains(&size) && hash_file(path).ok().and_then(|hash| store::title_for_hash(conn, &hash).ok()).flatten().is_some();
        files.push(ReaderFile {
            relative_path: relative,
            file_name: path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default(),
            size,
            format,
            already_in_catalog: already,
        });
    }
    files.sort_by(|left, right| left.file_name.to_lowercase().cmp(&right.file_name.to_lowercase()));
    Ok(files)
}

pub fn send_books(conn: &Connection, paths: &StorePaths, root: &Path, book_ids: &[String], cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64, &str)) -> Result<TransferReport, String> {
    let device = device_from_path(root)?;
    let profile = profile_for(&device);
    let send = send_dir(&device, profile);
    fs::create_dir_all(&send).map_err(|e| e.to_string())?;
    let mut report = TransferReport { copied: 0, skipped: 0, added: 0, notes: Vec::new() };
    let total = book_ids.len() as u64;
    for (index, book_id) in book_ids.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let files = store::catalog_files(conn, book_id, paths)?;
        let title = files.first().map(|file| file.title.clone()).unwrap_or_else(|| book_id.clone());
        progress(index as u64, total, &title);
        if files.is_empty() {
            let missing = store::get_book(conn, book_id).map(|details| details.book.title).unwrap_or(title);
            report.notes.push(TransferNote { code: "no-file".into(), name: missing, detail: String::new() });
            continue;
        }
        for file in files.iter() {
            let format = normalize_format(&file.format, &file.absolute);
            if !profile.accepts(&format) {
                report.notes.push(TransferNote {
                    code: "bad-format".into(),
                    name: file.title.clone(),
                    detail: profile.formats.join(", "),
                });
                continue;
            }
            let name = destination_name(&file.author, &file.title, &format);
            match place_copy(&file.absolute, &send, &name) {
                Ok(true) => report.copied += 1,
                Ok(false) => report.skipped += 1,
                Err(error) => report.notes.push(TransferNote { code: "failed".into(), name: file.title.clone(), detail: error }),
            }
        }
    }
    progress(total, total, "");
    Ok(report)
}

pub fn import_books(conn: &Connection, paths: &StorePaths, root: &Path, relative_paths: &[String], cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64, &str)) -> Result<TransferReport, String> {
    let device = device_from_path(root)?;
    let profile = profile_for(&device);
    let scan = scan_dir(&device, profile);
    let mut report = TransferReport { copied: 0, skipped: 0, added: 0, notes: Vec::new() };
    let total = relative_paths.len() as u64;
    for (index, relative) in relative_paths.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        progress(index as u64, total, relative);
        let source = match resolve_inside(&scan, relative) {
            Ok(path) => path,
            Err(error) => {
                report.notes.push(TransferNote { code: "failed".into(), name: relative.clone(), detail: error });
                continue;
            }
        };
        if !source.is_file() {
            report.notes.push(TransferNote { code: "failed".into(), name: relative.clone(), detail: "Nie ma takiego pliku.".into() });
            continue;
        }
        let Some(_format) = ebook_format(&source).filter(|format| known_ebook(format)) else {
            report.notes.push(TransferNote { code: "bad-format".into(), name: relative.clone(), detail: String::new() });
            continue;
        };
        let hash = match hash_file(&source) {
            Ok(hash) => hash,
            Err(error) => {
                report.notes.push(TransferNote { code: "failed".into(), name: relative.clone(), detail: error });
                continue;
            }
        };
        if store::title_for_hash(conn, &hash)?.is_some() {
            report.skipped += 1;
            continue;
        }
        let file_name = source.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| "ksiazka".into());
        let (title, author) = split_ebook_name(&file_name);
        let saved = match store::save_book(conn, draft_for(&title, &author), paths, None) {
            Ok(saved) => saved,
            Err(error) => {
                report.notes.push(TransferNote { code: "failed".into(), name: title, detail: error });
                continue;
            }
        };
        match store::register_digital(conn, &saved.book_id, &source, paths) {
            Ok(_) => {
                report.added += 1;
                report.copied += 1;
            }
            Err(error) if error.contains("już przy książce") => report.skipped += 1,
            Err(error) => report.notes.push(TransferNote { code: "failed".into(), name: title, detail: error }),
        }
    }
    progress(total, total, "");
    Ok(report)
}

fn draft_for(title: &str, author: &str) -> BookDraft {
    BookDraft {
        id: None,
        title: title.to_string(),
        original_title: None,
        subtitle: None,
        authors: vec![author.to_string()],
        translators: Vec::new(),
        publisher: None,
        publication_year: None,
        language: None,
        description: None,
        series_name: None,
        series_number: None,
        genres: Vec::new(),
        tags: Vec::new(),
        isbn10: None,
        isbn13: None,
        page_count: None,
        cover_url: None,
        metadata_source: None,
        external_id: None,
        edition_id: None,
        average_rating: None,
        rating_source: None,
    }
}

pub fn split_ebook_name(file_name: &str) -> (String, String) {
    let mut stem = file_name.trim();
    let lower = stem.to_lowercase();
    for suffix in [".kepub.epub", ".fb2.zip", ".azw3", ".azw4", ".epub", ".mobi", ".pdf", ".azw", ".kfx", ".djvu", ".docx", ".html", ".txt", ".doc", ".fb2", ".cbz", ".cbr", ".rtf", ".prc", ".htm"] {
        if lower.ends_with(suffix) {
            stem = &stem[..stem.len() - suffix.len()];
            break;
        }
    }
    let stem = stem.trim().trim_matches(|ch: char| ch == '.' || ch == ' ');
    if let Some((author, title)) = stem.split_once(" - ") {
        let author = author.trim();
        let title = title.trim();
        if !author.is_empty() && !title.is_empty() {
            return (title.to_string(), author.to_string());
        }
    }
    let title = if stem.is_empty() { "Bez tytułu".into() } else { stem.to_string() };
    (title, UNKNOWN_AUTHOR.into())
}

fn destination_name(author: &str, title: &str, format: &str) -> String {
    let base = safe_component(&format!("{author} - {title}"));
    format!("{base}.{format}")
}

fn place_copy(source: &Path, directory: &Path, name: &str) -> Result<bool, String> {
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err("Ścieżka wychodzi poza czytnik.".into());
    }
    let source_hash = hash_file(source)?;
    let mut candidate = directory.join(name);
    if candidate.exists() {
        if hash_file(&candidate).ok().as_deref() == Some(source_hash.as_str()) {
            return Ok(false);
        }
        let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "ksiazka".into());
        let ext = ebook_format(Path::new(name)).unwrap_or_else(|| "epub".into());
        for number in 2..50 {
            candidate = directory.join(format!("{stem} {number}.{ext}"));
            if !candidate.exists() {
                break;
            }
            if hash_file(&candidate).ok().as_deref() == Some(source_hash.as_str()) {
                return Ok(false);
            }
        }
        if candidate.exists() {
            return Err("Na czytniku nie ma wolnej nazwy pliku.".into());
        }
    }
    if let Some(parent) = candidate.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(source, &candidate).map_err(|e| e.to_string())?;
    Ok(true)
}

fn resolve_inside(base: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute() || relative_path.components().any(|part| matches!(part, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
        return Err("Ścieżka wychodzi poza czytnik.".into());
    }
    Ok(base.join(relative_path))
}

fn profile_for(device: &ReaderDevice) -> &'static Profile {
    match device.kind.as_str() {
        "kindle" => &KINDLE,
        "kobo" => &KOBO,
        "pocketbook" => &POCKETBOOK,
        "tolino" => &TOLINO,
        "boox" => &BOOX,
        _ => &POCKETBOOK,
    }
}

fn scan_dir(device: &ReaderDevice, profile: &Profile) -> PathBuf {
    let root = PathBuf::from(&device.root);
    if device.kind == "folder" || profile.scan.is_empty() {
        return root;
    }
    root.join(profile.scan)
}

fn send_dir(device: &ReaderDevice, profile: &Profile) -> PathBuf {
    let root = PathBuf::from(&device.root);
    if device.kind == "folder" {
        root
    } else {
        root.join(profile.send)
    }
}

impl Profile {
    fn accepts(&self, format: &str) -> bool {
        self.formats.iter().any(|item| item.eq_ignore_ascii_case(format))
    }
}

fn device(profile: &Profile, root: &Path) -> ReaderDevice {
    let mut volume = volume_label(root);
    if calibre_location(root).is_some_and(|code| code != "main") {
        volume = if volume.is_empty() { "karta pamięci".into() } else { format!("{volume}, karta pamięci") };
    }
    let shown = root.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
    let name = if volume.is_empty() {
        format!("{} ({shown})", profile.label)
    } else {
        format!("{} · {volume} ({shown})", profile.label)
    };
    ReaderDevice {
        kind: profile.kind.into(),
        name,
        root: root.to_string_lossy().to_string(),
        formats: profile.formats.iter().map(|ext| (*ext).to_string()).collect(),
    }
}

fn find_reader(path: &Path) -> Option<ReaderDevice> {
    let mut current = Some(path);
    for _ in 0..6 {
        let Some(dir) = current else { break };
        if let Some(profile) = match_root(dir) {
            return Some(device(profile, dir));
        }
        current = dir.parent();
    }
    None
}

fn match_root(root: &Path) -> Option<&'static Profile> {
    let documents = root.join("documents").is_dir();
    let kindle_system = root.join("system").join("version.txt").is_file()
        || root.join("DONT_HALT_ON_REBOOT").exists()
        || root.join("system").join("com.amazon.ebook.booklet.reader").is_dir();
    if documents && kindle_system {
        return Some(&KINDLE);
    }
    if root.join(".tolino").is_dir() {
        return Some(&TOLINO);
    }
    if root.join(".kobo").is_dir() {
        if volume_label(root).to_lowercase().contains("tolino") {
            return Some(&TOLINO);
        }
        return Some(&KOBO);
    }
    let system = root.join("system");
    if system.join(".pocketbook").exists()
        || system.join("state.cfg").exists()
        || system.join("config").join("device.cfg").exists()
        || system.join("explorer-3").is_dir()
        || has_pocketbook_icon(&system)
    {
        return Some(&POCKETBOOK);
    }
    let books = root.join("Books").is_dir();
    let boox = root.join("Note").is_dir() || root.join("Screenshots").is_dir() || root.join("Dictionary").is_dir();
    if books && boox {
        return Some(&BOOX);
    }
    calibre_profile(root)
}

fn has_pocketbook_icon(system: &Path) -> bool {
    let Ok(entries) = fs::read_dir(system) else { return false };
    entries.flatten().any(|entry| {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        name.starts_with("device") && name.ends_with(".ico")
    })
}

/// Calibre zostawia na każdym nośniku czytnika (także na karcie SD) plik
/// `driveinfo.calibre` z nazwą urządzenia. To najpewniejszy ślad, gdy sam
/// czytnik nie ma charakterystycznych folderów.
fn calibre_profile(root: &Path) -> Option<&'static Profile> {
    let text = fs::read_to_string(root.join("driveinfo.calibre")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let name = value.get("device_name")?.as_str()?.to_lowercase();
    profile_by_name(&name)
}

fn profile_by_name(name: &str) -> Option<&'static Profile> {
    if name.contains("kindle") {
        Some(&KINDLE)
    } else if name.contains("tolino") {
        Some(&TOLINO)
    } else if name.contains("kobo") {
        Some(&KOBO)
    } else if name.contains("pocketbook") || name.contains("inkpad") {
        Some(&POCKETBOOK)
    } else if name.contains("boox") || name.contains("onyx") {
        Some(&BOOX)
    } else {
        None
    }
}

fn calibre_location(root: &Path) -> Option<String> {
    let text = fs::read_to_string(root.join("driveinfo.calibre")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("location_code")?.as_str().map(str::to_string)
}

fn ebook_format(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    if name.ends_with(".kepub.epub") {
        return Some("kepub.epub".into());
    }
    if name.ends_with(".fb2.zip") {
        return Some("fb2.zip".into());
    }
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if ext.is_empty() { None } else { Some(ext) }
}

fn normalize_format(stored: &str, path: &Path) -> String {
    ebook_format(path).unwrap_or_else(|| stored.trim().to_lowercase())
}

fn known_ebook(format: &str) -> bool {
    KINDLE.accepts(format) || KOBO.accepts(format) || POCKETBOOK.accepts(format) || TOLINO.accepts(format) || BOOX.accepts(format)
}

fn skip_dir(name: &str) -> bool {
    let value = name.to_lowercase();
    value.starts_with('.')
        || value.ends_with(".sdr")
        || matches!(
            value.as_str(),
            "system" | "fonts" | "dictionaries" | "dictionary" | "screensavers" | "applications" | "notes" | "note" | "screenshots" | "audible"
        )
}

fn is_drive_root(path: &Path) -> bool {
    path.parent().is_none() || path.components().count() <= 2 && path.components().any(|part| matches!(part, Component::Prefix(_)))
}

fn blocked_path(path: &Path) -> bool {
    let text = path.to_string_lossy().to_lowercase();
    if let Ok(windows) = std::env::var("WINDIR") {
        if text.starts_with(&windows.to_lowercase()) {
            return true;
        }
    }
    text.contains("\\windows\\") || text.contains("\\program files") || text.contains("\\program files (x86)")
}

fn drive_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        (b'A'..=b'Z')
            .filter_map(|letter| {
                let root = format!("{}:\\", letter as char);
                let wide: Vec<u16> = std::os::windows::ffi::OsStrExt::encode_wide(std::ffi::OsStr::new(&root)).chain(std::iter::once(0)).collect();
                let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
                if kind == 2 || kind == 3 { Some(PathBuf::from(root)) } else { None }
            })
            .filter(|root| root.is_dir())
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn volume_label(root: &Path) -> String {
    #[cfg(windows)]
    {
        if !is_drive_root(root) {
            return String::new();
        }
        let wide: Vec<u16> = std::os::windows::ffi::OsStrExt::encode_wide(root.as_os_str()).chain(std::iter::once(0)).collect();
        let mut buffer = [0u16; 64];
        let ok = unsafe {
            GetVolumeInformationW(wide.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0)
        };
        if ok == 0 {
            return String::new();
        }
        let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
        return String::from_utf16_lossy(&buffer[..end]);
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        String::new()
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetVolumeInformationW(root: *const u16, name: *mut u16, name_size: u32, serial: *mut u32, max_component: *mut u32, flags: *mut u32, fs_name: *mut u16, fs_name_size: u32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SettingsPatch;
    use crate::infrastructure::db::Database;

    fn touch_dir(path: &Path) {
        fs::create_dir_all(path).unwrap();
    }

    #[test]
    fn detects_popular_readers_and_ignores_a_plain_documents_folder() {
        let root = std::env::temp_dir().join(format!("bib-reader-{}", uuid::Uuid::new_v4()));
        let kindle = root.join("kindle");
        touch_dir(&kindle.join("documents"));
        touch_dir(&kindle.join("system"));
        fs::write(kindle.join("system").join("version.txt"), b"1").unwrap();
        assert_eq!(match_root(&kindle).unwrap().kind, "kindle");

        let plain = root.join("plain");
        touch_dir(&plain.join("documents"));
        assert!(match_root(&plain).is_none());

        let kobo = root.join("kobo");
        touch_dir(&kobo.join(".kobo"));
        assert_eq!(match_root(&kobo).unwrap().kind, "kobo");

        let pocket = root.join("pocket");
        touch_dir(&pocket.join("system"));
        fs::write(pocket.join("system").join("state.cfg"), b"").unwrap();
        assert_eq!(match_root(&pocket).unwrap().kind, "pocketbook");

        let tolino = root.join("tolino");
        touch_dir(&tolino.join(".tolino"));
        assert_eq!(match_root(&tolino).unwrap().kind, "tolino");

        let boox = root.join("boox");
        touch_dir(&boox.join("Books"));
        touch_dir(&boox.join("Note"));
        assert_eq!(match_root(&boox).unwrap().kind, "boox");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_pocketbook_touch_lux_and_its_sd_card() {
        let dir = std::env::temp_dir().join(format!("bib-pb-{}", uuid::Uuid::new_v4()));
        let main = dir.as_path().join("pb");
        touch_dir(&main.join("system"));
        touch_dir(&main.join("Books"));
        fs::write(main.join("system").join(".pocketbook"), b"").unwrap();
        assert_eq!(match_root(&main).unwrap().kind, "pocketbook");

        let card = dir.as_path().join("card");
        touch_dir(&card.join("Books"));
        fs::write(card.join("driveinfo.calibre"), br#"{"device_name": "PocketBook Touch Lux 2", "location_code": "A"}"#).unwrap();
        assert_eq!(match_root(&card).unwrap().kind, "pocketbook");
        assert!(device(match_root(&card).unwrap(), &card).name.contains("karta"));

        let other = dir.as_path().join("other");
        touch_dir(&other.join("system"));
        assert!(match_root(&other).is_none());
    }

    #[test]
    fn splits_calibre_file_names() {
        assert_eq!(split_ebook_name("Sapkowski, Andrzej - Ostatnie życzenie.epub").0, "Ostatnie życzenie");
        assert_eq!(split_ebook_name("Sapkowski, Andrzej - Ostatnie życzenie.epub").1, "Sapkowski, Andrzej");
        assert_eq!(split_ebook_name("Notatki.pdf").1, UNKNOWN_AUTHOR);
    }

    #[test]
    fn rejects_paths_that_leave_the_reader() {
        let base = Path::new("E:\\documents");
        assert!(resolve_inside(base, "../system/version.txt").is_err());
        assert!(resolve_inside(base, "C:\\Windows\\notepad.exe").is_err());
        assert_eq!(resolve_inside(base, "Library Studio/a.epub").unwrap(), base.join("Library Studio").join("a.epub"));
    }

    #[test]
    fn sends_epub_to_kindle_and_refuses_a_tolino_mobi() {
        let root = std::env::temp_dir().join(format!("bib-send-{}", uuid::Uuid::new_v4()));
        let library = root.join("library");
        let kindle = root.join("kindle");
        let tolino = root.join("tolino");
        touch_dir(&kindle.join("documents"));
        fs::create_dir_all(kindle.join("system")).unwrap();
        fs::write(kindle.join("system").join("version.txt"), b"1").unwrap();
        touch_dir(&tolino.join(".tolino"));
        touch_dir(&tolino.join("Books"));
        let db = Database::open(&root.join("t.sqlite")).unwrap();
        let source = root.join("book.epub");
        fs::write(&source, b"epub-bytes").unwrap();
        let mobi = root.join("book.mobi");
        fs::write(&mobi, b"mobi-bytes").unwrap();
        db.with(|conn| {
            store::save_settings(conn, SettingsPatch {
                library_dir: Some(library.to_string_lossy().to_string()),
                theme: None, open_library: None, google_books: None, national_library: None, lubimy_czytac: None, upolujebooka: None, locale: None,
                auto_backup: None, backup_dir: None, sync_provider: None, sync_dir: None, webdav_url: None, webdav_user: None, reading_goal: None,
            })?;
            let paths = StorePaths { library: Some(library.clone()), covers: root.join("covers") };
            let saved = store::save_book(conn, draft_for("Pan Tadeusz", "Mickiewicz, Adam"), &paths, None)?;
            store::register_digital(conn, &saved.book_id, &source, &paths)?;
            let cancel = AtomicBool::new(false);
            let mut ticks = Vec::new();
            let report = send_books(conn, &paths, &kindle, &[saved.book_id.clone()], &cancel, &mut |c, t, n| ticks.push((c, t, n.to_string())))?;
            assert_eq!(report.copied, 1);
            assert!(kindle.join("documents").join(SEND_FOLDER).join("Mickiewicz, Adam - Pan Tadeusz.epub").is_file());
            let again = send_books(conn, &paths, &kindle, &[saved.book_id.clone()], &cancel, &mut |_, _, _| {})?;
            assert_eq!(again.skipped, 1);
            store::register_digital(conn, &saved.book_id, &mobi, &paths)?;
            let refused = send_books(conn, &paths, &tolino, &[saved.book_id], &cancel, &mut |_, _, _| {})?;
            assert!(refused.notes.iter().any(|note| note.code == "bad-format"));
            assert!(tolino.join("Books").join(SEND_FOLDER).join("Mickiewicz, Adam - Pan Tadeusz.epub").is_file());
            Ok(())
        }).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn imports_a_reader_file_into_the_catalog() {
        let root = std::env::temp_dir().join(format!("bib-import-{}", uuid::Uuid::new_v4()));
        let library = root.join("library");
        let kindle = root.join("kindle");
        touch_dir(&kindle.join("documents"));
        fs::create_dir_all(kindle.join("system")).unwrap();
        fs::write(kindle.join("system").join("version.txt"), b"1").unwrap();
        let book = kindle.join("documents").join("Lem, Stanisław - Solaris.epub");
        fs::write(&book, b"solaris").unwrap();
        let db = Database::open(&root.join("t.sqlite")).unwrap();
        db.with(|conn| {
            store::save_settings(conn, SettingsPatch {
                library_dir: Some(library.to_string_lossy().to_string()),
                theme: None, open_library: None, google_books: None, national_library: None, lubimy_czytac: None, upolujebooka: None, locale: None,
                auto_backup: None, backup_dir: None, sync_provider: None, sync_dir: None, webdav_url: None, webdav_user: None, reading_goal: None,
            })?;
            let paths = StorePaths { library: Some(library), covers: root.join("covers") };
            let cancel = AtomicBool::new(false);
            let report = import_books(conn, &paths, &kindle, &["Lem, Stanisław - Solaris.epub".into()], &cancel, &mut |_, _, _| {})?;
            assert_eq!(report.added, 1);
            let listed = list_books(conn, &kindle)?;
            assert!(listed.iter().any(|file| file.already_in_catalog && file.file_name.contains("Solaris")));
            let again = import_books(conn, &paths, &kindle, &["../system/version.txt".into()], &cancel, &mut |_, _, _| {})?;
            assert!(again.notes.iter().any(|note| note.code == "failed"));
            Ok(())
        }).unwrap();
        let _ = fs::remove_dir_all(root);
    }
}

