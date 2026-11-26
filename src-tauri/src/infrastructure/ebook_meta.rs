//! Dane książki zapisane przy samym pliku: indeks `metadata.calibre`, który Calibre trzyma na czytniku,
//! i metadane OPF wewnątrz pliku EPUB. Nazwa pliku jest dopiero ostatnią deską ratunku.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use serde_json::Value;

use crate::domain::fold;
use crate::infrastructure::importer::{assign_isbn, parse_opf, ImportRow};

const OPF_LIMIT: u64 = 2_000_000;
const CALIBRE_INDEX_LIMIT: u64 = 50_000_000;

/// Indeks `metadata.calibre` z katalogu głównego nośnika: ścieżka pliku (małe litery, ukośniki „/”) → dane.
pub fn calibre_index(root: &Path) -> HashMap<String, ImportRow> {
    let path = root.join("metadata.calibre");
    let mut out = HashMap::new();
    if path.metadata().map(|meta| meta.len() > CALIBRE_INDEX_LIMIT).unwrap_or(true) {
        return out;
    }
    let Ok(text) = fs::read_to_string(&path) else { return out };
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(&text) else { return out };
    for entry in entries {
        let Some(lpath) = entry.get("lpath").and_then(Value::as_str) else { continue };
        if let Some(row) = row_from_calibre(&entry) {
            out.insert(lpath_key(lpath), row);
        }
    }
    out
}

pub fn lpath_key(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_lowercase()
}

fn row_from_calibre(entry: &Value) -> Option<ImportRow> {
    let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let list = |key: &str| -> Vec<String> {
        entry.get(key).and_then(Value::as_array).map(|items| items.iter().filter_map(Value::as_str).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect()).unwrap_or_default()
    };
    let title = text("title")?;
    let mut row = ImportRow { title, ..Default::default() };
    row.authors = list("authors").into_iter().filter(|name| fold(name) != "unknown" && fold(name) != "nieznany").collect();
    row.publisher = text("publisher");
    // Calibre zapisuje brak daty jako rok 101.
    row.year = text("pubdate").and_then(|d| d.get(0..4).and_then(|y| y.parse().ok())).filter(|y| *y > 101);
    row.description = text("comments").map(|d| crate::domain::strip_html(&d));
    row.tags = list("tags");
    row.language = list("languages").into_iter().next();
    row.series_name = text("series");
    row.series_number = entry.get("series_index").and_then(Value::as_f64).filter(|_| row.series_name.is_some()).map(|n| {
        if n.fract() == 0.0 { format!("{}", n as i64) } else { n.to_string() }
    });
    if let Some(ids) = entry.get("identifiers").and_then(Value::as_object) {
        if let Some(isbn) = ids.get("isbn").and_then(Value::as_str) {
            assign_isbn(&mut row, isbn);
        }
    }
    Some(row)
}

/// Metadane OPF z wnętrza pliku EPUB (tytuł, autorzy, wydawca, ISBN, opis, seria Calibre).
pub fn from_epub(path: &Path) -> Option<ImportRow> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    if !name.ends_with(".epub") {
        return None;
    }
    let file = fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let container = read_entry(&mut archive, "META-INF/container.xml")?;
    let opf_path = container
        .split("full-path=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .map(str::to_string)
        .or_else(|| (0..archive.len()).find_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()).filter(|n| n.to_lowercase().ends_with(".opf"))))?;
    let opf = read_entry(&mut archive, &opf_path)?;
    // Katalog nie istnieje, więc parse_opf nie dołączy żadnych plików obok.
    let mut row = parse_opf(&opf, Path::new("\u{0}"))?;
    row.files.clear();
    row.cover_file = None;
    row.authors.retain(|name| !matches!(fold(name).as_str(), "unknown" | "nieznany" | "nieznany autor"));
    if row.title.trim().is_empty() || fold(&row.title) == "unknown" {
        return None;
    }
    Some(row)
}

fn read_entry(archive: &mut zip::ZipArchive<fs::File>, name: &str) -> Option<String> {
    let file = archive.by_name(name).ok()?;
    if file.size() > OPF_LIMIT {
        return None;
    }
    let mut text = String::new();
    file.take(OPF_LIMIT).read_to_string(&mut text).ok()?;
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_epub(path: &Path, opf: &str) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("mimetype", options).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        zip.start_file("META-INF/container.xml", options).unwrap();
        zip.write_all(br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
        zip.start_file("OEBPS/content.opf", options).unwrap();
        zip.write_all(opf.as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn reads_title_and_author_from_epub() {
        let dir = std::env::temp_dir().join(format!("bib-epub-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("AI dla Laika - Marek Zettel.epub");
        write_epub(&path, r#"<package><metadata><dc:title>AI dla Laika</dc:title><dc:creator opf:role="aut">Marek Zettel</dc:creator><dc:identifier opf:scheme="ISBN">9788328396661</dc:identifier><dc:language>pl</dc:language></metadata></package>"#);
        let row = from_epub(&path).unwrap();
        assert_eq!(row.title, "AI dla Laika");
        assert_eq!(row.authors, vec!["Marek Zettel".to_string()]);
        assert_eq!(row.isbn13.as_deref(), Some("9788328396661"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn reads_calibre_device_index() {
        let dir = std::env::temp_dir().join(format!("bib-cal-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("metadata.calibre"),
            r#"[{"lpath": "Books/AI dla Laika - Marek Zettel.epub", "title": "AI dla Laika", "authors": ["Marek Zettel"], "publisher": "Cyfrowy Przyjaciel", "pubdate": "0101-01-01T00:00:00+00:00", "series": "Dla laika", "series_index": 2.0, "tags": ["AI"], "identifiers": {"isbn": "978-83-283-9666-1"}}]"#,
        )
        .unwrap();
        let index = calibre_index(&dir);
        let row = index.get(&lpath_key("Books\\AI dla Laika - Marek Zettel.epub")).unwrap();
        assert_eq!(row.title, "AI dla Laika");
        assert_eq!(row.authors, vec!["Marek Zettel".to_string()]);
        assert_eq!(row.year, None);
        assert_eq!(row.series_number.as_deref(), Some("2"));
        assert_eq!(row.isbn13.as_deref(), Some("9788328396661"));
        let _ = fs::remove_dir_all(dir);
    }
}
