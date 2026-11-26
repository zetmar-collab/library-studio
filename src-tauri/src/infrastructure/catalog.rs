//! Funkcje katalogu ponad podstawowy zapis: kosz, duplikaty i scalanie, statystyki, filtry, eksport.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::domain::{dedupe_key, fold};
use crate::infrastructure::db::{map_sql, now};
use crate::infrastructure::files::path_within;
use crate::infrastructure::store::{
    self, find_book_by_isbn, find_book_by_key, load_book, load_digital, load_editions, put_setting, rebuild_search, resolve_path,
    setting, touch_book, write_metadata_file, StorePaths,
};

// ---------- Kosz ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashRow {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub deleted_at: String,
    pub files: i64,
}

pub fn list_trash(conn: &Connection) -> Result<Vec<TrashRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT b.id, b.title, b.authors, b.deleted_at,
                    (SELECT COUNT(*) FROM book_editions e JOIN digital_copies d ON d.book_edition_id = e.id WHERE e.book_id = b.id)
             FROM books b WHERE b.deleted_at IS NOT NULL AND b.purged_at IS NULL ORDER BY b.deleted_at DESC",
        )
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            let authors: String = row.get(2)?;
            Ok(TrashRow {
                id: row.get(0)?,
                title: row.get(1)?,
                authors: serde_json::from_str(&authors).unwrap_or_default(),
                deleted_at: row.get(3)?,
                files: row.get(4)?,
            })
        })
        .map_err(map_sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql)?;
    Ok(rows)
}

pub fn restore_book(conn: &Connection, id: &str) -> Result<(), String> {
    let book = load_book(conn, id)?.ok_or_else(|| "Nie ma takiej książki.".to_string())?;
    if book.deleted_at.is_none() {
        return Ok(());
    }
    if book.purged_at.is_some() {
        return Err("Ta książka została usunięta na stałe.".into());
    }
    let key = dedupe_key(&book.title, &book.authors);
    if find_book_by_key(conn, &key)?.is_some_and(|other| other != id) {
        return Err(format!("Nie można przywrócić: w katalogu jest już „{}”.", book.title));
    }
    for edition in load_editions(conn, id)? {
        for isbn in [edition.isbn13.as_deref(), edition.isbn10.as_deref()].into_iter().flatten() {
            if find_book_by_isbn(conn, isbn)?.is_some_and(|other| other != id) {
                return Err(format!("Nie można przywrócić: ISBN {isbn} ma już inna książka."));
            }
        }
    }
    for file in load_digital(conn, id)? {
        if let Some(title) = store::title_for_hash(conn, &file.file_hash_sha256)? {
            return Err(format!("Nie można przywrócić: plik {} jest już przy książce „{title}”.", file.file_name));
        }
    }
    conn.execute(
        "UPDATE books SET deleted_at=NULL, dedupe_key=?1, updated_at=?2 WHERE id=?3",
        params![key, now(), id],
    )
    .map_err(map_sql)?;
    rebuild_search(conn, id)
}

/// Usuwa książkę z kosza na stałe: wpisy w bazie i jej pliki w katalogu biblioteki.
/// Sam wiersz książki zostaje jako pusty nagrobek, żeby inne komputery też ją usunęły.
pub fn purge_book(conn: &Connection, paths: &StorePaths, id: &str) -> Result<u32, String> {
    let book = load_book(conn, id)?.ok_or_else(|| "Nie ma takiej książki.".to_string())?;
    if book.deleted_at.is_none() {
        return Err("Najpierw przenieś książkę do kosza.".into());
    }
    let mut doomed: Vec<PathBuf> = load_digital(conn, id)?
        .iter()
        .filter_map(|file| resolve_path(paths, &file.file_path))
        .collect();
    if let Some(cover) = book.cover_path.as_deref().filter(|v| !v.is_empty()).and_then(|v| resolve_path(paths, v)) {
        doomed.push(cover);
    }
    let tx = conn.unchecked_transaction().map_err(map_sql)?;
    tx.execute("DELETE FROM physical_copies WHERE book_edition_id IN (SELECT id FROM book_editions WHERE book_id=?1)", params![id]).map_err(map_sql)?;
    tx.execute("DELETE FROM digital_copies WHERE book_edition_id IN (SELECT id FROM book_editions WHERE book_id=?1)", params![id]).map_err(map_sql)?;
    tx.execute("DELETE FROM shelf_items WHERE book_id=?1", params![id]).map_err(map_sql)?;
    tx.execute("DELETE FROM reading_progress WHERE book_id=?1", params![id]).map_err(map_sql)?;
    tx.execute("DELETE FROM book_editions WHERE book_id=?1", params![id]).map_err(map_sql)?;
    tx.execute("DELETE FROM books_fts WHERE book_id=?1", params![id]).map_err(map_sql)?;
    let stamp = now();
    tx.execute(
        "UPDATE books SET purged_at=?1, updated_at=?1, description=NULL, cover_path=NULL, search_text='', dedupe_key=NULL WHERE id=?2",
        params![stamp, id],
    )
    .map_err(map_sql)?;
    tx.commit().map_err(map_sql)?;
    let mut removed = 0;
    let mut dirs = HashSet::new();
    for path in doomed {
        // Kasujemy tylko pliki leżące w katalogu biblioteki albo okładek.
        let inside = paths.library.as_ref().is_some_and(|root| path_within(root, &path)) || path_within(&paths.covers, &path);
        if inside && path.is_file() && fs::remove_file(&path).is_ok() {
            removed += 1;
            if let Some(parent) = path.parent() {
                dirs.insert(parent.to_path_buf());
            }
        }
    }
    for dir in dirs {
        remove_if_only_marker(&dir);
    }
    Ok(removed)
}

fn remove_if_only_marker(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let names: Vec<String> = entries.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    if names.iter().all(|name| name == "metadata.json") {
        let _ = fs::remove_file(dir.join("metadata.json"));
        let _ = fs::remove_dir(dir);
        if let Some(parent) = dir.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
}

pub fn empty_trash(conn: &Connection, paths: &StorePaths) -> Result<u32, String> {
    let ids: Vec<String> = list_trash(conn)?.into_iter().map(|row| row.id).collect();
    let mut count = 0;
    for id in ids {
        purge_book(conn, paths, &id)?;
        count += 1;
    }
    Ok(count)
}

// ---------- Duplikaty ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateBook {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub publication_year: Option<i64>,
    pub isbn: Option<String>,
    pub files: i64,
    pub copies: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub reason: String,
    pub books: Vec<DuplicateBook>,
}

fn ignored_pairs(conn: &Connection) -> Result<HashSet<String>, String> {
    Ok(setting(conn, "duplicates_ignored")?
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
        .into_iter()
        .collect())
}

fn pair_key(a: &str, b: &str) -> String {
    if a < b { format!("{a}|{b}") } else { format!("{b}|{a}") }
}

fn surname_words(authors: &[String]) -> HashSet<String> {
    fold(&authors.join(" ")).split_whitespace().filter(|word| word.chars().count() > 2).map(str::to_string).collect()
}

/// Szuka par książek, które wyglądają na tę samą pozycję: ten sam ISBN albo ten sam tytuł
/// i wspólne nazwisko autora (np. „Mickiewicz, Adam” i „Adam Mickiewicz”).
pub fn find_duplicates(conn: &Connection) -> Result<Vec<DuplicateGroup>, String> {
    let ignored = ignored_pairs(conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT b.id, b.title, b.authors, b.publication_year,
                    (SELECT COALESCE(e.isbn13, e.isbn10) FROM book_editions e WHERE e.book_id = b.id AND COALESCE(e.isbn13, e.isbn10) IS NOT NULL LIMIT 1),
                    (SELECT COUNT(*) FROM book_editions e JOIN digital_copies d ON d.book_edition_id = e.id WHERE e.book_id = b.id),
                    (SELECT COUNT(*) FROM book_editions e JOIN physical_copies p ON p.book_edition_id = e.id WHERE e.book_id = b.id)
             FROM books b WHERE b.deleted_at IS NULL",
        )
        .map_err(map_sql)?;
    let books: Vec<DuplicateBook> = stmt
        .query_map([], |row| {
            let authors: String = row.get(2)?;
            Ok(DuplicateBook {
                id: row.get(0)?,
                title: row.get(1)?,
                authors: serde_json::from_str(&authors).unwrap_or_default(),
                publication_year: row.get(3)?,
                isbn: row.get(4)?,
                files: row.get(5)?,
                copies: row.get(6)?,
            })
        })
        .map_err(map_sql)?
        .collect::<Result<_, _>>()
        .map_err(map_sql)?;
    drop(stmt);
    let mut groups = Vec::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut by_isbn: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, book) in books.iter().enumerate() {
        if let Some(isbn) = &book.isbn {
            by_isbn.entry(isbn.clone()).or_default().push(index);
        }
    }
    for (_, indexes) in by_isbn.into_iter().filter(|(_, list)| list.len() > 1) {
        let members: Vec<DuplicateBook> = indexes.iter().map(|i| books[*i].clone()).collect();
        if all_pairs_ignored(&members, &ignored) {
            continue;
        }
        used.extend(members.iter().map(|b| b.id.clone()));
        groups.push(DuplicateGroup { reason: "isbn".into(), books: members });
    }
    let mut by_title: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, book) in books.iter().enumerate() {
        let key = fold(&book.title);
        if !key.is_empty() {
            by_title.entry(key).or_default().push(index);
        }
    }
    for (_, indexes) in by_title.into_iter().filter(|(_, list)| list.len() > 1) {
        let mut members: Vec<DuplicateBook> = Vec::new();
        for (position, first) in indexes.iter().enumerate() {
            for second in indexes.iter().skip(position + 1) {
                let (a, b) = (&books[*first], &books[*second]);
                if ignored.contains(&pair_key(&a.id, &b.id)) {
                    continue;
                }
                let shared = surname_words(&a.authors).intersection(&surname_words(&b.authors)).next().is_some();
                let unknown = [a, b].iter().any(|book| book.authors.iter().any(|name| fold(name) == "nieznany autor"));
                if shared || unknown {
                    for book in [a, b] {
                        if !members.iter().any(|m| m.id == book.id) {
                            members.push(book.clone());
                        }
                    }
                }
            }
        }
        if members.len() > 1 && !members.iter().all(|m| used.contains(&m.id)) {
            groups.push(DuplicateGroup { reason: "title".into(), books: members });
        }
    }
    Ok(groups)
}

fn all_pairs_ignored(members: &[DuplicateBook], ignored: &HashSet<String>) -> bool {
    members.iter().enumerate().all(|(i, a)| members.iter().skip(i + 1).all(|b| ignored.contains(&pair_key(&a.id, &b.id))))
}

pub fn ignore_duplicates(conn: &Connection, ids: &[String]) -> Result<(), String> {
    let mut ignored = ignored_pairs(conn)?;
    for (i, a) in ids.iter().enumerate() {
        for b in ids.iter().skip(i + 1) {
            ignored.insert(pair_key(a, b));
        }
    }
    let mut list: Vec<String> = ignored.into_iter().collect();
    list.sort();
    put_setting(conn, "duplicates_ignored", &serde_json::to_string(&list).map_err(|e| e.to_string())?)
}

/// Scala książkę `merge_id` z `keep_id`: wydania, egzemplarze, pliki, postęp i półki przechodzą
/// do zostającej książki, puste pola są uzupełniane, a scalona trafia do kosza.
pub fn merge_books(conn: &Connection, paths: &StorePaths, keep_id: &str, merge_id: &str) -> Result<(), String> {
    if keep_id == merge_id {
        return Err("Wybierz dwie różne książki.".into());
    }
    let keep = load_book(conn, keep_id)?.filter(|b| b.deleted_at.is_none()).ok_or_else(|| "Nie ma takiej książki.".to_string())?;
    let other = load_book(conn, merge_id)?.filter(|b| b.deleted_at.is_none()).ok_or_else(|| "Nie ma takiej książki.".to_string())?;
    let tx = conn.unchecked_transaction().map_err(map_sql)?;
    tx.execute("UPDATE book_editions SET book_id=?1 WHERE book_id=?2", params![keep_id, merge_id]).map_err(map_sql)?;
    tx.execute("UPDATE reading_progress SET book_id=?1 WHERE book_id=?2", params![keep_id, merge_id]).map_err(map_sql)?;
    tx.execute(
        "DELETE FROM shelf_items WHERE book_id=?2 AND shelf_id IN (SELECT shelf_id FROM shelf_items WHERE book_id=?1)",
        params![keep_id, merge_id],
    )
    .map_err(map_sql)?;
    tx.execute("UPDATE shelf_items SET book_id=?1 WHERE book_id=?2", params![keep_id, merge_id]).map_err(map_sql)?;
    let union = |a: &[String], b: &[String]| {
        let mut out = a.to_vec();
        for item in b {
            if !out.iter().any(|existing| fold(existing) == fold(item)) {
                out.push(item.clone());
            }
        }
        serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
    };
    let pick = |a: &Option<String>, b: &Option<String>| a.clone().filter(|v| !v.trim().is_empty()).or_else(|| b.clone());
    tx.execute(
        "UPDATE books SET original_title=?1, subtitle=?2, publisher=?3, publication_year=?4, language=?5, description=?6,
                cover_path=?7, series_name=?8, series_number=?9, genres=?10, tags=?11, translators=?12,
                average_rating=COALESCE(average_rating, ?13), updated_at=?14 WHERE id=?15",
        params![
            pick(&keep.original_title, &other.original_title),
            pick(&keep.subtitle, &other.subtitle),
            pick(&keep.publisher, &other.publisher),
            keep.publication_year.or(other.publication_year),
            pick(&keep.language, &other.language),
            pick(&keep.description, &other.description),
            pick(&keep.cover_path, &other.cover_path),
            pick(&keep.series_name, &other.series_name),
            pick(&keep.series_number, &other.series_number),
            union(&keep.genres, &other.genres),
            union(&keep.tags, &other.tags),
            union(&keep.translators, &other.translators),
            other.average_rating,
            now(),
            keep_id
        ],
    )
    .map_err(map_sql)?;
    tx.execute(
        "UPDATE books SET deleted_at=?1, updated_at=?1, dedupe_key=NULL, cover_path=CASE WHEN cover_path=?2 THEN NULL ELSE cover_path END WHERE id=?3",
        params![now(), keep.cover_path.clone().filter(|v| !v.is_empty()).or(other.cover_path.clone()).unwrap_or_default(), merge_id],
    )
    .map_err(map_sql)?;
    touch_book(&tx, keep_id)?;
    rebuild_search(&tx, keep_id)?;
    rebuild_search(&tx, merge_id)?;
    tx.commit().map_err(map_sql)?;
    write_metadata_file(conn, paths, keep_id)?;
    Ok(())
}

// ---------- Statystyki ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Count {
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub year: i32,
    pub goal: Option<i64>,
    pub finished: i64,
    pub pages: i64,
    pub by_month: Vec<i64>,
    pub total_books: i64,
    pub paper: i64,
    pub ebooks: i64,
    pub loaned: i64,
    pub overdue: i64,
    pub unread: i64,
    pub top_genres: Vec<Count>,
    pub top_authors: Vec<Count>,
    pub by_year: Vec<Count>,
}

fn count(conn: &Connection, sql: &str) -> Result<i64, String> {
    conn.query_row(sql, [], |row| row.get(0)).map_err(map_sql)
}

pub fn stats(conn: &Connection, year: i32) -> Result<Stats, String> {
    let prefix = format!("{year:04}");
    // Książka liczy się raz w roku, nawet przy kilku wpisach postępu.
    let finished_rows = {
        let mut stmt = conn
            .prepare(
                "SELECT r.book_id, MAX(r.finished_at) FROM reading_progress r JOIN books b ON b.id = r.book_id
                 WHERE b.deleted_at IS NULL AND r.finished_at LIKE ?1 || '%' GROUP BY r.book_id",
            )
            .map_err(map_sql)?;
        let rows = stmt
            .query_map(params![prefix], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(map_sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_sql)?;
        rows
    };
    let mut by_month = vec![0_i64; 12];
    let mut pages = 0;
    for (book_id, finished_at) in &finished_rows {
        if let Some(month) = finished_at.get(5..7).and_then(|m| m.parse::<usize>().ok()).filter(|m| (1..=12).contains(m)) {
            by_month[month - 1] += 1;
        }
        let book_pages: Option<i64> = conn
            .query_row("SELECT MAX(page_count) FROM book_editions WHERE book_id=?1", params![book_id], |row| row.get(0))
            .optional()
            .map_err(map_sql)?
            .flatten();
        pages += book_pages.unwrap_or(0);
    }
    let top = |sql: &str| -> Result<Vec<Count>, String> {
        let mut stmt = conn.prepare(sql).map_err(map_sql)?;
        let rows = stmt
            .query_map([], |row| Ok(Count { name: row.get(0)?, count: row.get(1)? }))
            .map_err(map_sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_sql)?;
        Ok(rows)
    };
    let live_paper = "EXISTS(SELECT 1 FROM book_editions e JOIN physical_copies p ON p.book_edition_id = e.id WHERE e.book_id = b.id";
    Ok(Stats {
        year,
        goal: setting(conn, "reading_goal")?.and_then(|v| v.parse().ok()).filter(|v: &i64| *v > 0),
        finished: finished_rows.len() as i64,
        pages,
        by_month,
        total_books: count(conn, "SELECT COUNT(*) FROM books WHERE deleted_at IS NULL")?,
        paper: count(conn, &format!("SELECT COUNT(*) FROM books b WHERE b.deleted_at IS NULL AND {live_paper})"))?,
        ebooks: count(conn, "SELECT COUNT(*) FROM books b WHERE b.deleted_at IS NULL AND EXISTS(SELECT 1 FROM book_editions e JOIN digital_copies d ON d.book_edition_id = e.id WHERE e.book_id = b.id)")?,
        loaned: count(conn, &format!("SELECT COUNT(*) FROM books b WHERE b.deleted_at IS NULL AND {live_paper} AND p.loaned_to IS NOT NULL AND p.loaned_to != '')"))?,
        overdue: count(conn, &format!("SELECT COUNT(*) FROM books b WHERE b.deleted_at IS NULL AND {live_paper} AND p.loaned_to IS NOT NULL AND p.loaned_to != '' AND p.return_due_date IS NOT NULL AND p.return_due_date != '' AND p.return_due_date < date('now', 'localtime'))"))?,
        unread: count(conn, "SELECT COUNT(*) FROM books b WHERE b.deleted_at IS NULL AND NOT EXISTS(SELECT 1 FROM reading_progress r WHERE r.book_id = b.id AND r.finished_at IS NOT NULL)")?,
        top_genres: top("SELECT j.value, COUNT(*) FROM books b, json_each(b.genres) j WHERE b.deleted_at IS NULL AND TRIM(j.value) != '' GROUP BY j.value ORDER BY COUNT(*) DESC, j.value LIMIT 8")?,
        top_authors: top("SELECT j.value, COUNT(*) FROM books b, json_each(b.authors) j WHERE b.deleted_at IS NULL AND TRIM(j.value) != '' GROUP BY j.value ORDER BY COUNT(*) DESC, j.value LIMIT 8")?,
        by_year: top("SELECT substr(r.finished_at, 1, 4), COUNT(DISTINCT r.book_id) FROM reading_progress r JOIN books b ON b.id = r.book_id WHERE b.deleted_at IS NULL AND r.finished_at IS NOT NULL AND length(r.finished_at) >= 4 GROUP BY substr(r.finished_at, 1, 4) ORDER BY 1 DESC LIMIT 10")?,
    })
}

// ---------- Filtry (fasety) ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facets {
    pub authors: Vec<Count>,
    pub series: Vec<Count>,
    pub tags: Vec<Count>,
    pub years: Vec<Count>,
}

pub fn facets(conn: &Connection) -> Result<Facets, String> {
    let top = |sql: &str| -> Result<Vec<Count>, String> {
        let mut stmt = conn.prepare(sql).map_err(map_sql)?;
        let rows = stmt
            .query_map([], |row| Ok(Count { name: row.get(0)?, count: row.get(1)? }))
            .map_err(map_sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_sql)?;
        Ok(rows)
    };
    Ok(Facets {
        authors: top("SELECT j.value, COUNT(*) FROM books b, json_each(b.authors) j WHERE b.deleted_at IS NULL AND TRIM(j.value) != '' GROUP BY j.value ORDER BY j.value COLLATE NOCASE LIMIT 500")?,
        series: top("SELECT series_name, COUNT(*) FROM books WHERE deleted_at IS NULL AND series_name IS NOT NULL AND series_name != '' GROUP BY series_name ORDER BY series_name COLLATE NOCASE LIMIT 500")?,
        tags: top("SELECT value, SUM(n) FROM (SELECT j.value AS value, COUNT(*) AS n FROM books b, json_each(b.tags) j WHERE b.deleted_at IS NULL GROUP BY j.value UNION ALL SELECT j.value, COUNT(*) FROM books b, json_each(b.genres) j WHERE b.deleted_at IS NULL GROUP BY j.value) WHERE TRIM(value) != '' GROUP BY value ORDER BY value COLLATE NOCASE LIMIT 500")?,
        years: top("SELECT CAST(publication_year AS TEXT), COUNT(*) FROM books WHERE deleted_at IS NULL AND publication_year IS NOT NULL GROUP BY publication_year ORDER BY publication_year DESC LIMIT 300")?,
    })
}

// ---------- Eksport CSV i dane do druku ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRow {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub isbn13: Option<String>,
    pub isbn10: Option<String>,
    pub publisher: Option<String>,
    pub year: Option<i64>,
    pub language: Option<String>,
    pub series: Option<String>,
    pub series_number: Option<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub shelves: Vec<String>,
    pub paper: bool,
    pub ebook: bool,
    pub location: Option<String>,
    pub barcode: Option<String>,
    pub loaned_to: Option<String>,
    pub return_due: Option<String>,
    pub rating: Option<f64>,
    pub finished_at: Option<String>,
}

pub fn export_rows(conn: &Connection) -> Result<Vec<ExportRow>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM books WHERE deleted_at IS NULL ORDER BY title COLLATE NOCASE")
        .map_err(map_sql)?;
    let ids: Vec<String> = stmt.query_map([], |row| row.get(0)).map_err(map_sql)?.collect::<Result<_, _>>().map_err(map_sql)?;
    drop(stmt);
    let mut rows = Vec::new();
    for id in ids {
        let details = store::get_book(conn, &id)?;
        let edition = details.editions.first();
        let copy = details.physical.first();
        rows.push(ExportRow {
            id: id.clone(),
            title: details.book.title.clone(),
            authors: details.book.authors.clone(),
            isbn13: edition.and_then(|e| e.isbn13.clone()),
            isbn10: edition.and_then(|e| e.isbn10.clone()),
            publisher: details.book.publisher.clone(),
            year: details.book.publication_year,
            language: details.book.language.clone(),
            series: details.book.series_name.clone(),
            series_number: details.book.series_number.clone(),
            genres: details.book.genres.clone(),
            tags: details.book.tags.clone(),
            shelves: details.shelves.iter().map(|s| s.name.clone()).collect(),
            paper: !details.physical.is_empty(),
            ebook: !details.digital.is_empty(),
            location: copy.and_then(|c| c.location_note.clone()),
            barcode: copy.and_then(|c| c.barcode_value.clone()),
            loaned_to: copy.and_then(|c| c.loaned_to.clone()).filter(|v| !v.is_empty()),
            return_due: copy.and_then(|c| c.return_due_date.clone()).filter(|v| !v.is_empty()),
            rating: details.book.average_rating,
            finished_at: details.progress.iter().filter_map(|p| p.finished_at.clone()).max(),
        });
    }
    Ok(rows)
}

fn csv_field(value: &str) -> String {
    // Formuły arkusza na początku pola (=, +, -, @) są neutralizowane apostrofem.
    let value = if value.starts_with(['=', '+', '-', '@']) { format!("'{value}") } else { value.to_string() };
    if value.contains([',', '"', '\n', '\r', ';']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

pub fn export_csv(conn: &Connection, folder: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let rows = export_rows(conn)?;
    let mut out = String::from("\u{feff}title,authors,isbn13,isbn10,publisher,year,language,series,series_number,genres,tags,shelves,paper,ebook,location,loaned_to,return_due,rating,finished_at\r\n");
    for row in rows {
        let fields = [
            row.title,
            row.authors.join("; "),
            row.isbn13.unwrap_or_default(),
            row.isbn10.unwrap_or_default(),
            row.publisher.unwrap_or_default(),
            row.year.map(|y| y.to_string()).unwrap_or_default(),
            row.language.unwrap_or_default(),
            row.series.unwrap_or_default(),
            row.series_number.unwrap_or_default(),
            row.genres.join("; "),
            row.tags.join("; "),
            row.shelves.join("; "),
            if row.paper { "1".into() } else { "0".into() },
            if row.ebook { "1".into() } else { "0".into() },
            row.location.unwrap_or_default(),
            row.loaned_to.unwrap_or_default(),
            row.return_due.unwrap_or_default(),
            row.rating.map(|r| format!("{r:.1}")).unwrap_or_default(),
            row.finished_at.unwrap_or_default(),
        ];
        out.push_str(&fields.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d");
    let target = folder.join(format!("library-studio-{stamp}.csv"));
    crate::infrastructure::files::write_atomic(&target, out.as_bytes())?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::BookDraft;
    use crate::infrastructure::db::Database;
    use crate::infrastructure::store::save_book;

    fn draft(title: &str, author: &str, isbn: Option<&str>) -> BookDraft {
        BookDraft {
            id: None, title: title.into(), original_title: None, subtitle: None, authors: vec![author.into()],
            translators: vec![], publisher: None, publication_year: None, language: None, description: None,
            series_name: None, series_number: None, genres: vec!["Powieść".into()], tags: vec![], isbn10: None, isbn13: isbn.map(str::to_string),
            page_count: Some(300), cover_url: None, metadata_source: Some("manual".into()), external_id: None, edition_id: None,
            average_rating: None, rating_source: None,
        }
    }

    fn rig() -> (Database, StorePaths, PathBuf) {
        let dir = std::env::temp_dir().join(format!("bib-cat-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&dir.join("t.sqlite")).unwrap();
        let paths = StorePaths { library: Some(dir.join("lib")), covers: dir.join("covers") };
        (db, paths, dir)
    }

    #[test]
    fn deleted_book_can_be_added_again_with_the_same_isbn_and_restored() {
        let (db, paths, dir) = rig();
        let first = db.with(|c| save_book(c, draft("Lalka", "Prus", Some("9780140328721")), &paths, None)).unwrap();
        db.with(|c| store::delete_book(c, &first.book_id)).unwrap();
        let second = db.with(|c| save_book(c, draft("Lalka", "Prus", Some("9780140328721")), &paths, None)).unwrap();
        assert!(second.created, "po usunięciu ten sam ISBN musi dać się dodać ponownie");
        let refused = db.with(|c| restore_book(c, &first.book_id));
        assert!(refused.is_err(), "przywrócenie duplikatu powinno zostać zatrzymane");
        db.with(|c| store::delete_book(c, &second.book_id)).unwrap();
        db.with(|c| restore_book(c, &first.book_id)).unwrap();
        assert_eq!(db.with(store::book_count).unwrap(), 1);
        assert_eq!(db.with(list_trash).unwrap().len(), 1);
        db.with(|c| purge_book(c, &paths, &second.book_id)).unwrap();
        assert!(db.with(list_trash).unwrap().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn finds_and_merges_duplicates() {
        let (db, paths, dir) = rig();
        let a = db.with(|c| save_book(c, draft("Pan Tadeusz", "Mickiewicz, Adam", None), &paths, None)).unwrap().book_id;
        let b = db.with(|c| save_book(c, draft("Pan Tadeusz", "Adam Mickiewicz", Some("9780140328721")), &paths, None)).unwrap().book_id;
        let groups = db.with(find_duplicates).unwrap();
        assert_eq!(groups.len(), 1);
        db.with(|c| merge_books(c, &paths, &a, &b)).unwrap();
        let merged = db.with(|c| store::get_book(c, &a)).unwrap();
        assert!(merged.editions.iter().any(|e| e.isbn13.as_deref() == Some("9780140328721")));
        assert_eq!(db.with(store::book_count).unwrap(), 1);
        assert!(db.with(find_duplicates).unwrap().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ignored_duplicates_stay_hidden() {
        let (db, paths, dir) = rig();
        let a = db.with(|c| save_book(c, draft("Dziady", "Mickiewicz", None), &paths, None)).unwrap().book_id;
        let b = db.with(|c| save_book(c, draft("Dziady", "Adam Mickiewicz", None), &paths, None)).unwrap().book_id;
        db.with(|c| ignore_duplicates(c, &[a.clone(), b.clone()])).unwrap();
        assert!(db.with(find_duplicates).unwrap().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn stats_count_books_finished_in_the_year() {
        let (db, paths, dir) = rig();
        let id = db.with(|c| save_book(c, draft("Solaris", "Lem", None), &paths, None)).unwrap().book_id;
        db.with(|c| {
            store::save_progress(c, crate::domain::ProgressDraft {
                id: None, book_id: id.clone(), format: None, progress_percent: None, current_page: None, current_chapter: None,
                started_at: None, finished_at: Some("2026-03-14".into()), rating: Some(5), review: None, notes: None,
            })
        })
        .unwrap();
        let result = db.with(|c| stats(c, 2026)).unwrap();
        assert_eq!(result.finished, 1);
        assert_eq!(result.pages, 300);
        assert_eq!(result.by_month[2], 1);
        assert_eq!(db.with(|c| stats(c, 2025)).unwrap().finished, 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn csv_export_quotes_and_neutralizes_formulas() {
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("=SUMA(A1)"), "'=SUMA(A1)");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        let (db, paths, dir) = rig();
        db.with(|c| save_book(c, draft("Ferdydurke", "Gombrowicz", None), &paths, None)).unwrap();
        let file = db.with(|c| export_csv(c, &dir.join("out"))).unwrap();
        let text = fs::read_to_string(file).unwrap();
        assert!(text.contains("Ferdydurke,Gombrowicz"));
        let _ = fs::remove_dir_all(dir);
    }
}

// ---------- Naprawa danych z plików ----------

/// Książka z czytnika, której tytuł i autor w katalogu różnią się od metadanych zapisanych w pliku EPUB.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRepair {
    pub book_id: String,
    pub old_title: String,
    pub old_authors: Vec<String>,
    pub new_title: String,
    pub new_authors: Vec<String>,
}

fn repair_candidates(conn: &Connection) -> Result<Vec<String>, String> {
    // Tylko książki zaimportowane z czytnika (wcześniej bez źródła) i bez ręcznie poprawionego tytułu lub autora.
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT b.id FROM books b JOIN book_editions e ON e.book_id = b.id JOIN digital_copies d ON d.book_edition_id = e.id
             WHERE b.deleted_at IS NULL AND (e.metadata_source IS NULL OR e.metadata_source = 'reader')
               AND b.manually_edited_fields NOT LIKE '%\"title\"%' AND b.manually_edited_fields NOT LIKE '%\"authors\"%'",
        )
        .map_err(map_sql)?;
    let ids = stmt.query_map([], |row| row.get::<_, String>(0)).map_err(map_sql)?;
    ids.collect::<Result<Vec<_>, _>>().map_err(map_sql)
}

fn file_row(conn: &Connection, paths: &StorePaths, book_id: &str) -> Result<Option<crate::infrastructure::importer::ImportRow>, String> {
    for file in store::catalog_files(conn, book_id, paths)? {
        if let Some(row) = crate::infrastructure::ebook_meta::from_epub(&file.absolute) {
            if !row.authors.is_empty() {
                return Ok(Some(row));
            }
        }
    }
    Ok(None)
}

pub fn file_repairs(conn: &Connection, paths: &StorePaths) -> Result<Vec<FileRepair>, String> {
    let mut out = Vec::new();
    for id in repair_candidates(conn)? {
        let Some(book) = load_book(conn, &id)? else { continue };
        let Some(row) = file_row(conn, paths, &id)? else { continue };
        let new_title = row.title.trim().to_string();
        if fold(&new_title) == fold(&book.title) && dedupe_key(&new_title, &row.authors) == dedupe_key(&book.title, &book.authors) {
            continue;
        }
        out.push(FileRepair { book_id: id, old_title: book.title, old_authors: book.authors, new_title, new_authors: row.authors });
    }
    out.sort_by(|a, b| a.new_title.to_lowercase().cmp(&b.new_title.to_lowercase()));
    Ok(out)
}

/// Zastępuje tytuł i autora danymi z pliku i uzupełnia puste pola. Zwraca liczbę poprawionych książek.
pub fn apply_file_repairs(conn: &Connection, paths: &StorePaths, ids: &[String]) -> Result<u32, String> {
    let allowed: HashSet<String> = repair_candidates(conn)?.into_iter().collect();
    let mut fixed = 0;
    for id in ids.iter().filter(|id| allowed.contains(*id)) {
        let Some(row) = file_row(conn, paths, id)? else { continue };
        let title = row.title.trim().to_string();
        let key = dedupe_key(&title, &row.authors);
        // Ten sam tytuł i autor przy innej książce: to duplikat, zostawiamy go narzędziu „Duplikaty”.
        if find_book_by_key(conn, &key)?.is_some_and(|other| &other != id) {
            continue;
        }
        let stamp = now();
        conn.execute(
            "UPDATE books SET title=?1, authors=?2, dedupe_key=?3,
                publisher=COALESCE(NULLIF(publisher, ''), ?4), publication_year=COALESCE(publication_year, ?5),
                language=COALESCE(NULLIF(language, ''), ?6), description=COALESCE(NULLIF(description, ''), ?7),
                series_name=COALESCE(NULLIF(series_name, ''), ?8), series_number=COALESCE(NULLIF(series_number, ''), ?9),
                updated_at=?10 WHERE id=?11",
            params![
                title, serde_json::to_string(&row.authors).unwrap(), key, row.publisher, row.year, row.language,
                row.description, row.series_name, row.series_number, stamp, id
            ],
        )
        .map_err(map_sql)?;
        let edition = store::first_edition(conn, id)?;
        for isbn in [row.isbn13.as_deref(), row.isbn10.as_deref()].into_iter().flatten() {
            if find_book_by_isbn(conn, isbn)?.is_none() {
                let column = if isbn.len() == 13 { "isbn13" } else { "isbn10" };
                conn.execute(&format!("UPDATE book_editions SET {column}=COALESCE({column}, ?1), updated_at=?2 WHERE id=?3"), params![isbn, stamp, edition]).map_err(map_sql)?;
            }
        }
        conn.execute("UPDATE book_editions SET metadata_source='reader', updated_at=?1 WHERE id=?2", params![stamp, edition]).map_err(map_sql)?;
        rebuild_search(conn, id)?;
        let _ = write_metadata_file(conn, paths, id);
        fixed += 1;
    }
    Ok(fixed)
}

