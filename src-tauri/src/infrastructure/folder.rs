//! Dodawanie e-booków z dowolnego folderu na komputerze: podgląd (tytuł i autor z metadanych pliku)
//! i import zaznaczonych plików do biblioteki. Pliki są kopiowane, oryginały zostają na miejscu.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;
use serde::Serialize;

use crate::domain::BookDraft;
use crate::infrastructure::ebook_meta;
use crate::infrastructure::files::{hash_file, path_within, CANCELLED};
use crate::infrastructure::importer::{draft_from_row, parse_opf, ImportRow};
use crate::infrastructure::reader::{
    blocked_path, draft_for, ebook_format, guess_name, known_ebook, resolve_inside, skip_dir, TransferNote, TransferReport,
};
use crate::infrastructure::store::{self, StorePaths};

const MAX_FILES: usize = 3000;
const MAX_DEPTH: usize = 12;
const COVER_LIMIT: u64 = 8_000_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderFile {
    pub relative_path: String,
    pub file_name: String,
    pub size: u64,
    pub format: String,
    pub title: String,
    pub authors: Vec<String>,
    pub already_in_catalog: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderScan {
    pub root: String,
    pub files: Vec<FolderFile>,
    /// Plików było więcej niż limit; pokazujemy tylko pierwsze.
    pub truncated: bool,
}

/// Sprawdza, czy folder nadaje się do przejrzenia: istnieje, nie jest folderem systemu ani całym dyskiem systemowym.
fn check_root(root: &Path) -> Result<PathBuf, String> {
    if !root.is_dir() {
        return Err("Nie ma takiego folderu.".into());
    }
    if blocked_path(root) {
        return Err("Tego folderu nie przeglądam.".into());
    }
    if let Ok(system) = std::env::var("SystemDrive") {
        let text = root.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
        if text == system.to_lowercase() {
            return Err("Wskaż folder z książkami, a nie cały dysk systemowy.".into());
        }
    }
    Ok(root.to_path_buf())
}

/// Metadane pliku: folder biblioteki Calibre (metadata.opf obok), indeks metadata.calibre (gdy folder to
/// kopia czytnika), dane OPF wewnątrz EPUB-a. `None` znaczy: zostaje tylko nazwa pliku.
fn file_row(path: &Path, root: &Path, calibre: &HashMap<String, ImportRow>) -> Option<ImportRow> {
    if let Some(dir) = path.parent() {
        let opf = dir.join("metadata.opf");
        if opf.is_file() && opf.metadata().map(|meta| meta.len() < 2_000_000).unwrap_or(false) {
            if let Some(mut row) = fs::read_to_string(&opf).ok().and_then(|text| parse_opf(&text, Path::new("\u{0}"))) {
                row.files.clear();
                row.cover_file = dir.join("cover.jpg").is_file().then(|| dir.join("cover.jpg").display().to_string());
                return Some(row);
            }
        }
    }
    if let Ok(relative) = path.strip_prefix(root) {
        if let Some(row) = calibre.get(&ebook_meta::lpath_key(&relative.to_string_lossy())) {
            return Some(row.clone());
        }
    }
    ebook_meta::from_epub(path)
}

fn draft_for_file(path: &Path, root: &Path, calibre: &HashMap<String, ImportRow>) -> (BookDraft, Option<PathBuf>) {
    match file_row(path, root, calibre) {
        Some(row) => {
            let mut draft = draft_from_row(&row, "file");
            if row.authors.is_empty() {
                draft.authors = vec![guess_name(path).1];
            }
            (draft, row.cover_file.map(PathBuf::from))
        }
        None => {
            let (title, author) = guess_name(path);
            let mut draft = draft_for(&title, &author);
            draft.metadata_source = Some("file".into());
            (draft, None)
        }
    }
}

fn known_sizes(conn: &Connection) -> Result<HashSet<u64>, String> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT d.file_size FROM digital_copies d JOIN book_editions e ON e.id = d.book_edition_id JOIN books b ON b.id = e.book_id WHERE b.deleted_at IS NULL")
        .map_err(crate::infrastructure::db::map_sql)?;
    let sizes = stmt
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(crate::infrastructure::db::map_sql)?
        .filter_map(|size| size.ok())
        .map(|size| size as u64)
        .collect();
    Ok(sizes)
}

fn walk(root: &Path, library: Option<&Path>) -> (Vec<PathBuf>, bool) {
    let mut out = Vec::new();
    let mut truncated = false;
    let walker = walkdir::WalkDir::new(root).max_depth(MAX_DEPTH).into_iter().filter_entry(|entry| {
        if entry.depth() == 0 || !entry.file_type().is_dir() {
            return true;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        // Folder biblioteki pomijamy: te pliki już są w katalogu.
        !skip_dir(&name) && !matches!(name.as_str(), "node_modules" | "$recycle.bin" | "appdata") && library.is_none_or(|lib| !path_within(lib, entry.path()))
    });
    for entry in walker.filter_map(|entry| entry.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(format) = ebook_format(entry.path()) else { continue };
        if !known_ebook(&format) {
            continue;
        }
        if out.len() >= MAX_FILES {
            truncated = true;
            break;
        }
        out.push(entry.path().to_path_buf());
    }
    (out, truncated)
}

pub fn scan(conn: &Connection, paths: &StorePaths, root: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64, &str)) -> Result<FolderScan, String> {
    let root = check_root(root)?;
    let (found, truncated) = walk(&root, paths.library.as_deref());
    let calibre = ebook_meta::calibre_index(&root);
    let sizes = known_sizes(conn)?;
    let total = found.len() as u64;
    let mut files = Vec::new();
    for (index, path) in found.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let file_name = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
        if index % 25 == 0 {
            progress(index as u64, total, &file_name);
        }
        let relative = path.strip_prefix(&root).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
        let size = path.metadata().map(|meta| meta.len()).unwrap_or(0);
        let already = sizes.contains(&size) && hash_file(path).ok().and_then(|hash| store::title_for_hash(conn, &hash).ok()).flatten().is_some();
        let (draft, _) = draft_for_file(path, &root, &calibre);
        files.push(FolderFile {
            relative_path: relative,
            file_name,
            size,
            format: ebook_format(path).unwrap_or_default(),
            title: draft.title,
            authors: draft.authors,
            already_in_catalog: already,
        });
    }
    progress(total, total, "");
    files.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()).then_with(|| a.relative_path.cmp(&b.relative_path)));
    Ok(FolderScan { root: root.display().to_string(), files, truncated })
}

fn read_cover(path: &Path) -> Option<Vec<u8>> {
    if path.metadata().ok()?.len() > COVER_LIMIT {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    crate::infrastructure::metadata::image_type(&bytes).map(|_| bytes)
}

pub fn import(conn: &Connection, paths: &StorePaths, root: &Path, relative_paths: &[String], cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64, &str)) -> Result<TransferReport, String> {
    let root = check_root(root)?;
    if paths.library.is_none() {
        return Err("Najpierw wybierz folder biblioteki w ustawieniach.".into());
    }
    let calibre = ebook_meta::calibre_index(&root);
    let mut report = TransferReport { copied: 0, skipped: 0, added: 0, notes: Vec::new() };
    let total = relative_paths.len() as u64;
    for (index, relative) in relative_paths.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        progress(index as u64, total, relative);
        let source = match resolve_inside(&root, relative) {
            Ok(path) if path.is_file() => path,
            Ok(_) => {
                report.notes.push(TransferNote { code: "failed".into(), name: relative.clone(), detail: "Nie ma takiego pliku.".into() });
                continue;
            }
            Err(error) => {
                report.notes.push(TransferNote { code: "failed".into(), name: relative.clone(), detail: error });
                continue;
            }
        };
        if !ebook_format(&source).is_some_and(|format| known_ebook(&format)) {
            report.notes.push(TransferNote { code: "bad-format".into(), name: relative.clone(), detail: String::new() });
            continue;
        }
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
        let (draft, cover_file) = draft_for_file(&source, &root, &calibre);
        let title = draft.title.clone();
        let cover = cover_file.as_deref().and_then(read_cover);
        // Gdy taka książka już jest (ten sam tytuł i autor albo ISBN), plik trafia do niej jako kolejny format.
        let saved = match store::save_book(conn, draft, paths, cover) {
            Ok(saved) => saved,
            Err(error) => {
                report.notes.push(TransferNote { code: "failed".into(), name: title, detail: error });
                continue;
            }
        };
        match store::register_digital(conn, &saved.book_id, &source, paths) {
            Ok(_) => {
                report.copied += 1;
                if saved.created {
                    report.added += 1;
                }
            }
            Err(error) if error.contains("już przy książce") => report.skipped += 1,
            Err(error) => report.notes.push(TransferNote { code: "failed".into(), name: title, detail: error }),
        }
    }
    progress(total, total, "");
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db::Database;

    #[test]
    fn scans_a_folder_and_imports_picked_files() {
        let dir = std::env::temp_dir().join(format!("bib-folder-{}", uuid::Uuid::new_v4()));
        let source = dir.join("Moje e-booki");
        fs::create_dir_all(source.join("Lem, Stanisław")).unwrap();
        fs::create_dir_all(source.join(".ukryte")).unwrap();
        fs::write(source.join("Lem, Stanisław").join("Solaris - Stanisław Lem.epub"), b"epub-1").unwrap();
        fs::write(source.join("Mickiewicz, Adam - Pan Tadeusz.pdf"), b"pdf-1").unwrap();
        fs::write(source.join("notatki.docx.tmp"), b"x").unwrap();
        fs::write(source.join(".ukryte").join("Ukryta - Ktoś.epub"), b"x").unwrap();
        let library = dir.join("biblioteka");
        fs::create_dir_all(&library).unwrap();
        let db = Database::open(&dir.join("t.sqlite")).unwrap();
        let paths = StorePaths { library: Some(library.clone()), covers: dir.join("covers") };
        let never = AtomicBool::new(false);
        db.with(|conn| {
            let found = scan(conn, &paths, &source, &never, &mut |_, _, _| {})?;
            assert_eq!(found.files.len(), 2, "{:?}", found.files);
            let solaris = found.files.iter().find(|f| f.title == "Solaris").unwrap();
            assert_eq!(solaris.authors, vec!["Stanisław Lem".to_string()]);
            let tadeusz = found.files.iter().find(|f| f.title == "Pan Tadeusz").unwrap();
            assert_eq!(tadeusz.authors, vec!["Mickiewicz, Adam".to_string()]);
            let picked: Vec<String> = found.files.iter().map(|f| f.relative_path.clone()).collect();
            let report = import(conn, &paths, &source, &picked, &never, &mut |_, _, _| {})?;
            assert_eq!(report.added, 2);
            assert!(source.join("Mickiewicz, Adam - Pan Tadeusz.pdf").is_file(), "oryginał zostaje");
            let again = scan(conn, &paths, &source, &never, &mut |_, _, _| {})?;
            assert!(again.files.iter().all(|f| f.already_in_catalog));
            let repeat = import(conn, &paths, &source, &picked, &never, &mut |_, _, _| {})?;
            assert_eq!((repeat.added, repeat.skipped), (0, 2));
            let escape = import(conn, &paths, &source, &["../t.sqlite".into()], &never, &mut |_, _, _| {})?;
            assert_eq!(escape.notes.len(), 1);
            Ok(())
        })
        .unwrap();
        let _ = fs::remove_dir_all(dir);
    }
}
