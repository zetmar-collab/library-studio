//! Import z innych katalogów: biblioteka Calibre (metadata.opf), eksport Goodreads (CSV) i zwykły CSV.

use std::fs;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::domain::{validate_isbn, BookDraft, ProgressDraft};
use crate::infrastructure::db::map_sql;
use crate::infrastructure::store::{self, StorePaths};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImportRow {
    pub title: String,
    pub authors: Vec<String>,
    pub isbn13: Option<String>,
    pub isbn10: Option<String>,
    pub publisher: Option<String>,
    pub year: Option<i64>,
    pub language: Option<String>,
    pub description: Option<String>,
    pub series_name: Option<String>,
    pub series_number: Option<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub page_count: Option<i64>,
    pub rating: Option<i64>,
    pub finished_at: Option<String>,
    pub to_read: bool,
    pub files: Vec<String>,
    pub cover_file: Option<String>,
    /// Id książki w katalogu, z którą ta pozycja się pokrywa (tytuł i autor albo ISBN).
    pub duplicate_of: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub added: u32,
    pub skipped: u32,
    pub files: u32,
    pub errors: Vec<String>,
}

pub fn preview(conn: &Connection, kind: &str, path: &Path) -> Result<Vec<ImportRow>, String> {
    let mut rows = match kind {
        "calibre" => read_calibre(path)?,
        "goodreads" => read_goodreads(&read_text(path)?)?,
        "csv" => read_generic_csv(&read_text(path)?)?,
        _ => return Err("Nieznany rodzaj importu.".into()),
    };
    for row in &mut rows {
        row.duplicate_of = existing_book(conn, row)?;
    }
    Ok(rows)
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("Nie można odczytać pliku: {e}"))?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    Ok(text.trim_start_matches('\u{feff}').to_string())
}

fn existing_book(conn: &Connection, row: &ImportRow) -> Result<Option<String>, String> {
    for isbn in [row.isbn13.as_deref(), row.isbn10.as_deref()].into_iter().flatten() {
        if let Some(id) = store::find_book_by_isbn(conn, isbn)? {
            return Ok(Some(id));
        }
    }
    let authors: Vec<String> = row.authors.iter().filter(|a| !a.trim().is_empty()).cloned().collect();
    if row.title.trim().is_empty() || authors.is_empty() {
        return Ok(None);
    }
    store::find_book_by_key(conn, &crate::domain::dedupe_key(row.title.trim(), &authors))
}

pub fn commit(conn: &Connection, paths: &StorePaths, rows: &[ImportRow]) -> Result<ImportSummary, String> {
    let mut summary = ImportSummary::default();
    let to_read_shelf: Option<String> = conn
        .query_row("SELECT id FROM shelves WHERE name='Do przeczytania' AND is_smart_shelf=0 AND deleted_at IS NULL LIMIT 1", [], |row| row.get(0))
        .optional()
        .map_err(map_sql)?;
    for row in rows {
        let authors: Vec<String> = row.authors.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
        let draft = BookDraft {
            id: None,
            title: row.title.trim().to_string(),
            original_title: None,
            subtitle: None,
            authors: if authors.is_empty() { vec!["Nieznany autor".into()] } else { authors },
            translators: vec![],
            publisher: row.publisher.clone(),
            publication_year: row.year,
            language: row.language.clone(),
            description: row.description.clone(),
            series_name: row.series_name.clone(),
            series_number: row.series_number.clone(),
            genres: row.genres.clone(),
            tags: row.tags.clone(),
            isbn10: row.isbn10.clone(),
            isbn13: row.isbn13.clone(),
            page_count: row.page_count,
            cover_url: None,
            metadata_source: Some("import".into()),
            external_id: None,
            edition_id: None,
            average_rating: None,
            rating_source: None,
        };
        let saved = match store::save_book(conn, draft, paths, None) {
            Ok(saved) => saved,
            Err(error) => {
                summary.errors.push(format!("{}: {error}", row.title));
                continue;
            }
        };
        if !saved.created {
            summary.skipped += 1;
            continue;
        }
        summary.added += 1;
        if row.rating.is_some() || row.finished_at.is_some() {
            let progress = ProgressDraft {
                id: None,
                book_id: saved.book_id.clone(),
                format: None,
                progress_percent: row.finished_at.as_ref().map(|_| 100.0),
                current_page: None,
                current_chapter: None,
                started_at: None,
                finished_at: row.finished_at.clone(),
                rating: row.rating.filter(|r| (1..=5).contains(r)),
                review: None,
                notes: None,
            };
            if let Err(error) = store::save_progress(conn, progress) {
                summary.errors.push(format!("{}: {error}", row.title));
            }
        }
        if row.to_read {
            if let Some(shelf) = &to_read_shelf {
                let _ = store::add_to_shelf(conn, shelf, &saved.book_id);
            }
        }
        if paths.library.is_some() {
            for file in &row.files {
                match store::register_digital(conn, &saved.book_id, Path::new(file), paths) {
                    Ok(_) => summary.files += 1,
                    Err(error) => summary.errors.push(format!("{}: {error}", row.title)),
                }
            }
        }
        if let Some(cover) = &row.cover_file {
            if let Ok(bytes) = fs::read(cover) {
                if bytes.len() <= 8_000_000 && crate::infrastructure::metadata::image_type(&bytes).is_some() {
                    let _ = store::attach_cover(conn, paths, &saved.book_id, &bytes);
                }
            }
        }
    }
    Ok(summary)
}

// ---------- Calibre ----------

fn read_calibre(folder: &Path) -> Result<Vec<ImportRow>, String> {
    if !folder.is_dir() {
        return Err("Nie ma takiego folderu.".into());
    }
    let mut rows = Vec::new();
    for entry in walkdir::WalkDir::new(folder).max_depth(4).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() || entry.file_name() != "metadata.opf" {
            continue;
        }
        let Ok(text) = fs::read_to_string(entry.path()) else { continue };
        let Some(dir) = entry.path().parent() else { continue };
        if let Some(row) = parse_opf(&text, dir) {
            rows.push(row);
        }
        if rows.len() >= 20_000 {
            break;
        }
    }
    if rows.is_empty() {
        return Err("W tym folderze nie ma plików metadata.opf. Wskaż folder biblioteki Calibre.".into());
    }
    Ok(rows)
}

pub fn parse_opf(text: &str, dir: &Path) -> Option<ImportRow> {
    let title = xml_values(text, "dc:title").into_iter().next()?;
    let mut row = ImportRow { title, ..Default::default() };
    row.authors = xml_values(text, "dc:creator");
    row.publisher = xml_values(text, "dc:publisher").into_iter().next();
    row.year = xml_values(text, "dc:date").into_iter().next().and_then(|d| d.get(0..4).and_then(|y| y.parse().ok())).filter(|y| *y > 101);
    row.language = xml_values(text, "dc:language").into_iter().next();
    row.description = xml_values(text, "dc:description").into_iter().next().map(|d| crate::domain::strip_html(&d));
    row.genres = xml_values(text, "dc:subject");
    for identifier in xml_elements(text, "dc:identifier") {
        let scheme = identifier.attrs.to_lowercase();
        let value = identifier.text.trim().trim_start_matches("urn:isbn:").to_string();
        if scheme.contains("isbn") || identifier.text.to_lowercase().starts_with("urn:isbn:") {
            assign_isbn(&mut row, &value);
        }
    }
    row.series_name = meta_content(text, "calibre:series");
    row.series_number = meta_content(text, "calibre:series_index").map(|n| n.trim_end_matches(".0").to_string());
    row.rating = meta_content(text, "calibre:rating").and_then(|r| r.parse::<f64>().ok()).map(|r| (r / 2.0).round() as i64).filter(|r| (1..=5).contains(r));
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if ["epub", "pdf", "mobi", "azw3", "azw", "fb2", "djvu", "cbz", "kepub"].contains(&ext.as_str()) {
                row.files.push(path.display().to_string());
            } else if name == "cover.jpg" || name == "cover.png" {
                row.cover_file = Some(path.display().to_string());
            }
        }
    }
    row.files.sort();
    Some(row)
}

struct XmlElement {
    attrs: String,
    text: String,
}

fn xml_elements(text: &str, tag: &str) -> Vec<XmlElement> {
    let mut out = Vec::new();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut rest = text;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        // Nazwa musi się kończyć tutaj (np. <dc:title>, a nie <dc:titles>).
        if !after.starts_with(['>', ' ', '\t', '\n', '\r', '/']) {
            rest = after;
            continue;
        }
        let Some(head_end) = after.find('>') else { break };
        let attrs = after[..head_end].to_string();
        if attrs.ends_with('/') {
            rest = &after[head_end + 1..];
            continue;
        }
        let body = &after[head_end + 1..];
        let Some(end) = body.find(&close) else { break };
        out.push(XmlElement { attrs, text: decode_entities(body[..end].trim()) });
        rest = &body[end + close.len()..];
    }
    out
}

fn xml_values(text: &str, tag: &str) -> Vec<String> {
    xml_elements(text, tag).into_iter().map(|e| e.text).filter(|v| !v.is_empty()).collect()
}

fn meta_content(text: &str, name: &str) -> Option<String> {
    let needle = format!("name=\"{name}\"");
    for chunk in text.split("<meta").skip(1) {
        let head = chunk.split('>').next().unwrap_or("");
        if !head.contains(&needle) {
            continue;
        }
        let start = head.find("content=\"")? + "content=\"".len();
        let end = head[start..].find('"')? + start;
        let value = decode_entities(&head[start..end]);
        if !value.trim().is_empty() {
            return Some(value);
        }
    }
    None
}

pub(crate) fn decode_entities(value: &str) -> String {
    let mut out = value.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'");
    for (name, ch) in [
        ("&nbsp;", " "), ("&oacute;", "ó"), ("&Oacute;", "Ó"), ("&ndash;", "–"), ("&mdash;", "—"), ("&hellip;", "…"),
        ("&bdquo;", "„"), ("&rdquo;", "”"), ("&ldquo;", "“"), ("&rsquo;", "’"), ("&lsquo;", "‘"), ("&laquo;", "«"), ("&raquo;", "»"),
    ] {
        out = out.replace(name, ch);
    }
    while let Some(start) = out.find("&#") {
        let Some(len) = out[start..].find(';') else { break };
        let code = &out[start + 2..start + len];
        let parsed = if let Some(hex) = code.strip_prefix(['x', 'X']) { u32::from_str_radix(hex, 16).ok() } else { code.parse().ok() };
        let Some(ch) = parsed.and_then(char::from_u32) else { break };
        out.replace_range(start..start + len + 1, &ch.to_string());
    }
    out.replace("&amp;", "&")
}

fn assign_isbn(row: &mut ImportRow, raw: &str) {
    let Ok(isbn) = validate_isbn(raw) else { return };
    if isbn.len() == 13 {
        row.isbn13.get_or_insert(isbn);
    } else {
        row.isbn10.get_or_insert(isbn);
    }
}

// ---------- CSV ----------

/// Parser CSV zgodny z RFC 4180: cudzysłowy, podwójne cudzysłowy, nowe linie w polach.
/// Separator wykrywany z pierwszej linii (przecinek, średnik albo tabulator).
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let first_line = text.lines().next().unwrap_or("");
    let separator = [',', ';', '\t'].into_iter().max_by_key(|sep| first_line.matches(*sep).count()).unwrap_or(',');
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
            }
        } else if ch == '"' && field.is_empty() {
            quoted = true;
        } else if ch == separator {
            row.push(std::mem::take(&mut field));
        } else if ch == '\n' || ch == '\r' {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            if row.iter().any(|value| !value.trim().is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
        } else {
            field.push(ch);
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        if row.iter().any(|value| !value.trim().is_empty()) {
            rows.push(row);
        }
    }
    rows
}

struct Table {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    fn new(text: &str) -> Result<Self, String> {
        let mut all = parse_csv(text);
        if all.is_empty() {
            return Err("Plik CSV jest pusty.".into());
        }
        let header = all.remove(0).into_iter().map(|h| crate::domain::fold(&h)).collect();
        Ok(Self { header, rows: all })
    }

    fn column(&self, names: &[&str]) -> Option<usize> {
        names.iter().find_map(|name| self.header.iter().position(|h| h == &crate::domain::fold(name)))
    }
}

fn cell(row: &[String], index: Option<usize>) -> Option<String> {
    index.and_then(|i| row.get(i)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn split_list(value: Option<String>) -> Vec<String> {
    value
        .map(|v| v.split([';', '|']).map(|part| part.trim().to_string()).filter(|part| !part.is_empty()).collect())
        .unwrap_or_default()
}

/// Goodreads zapisuje ISBN jako ="0140328726", żeby arkusz nie zgubił zer.
fn clean_isbn_cell(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim_start_matches('=').trim_matches('"').to_string()).filter(|v| !v.is_empty())
}

fn read_goodreads(text: &str) -> Result<Vec<ImportRow>, String> {
    let table = Table::new(text)?;
    let title = table.column(&["Title"]).ok_or_else(|| "To nie wygląda na eksport Goodreads: brak kolumny Title.".to_string())?;
    let author = table.column(&["Author"]);
    let additional = table.column(&["Additional Authors"]);
    let isbn = table.column(&["ISBN"]);
    let isbn13 = table.column(&["ISBN13"]);
    let rating = table.column(&["My Rating"]);
    let publisher = table.column(&["Publisher"]);
    let pages = table.column(&["Number of Pages"]);
    let year = table.column(&["Original Publication Year", "Year Published"]);
    let read_at = table.column(&["Date Read"]);
    let shelf = table.column(&["Exclusive Shelf"]);
    let shelves = table.column(&["Bookshelves"]);
    let mut out = Vec::new();
    for row in &table.rows {
        let Some(title_text) = cell(row, Some(title)) else { continue };
        let mut item = ImportRow { title: title_text, ..Default::default() };
        // Tytuł z serią: „Ostatnie życzenie (Wiedźmin, #1)”.
        if let Some(open) = item.title.rfind(" (") {
            if item.title.ends_with(')') && item.title[open..].contains('#') {
                let inner = item.title[open + 2..item.title.len() - 1].to_string();
                if let Some((series, number)) = inner.split_once(", #") {
                    item.series_name = Some(series.trim().to_string());
                    item.series_number = Some(number.trim().to_string());
                    item.title = item.title[..open].trim().to_string();
                }
            }
        }
        item.authors = cell(row, author).into_iter().chain(split_list(cell(row, additional)).into_iter().flat_map(|a| a.split(',').map(|x| x.trim().to_string()).collect::<Vec<_>>())).filter(|a| !a.is_empty()).collect();
        for raw in [clean_isbn_cell(cell(row, isbn13)), clean_isbn_cell(cell(row, isbn))].into_iter().flatten() {
            assign_isbn(&mut item, &raw);
        }
        item.rating = cell(row, rating).and_then(|r| r.parse().ok()).filter(|r| (1..=5).contains(r));
        item.publisher = cell(row, publisher);
        item.page_count = cell(row, pages).and_then(|p| p.parse().ok());
        item.year = cell(row, year).and_then(|y| y.parse().ok());
        item.finished_at = cell(row, read_at).map(|d| d.replace('/', "-"));
        let exclusive = cell(row, shelf).unwrap_or_default();
        item.to_read = exclusive == "to-read";
        item.tags = split_list(cell(row, shelves).map(|s| s.replace(',', ";")))
            .into_iter()
            .filter(|tag| !["read", "to-read", "currently-reading"].contains(&tag.as_str()))
            .collect();
        out.push(item);
    }
    Ok(out)
}

fn read_generic_csv(text: &str) -> Result<Vec<ImportRow>, String> {
    let table = Table::new(text)?;
    let title = table
        .column(&["title", "tytuł", "tytul"])
        .ok_or_else(|| "Plik CSV potrzebuje kolumny title albo tytuł.".to_string())?;
    let authors = table.column(&["authors", "author", "autorzy", "autor"]);
    let isbn = table.column(&["isbn", "isbn13", "isbn10"]);
    let isbn10 = table.column(&["isbn10"]);
    let publisher = table.column(&["publisher", "wydawca", "wydawnictwo"]);
    let year = table.column(&["year", "rok", "rok wydania"]);
    let language = table.column(&["language", "język", "jezyk"]);
    let series = table.column(&["series", "seria"]);
    let series_number = table.column(&["series_number", "series number", "tom", "numer w serii"]);
    let genres = table.column(&["genres", "gatunki", "gatunek"]);
    let tags = table.column(&["tags", "tagi"]);
    let rating = table.column(&["rating", "ocena"]);
    let finished = table.column(&["finished_at", "przeczytane", "data przeczytania"]);
    let pages = table.column(&["pages", "strony", "liczba stron"]);
    let mut out = Vec::new();
    for row in &table.rows {
        let Some(title_text) = cell(row, Some(title)) else { continue };
        let mut item = ImportRow { title: title_text, ..Default::default() };
        item.authors = split_list(cell(row, authors));
        for raw in [cell(row, isbn), cell(row, isbn10)].into_iter().flatten() {
            assign_isbn(&mut item, &raw);
        }
        item.publisher = cell(row, publisher);
        item.year = cell(row, year).and_then(|y| y.get(0..4).and_then(|v| v.parse().ok()));
        item.language = cell(row, language);
        item.series_name = cell(row, series);
        item.series_number = cell(row, series_number);
        item.genres = split_list(cell(row, genres));
        item.tags = split_list(cell(row, tags));
        item.rating = cell(row, rating).and_then(|r| r.replace(',', ".").parse::<f64>().ok()).map(|r| r.round() as i64).filter(|r| (1..=5).contains(r));
        item.finished_at = cell(row, finished);
        item.page_count = cell(row, pages).and_then(|p| p.parse().ok());
        out.push(item);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_parser_handles_quotes_newlines_and_semicolons() {
        let rows = parse_csv("a;b;c\r\n\"x; y\";\"z \"\"q\"\"\";\"line\nbreak\"\r\n\r\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1], vec!["x; y".to_string(), "z \"q\"".into(), "line\nbreak".into()]);
    }

    #[test]
    fn reads_goodreads_export() {
        let text = "Book Id,Title,Author,Additional Authors,ISBN,ISBN13,My Rating,Publisher,Number of Pages,Year Published,Original Publication Year,Date Read,Bookshelves,Exclusive Shelf\n\
                    1,\"Ostatnie życzenie (Wiedźmin, #1)\",Andrzej Sapkowski,,\"=\"\"8375780634\"\"\",\"=\"\"9788375780635\"\"\",5,SuperNOWA,332,2014,1993,2024/05/01,\"fantasy, read\",read\n\
                    2,Solaris,Stanisław Lem,,=\"\",=\"\",0,,,,,,to-read,to-read\n";
        let rows = read_goodreads(text).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "Ostatnie życzenie");
        assert_eq!(rows[0].series_name.as_deref(), Some("Wiedźmin"));
        assert_eq!(rows[0].series_number.as_deref(), Some("1"));
        assert_eq!(rows[0].isbn13.as_deref(), Some("9788375780635"));
        assert_eq!(rows[0].rating, Some(5));
        assert_eq!(rows[0].finished_at.as_deref(), Some("2024-05-01"));
        assert_eq!(rows[0].tags, vec!["fantasy".to_string()]);
        assert!(rows[1].to_read);
        assert_eq!(rows[1].rating, None);
    }

    #[test]
    fn reads_generic_polish_csv() {
        let rows = read_generic_csv("Tytuł;Autor;ISBN;Rok;Tagi\nLalka;Bolesław Prus;9780140328721;1890;klasyka|lektura\n").unwrap();
        assert_eq!(rows[0].title, "Lalka");
        assert_eq!(rows[0].authors, vec!["Bolesław Prus".to_string()]);
        assert_eq!(rows[0].year, Some(1890));
        assert_eq!(rows[0].tags.len(), 2);
        assert_eq!(rows[0].isbn13.as_deref(), Some("9780140328721"));
    }

    #[test]
    fn parses_calibre_opf() {
        let opf = r#"<?xml version='1.0' encoding='utf-8'?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="uuid_id" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier opf:scheme="calibre" id="calibre_id">12</dc:identifier>
    <dc:identifier opf:scheme="ISBN">978-0-14-032872-1</dc:identifier>
    <dc:title>Pan Tadeusz &amp; inne</dc:title>
    <dc:creator opf:file-as="Mickiewicz, Adam" opf:role="aut">Adam Mickiewicz</dc:creator>
    <dc:date>1834-06-28T00:00:00+00:00</dc:date>
    <dc:publisher>Ossolineum</dc:publisher>
    <dc:language>pol</dc:language>
    <dc:subject>Epopeja</dc:subject>
    <meta name="calibre:series" content="Lektury"/>
    <meta name="calibre:series_index" content="3.0"/>
    <meta name="calibre:rating" content="8"/>
  </metadata>
</package>"#;
        let row = parse_opf(opf, Path::new("C:/nie-istnieje")).unwrap();
        assert_eq!(row.title, "Pan Tadeusz & inne");
        assert_eq!(row.authors, vec!["Adam Mickiewicz".to_string()]);
        assert_eq!(row.isbn13.as_deref(), Some("9780140328721"));
        assert_eq!(row.year, Some(1834));
        assert_eq!(row.series_name.as_deref(), Some("Lektury"));
        assert_eq!(row.series_number.as_deref(), Some("3"));
        assert_eq!(row.rating, Some(4));
        assert_eq!(row.genres, vec!["Epopeja".to_string()]);
    }

    #[test]
    fn commit_adds_books_and_skips_existing() {
        let dir = std::env::temp_dir().join(format!("bib-imp-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = crate::infrastructure::db::Database::open(&dir.join("t.sqlite")).unwrap();
        let paths = StorePaths { library: None, covers: dir.clone() };
        let rows = vec![
            ImportRow { title: "Lalka".into(), authors: vec!["Prus".into()], rating: Some(4), finished_at: Some("2025-01-02".into()), ..Default::default() },
            ImportRow { title: "Solaris".into(), authors: vec!["Lem".into()], to_read: true, ..Default::default() },
        ];
        let first = db.with(|c| commit(c, &paths, &rows)).unwrap();
        assert_eq!(first.added, 2);
        let second = db.with(|c| commit(c, &paths, &rows)).unwrap();
        assert_eq!(second.skipped, 2);
        let previewed = db.with(|c| {
            let mut copy = rows[0].clone();
            copy.duplicate_of = existing_book(c, &copy)?;
            Ok(copy)
        }).unwrap();
        assert!(previewed.duplicate_of.is_some());
        let _ = fs::remove_dir_all(dir);
    }
}
