use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::domain::safe_component;

/// Flaga dla operacji, których nie da się anulować.
#[allow(dead_code)]
pub static NEVER: AtomicBool = AtomicBool::new(false);

pub const CANCELLED: &str = "Przerwano.";

pub fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("Nie można otworzyć pliku: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Plik tymczasowy obok celu: `nazwa.epub.part`, więc dwa pliki z tym samym rdzeniem nie kolidują.
fn temp_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "plik".into());
    name.push(".part");
    target.with_file_name(name)
}

pub fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = temp_path(target);
    {
        let mut file = File::create(&tmp).map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    // fs::rename w Windows zastępuje istniejący plik, więc nie ma chwili bez pliku docelowego.
    fs::rename(&tmp, target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Kopiuje strumieniowo, bez wczytywania całego pliku do pamięci.
pub fn copy_atomic(source: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = temp_path(target);
    let result = (|| -> Result<(), String> {
        let mut input = File::open(source).map_err(|e| format!("Nie można odczytać pliku: {e}"))?;
        let mut output = File::create(&tmp).map_err(|e| e.to_string())?;
        io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    fs::rename(&tmp, target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

pub fn book_directory(library: &Path, author: &str, title: &str, book_id: &str) -> Result<PathBuf, String> {
    let author = safe_component(author);
    let title = safe_component(title);
    let direct = library.join(&author).join(&title);
    if directory_belongs_to(&direct, book_id)? || !direct.exists() {
        return Ok(direct);
    }
    let short: String = book_id.chars().take(8).collect();
    Ok(library.join(author).join(format!("{title} {short}")))
}

fn directory_belongs_to(dir: &Path, book_id: &str) -> Result<bool, String> {
    let marker = dir.join("metadata.json");
    if !marker.exists() {
        return Ok(!dir.exists() || fs::read_dir(dir).map_err(|e| e.to_string())?.next().is_none());
    }
    let text = fs::read_to_string(marker).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({}));
    Ok(value.get("bookId").and_then(|id| id.as_str()) == Some(book_id))
}

/// Plik docelowy jest aktualny, gdy ma ten sam rozmiar i nie jest starszy od źródła.
/// Dzięki temu kopia i synchronizacja nie liczą skrótu SHA-256 każdego pliku za każdym razem.
pub fn up_to_date(source: &Path, target: &Path) -> bool {
    let (Ok(left), Ok(right)) = (fs::metadata(source), fs::metadata(target)) else {
        return false;
    };
    if left.len() != right.len() {
        return false;
    }
    match (left.modified(), right.modified()) {
        (Ok(source_time), Ok(target_time)) => target_time >= source_time,
        _ => false,
    }
}

pub fn copy_tree(source: &Path, destination: &Path, progress: &mut dyn FnMut(u64, u64, &str), cancel: &AtomicBool) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    let files: Vec<_> = walkdir::WalkDir::new(source)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| !entry.file_name().to_string_lossy().ends_with(".part"))
        .collect();
    let total = files.len() as u64;
    for (index, entry) in files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let relative = entry.path().strip_prefix(source).map_err(|e| e.to_string())?;
        let target = destination.join(relative);
        if !up_to_date(entry.path(), &target) {
            copy_atomic(entry.path(), &target)?;
        }
        progress(index as u64 + 1, total, &relative.to_string_lossy());
    }
    Ok(())
}

/// Ścieżka względna z danych z zewnątrz (eksport, synchronizacja). Odrzuca ścieżki bezwzględne,
/// litery dysków i wyjście w górę katalogu, żeby obcy plik nie mógł zapisać nic poza biblioteką.
pub fn safe_relative(raw: &str) -> Option<PathBuf> {
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') || normalized.contains(':') {
        return None;
    }
    let mut clean = PathBuf::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if clean.as_os_str().is_empty() {
        None
    } else {
        Some(clean)
    }
}

fn comparable(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\");
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text).to_string();
    text.trim_end_matches('\\').to_lowercase()
}

/// Czy `path` leży w katalogu `root`. Porównanie nie zależy od wielkości liter i prefiksu `\\?\`.
pub fn path_within(root: &Path, path: &Path) -> bool {
    let root = comparable(root);
    let path = comparable(path);
    !root.is_empty() && (path == root || path.starts_with(&format!("{root}\\")))
}

/// Część ścieżki `target` po katalogu `root`, z ukośnikami `/`.
pub fn relative_inside(root: &Path, target: &Path) -> Option<String> {
    if let Ok(stripped) = target.strip_prefix(root) {
        return Some(stripped.to_string_lossy().replace('\\', "/"));
    }
    if !path_within(root, target) {
        return None;
    }
    let root_len = comparable(root).chars().count();
    let target_text = target.to_string_lossy().replace('/', "\\");
    let target_text = target_text.strip_prefix(r"\\?\").unwrap_or(&target_text).to_string();
    let rest: String = target_text.chars().skip(root_len).collect();
    Some(rest.trim_start_matches('\\').replace('\\', "/"))
}

pub fn read_limited(source: &mut impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = source.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        if bytes.len() + read > limit {
            return Err("Plik jest za duży.".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_sha256() {
        let dir = std::env::temp_dir().join(format!("bib-hash-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        fs::write(&path, b"biblioteka").unwrap();
        let real = hash_file(&path).unwrap();
        assert_eq!(real.len(), 64);
        assert_eq!(real, hash_bytes(b"biblioteka"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn separate_directories_for_different_books() {
        let dir = std::env::temp_dir().join(format!("bib-dir-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let first = book_directory(&dir, "Autor: A", "Tytuł/1", "book-1").unwrap();
        fs::create_dir_all(&first).unwrap();
        write_atomic(&first.join("metadata.json"), br#"{"bookId":"book-1"}"#).unwrap();
        let second = book_directory(&dir, "Autor: A", "Tytuł/1", "book-2").unwrap();
        assert_ne!(first, second);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn safe_relative_rejects_escapes() {
        assert!(safe_relative("../../Windows/system.ini").is_none());
        assert!(safe_relative("C:\\Windows\\notepad.exe").is_none());
        assert!(safe_relative("/etc/passwd").is_none());
        assert!(safe_relative("a/../../b").is_none());
        assert!(safe_relative("").is_none());
        assert_eq!(safe_relative("Autor/Tytuł/książka.epub"), Some(PathBuf::from("Autor/Tytuł/książka.epub")));
        assert_eq!(safe_relative("Autor\\Tytuł\\a.pdf"), Some(PathBuf::from("Autor/Tytuł/a.pdf")));
    }

    #[test]
    fn relative_inside_ignores_verbatim_prefix_and_case() {
        let root = Path::new(r"C:\Biblioteka");
        assert_eq!(relative_inside(root, Path::new(r"C:\Biblioteka\A\b.epub")).as_deref(), Some("A/b.epub"));
        assert_eq!(relative_inside(Path::new(r"\\?\C:\Biblioteka"), Path::new(r"c:\biblioteka\A\b.epub")).as_deref(), Some("A/b.epub"));
        assert_eq!(relative_inside(root, Path::new(r"C:\Inne\b.epub")), None);
        assert_eq!(relative_inside(root, Path::new(r"C:\BibliotekaX\b.epub")), None);
    }

    #[test]
    fn copy_tree_skips_up_to_date_and_stops_on_cancel() {
        let dir = std::env::temp_dir().join(format!("bib-tree-{}", uuid::Uuid::new_v4()));
        let source = dir.join("src");
        fs::create_dir_all(source.join("a")).unwrap();
        fs::write(source.join("a/1.epub"), b"jeden").unwrap();
        let target = dir.join("dst");
        copy_tree(&source, &target, &mut |_, _, _| {}, &NEVER).unwrap();
        assert_eq!(fs::read(target.join("a/1.epub")).unwrap(), b"jeden");
        assert!(up_to_date(&source.join("a/1.epub"), &target.join("a/1.epub")));
        let cancel = AtomicBool::new(true);
        assert_eq!(copy_tree(&source, &dir.join("other"), &mut |_, _, _| {}, &cancel).unwrap_err(), CANCELLED);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn read_limited_rejects_oversized_input() {
        let data = vec![1_u8; 20_000];
        assert!(read_limited(&mut data.as_slice(), 10_000).is_err());
        assert_eq!(read_limited(&mut data.as_slice(), 20_000).unwrap().len(), 20_000);
    }
}
