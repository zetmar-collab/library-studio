use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use uuid::Uuid;

use crate::domain::{
    dedupe_key, fold, safe_component, validate_isbn, BackupRow, BookCard, BookDetails, BookDraft,
    BookRecord, ConflictRow, DigitalRecord, EditionRecord, FileRow, ListQuery, PhysicalDraft,
    PhysicalRecord, ProgressDraft, ProgressRecord, SaveResult, Settings, SettingsPatch, ShelfDraft,
    ImportReport, ShelfItemRecord, ShelfRecord, Snapshot,
};
use crate::infrastructure::db::{map_sql, now};
use crate::infrastructure::files::{path_within, relative_inside, safe_relative};

pub struct StorePaths {
    pub library: Option<PathBuf>,
    pub covers: PathBuf,
}

pub fn get_settings(conn: &Connection) -> Result<Settings, String> {
    Ok(Settings {
        library_dir: setting(conn, "library_dir")?,
        theme: setting(conn, "theme")?.unwrap_or_else(|| "light".into()),
        open_library: flag(conn, "open_library", true)?,
        google_books: flag(conn, "google_books", true)?,
        national_library: flag(conn, "national_library", true)?,
        lubimy_czytac: flag(conn, "lubimy_czytac", true)?,
        upolujebooka: flag(conn, "upolujebooka", true)?,
        google_books_has_key: false,
        locale: setting(conn, "locale")?.unwrap_or_else(|| "pl".into()),
        auto_backup: flag(conn, "auto_backup", false)?,
        backup_dir: setting(conn, "backup_dir")?,
        sync_provider: setting(conn, "sync_provider")?.unwrap_or_else(|| "folder".into()),
        sync_dir: setting(conn, "sync_dir")?,
        webdav_url: setting(conn, "webdav_url")?,
        webdav_user: setting(conn, "webdav_user")?,
        webdav_has_password: false,
        reading_goal: setting(conn, "reading_goal")?.and_then(|value| value.parse().ok()),
    })
}

pub fn save_settings(conn: &Connection, patch: SettingsPatch) -> Result<(), String> {
    if let Some(dir) = patch.library_dir {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            fs::create_dir_all(trimmed).map_err(|e| format!("Nie udało się utworzyć katalogu biblioteki: {e}"))?;
            put_setting(conn, "library_dir", trimmed)?;
        }
    }
    if let Some(theme) = patch.theme {
        if theme == "light" || theme == "dark" {
            put_setting(conn, "theme", &theme)?;
        }
    }
    if let Some(value) = patch.open_library {
        put_setting(conn, "open_library", if value { "1" } else { "0" })?;
    }
    if let Some(value) = patch.google_books {
        put_setting(conn, "google_books", if value { "1" } else { "0" })?;
    }
    if let Some(value) = patch.national_library {
        put_setting(conn, "national_library", if value { "1" } else { "0" })?;
    }
    if let Some(value) = patch.lubimy_czytac {
        put_setting(conn, "lubimy_czytac", if value { "1" } else { "0" })?;
    }
    if let Some(value) = patch.upolujebooka {
        put_setting(conn, "upolujebooka", if value { "1" } else { "0" })?;
    }
    if let Some(locale) = patch.locale {
        if locale == "pl" || locale == "en" {
            put_setting(conn, "locale", &locale)?;
        }
    }
    if let Some(value) = patch.auto_backup {
        put_setting(conn, "auto_backup", if value { "1" } else { "0" })?;
    }
    if let Some(dir) = patch.backup_dir {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            fs::create_dir_all(trimmed).map_err(|e| format!("Nie udało się utworzyć katalogu kopii: {e}"))?;
            put_setting(conn, "backup_dir", trimmed)?;
        }
    }
    if let Some(provider) = patch.sync_provider {
        if provider == "folder" || provider == "webdav" {
            put_setting(conn, "sync_provider", &provider)?;
        }
    }
    if let Some(dir) = patch.sync_dir {
        put_setting(conn, "sync_dir", dir.trim())?;
    }
    if let Some(url) = patch.webdav_url {
        let url = url.trim();
        if !url.is_empty() {
            crate::infrastructure::sync::validate_webdav_url(url)?;
        }
        put_setting(conn, "webdav_url", url)?;
    }
    if let Some(goal) = patch.reading_goal {
        if !(0..=10_000).contains(&goal) {
            return Err("Cel roczny musi być od 0 do 10000 książek.".into());
        }
        put_setting(conn, "reading_goal", &goal.to_string())?;
    }
    if let Some(user) = patch.webdav_user {
        put_setting(conn, "webdav_user", user.trim())?;
    }
    Ok(())
}

pub fn list_books(conn: &Connection, query: &ListQuery, paths: &StorePaths) -> Result<Vec<BookCard>, String> {
    let mut sql = String::from(
        "SELECT b.id, b.title, b.authors, b.publication_year, b.cover_path, b.series_name, b.series_number,
                b.tags, b.created_at, b.average_rating,
                EXISTS(SELECT 1 FROM book_editions e JOIN physical_copies p ON p.book_edition_id = e.id WHERE e.book_id = b.id),
                EXISTS(SELECT 1 FROM book_editions e JOIN digital_copies d ON d.book_edition_id = e.id WHERE e.book_id = b.id),
                (SELECT p.loaned_to FROM book_editions e JOIN physical_copies p ON p.book_edition_id = e.id
                  WHERE e.book_id = b.id AND p.loaned_to IS NOT NULL AND p.loaned_to != '' LIMIT 1),
                EXISTS(SELECT 1 FROM reading_progress r WHERE r.book_id = b.id AND r.finished_at IS NOT NULL),
                (SELECT MIN(p.return_due_date) FROM book_editions e JOIN physical_copies p ON p.book_edition_id = e.id
                  WHERE e.book_id = b.id AND p.loaned_to IS NOT NULL AND p.loaned_to != '' AND p.return_due_date IS NOT NULL AND p.return_due_date != ''),
                b.updated_at
         FROM books b",
    );
    let mut args: Vec<String> = Vec::new();
    let folded = fold(&query.q);
    let ranked = !folded.is_empty();
    if ranked {
        sql.push_str(" LEFT JOIN (SELECT book_id, bm25(books_fts) AS rank FROM books_fts WHERE books_fts MATCH ?) f ON f.book_id = b.id");
        args.push(fts_query(&folded));
    }
    sql.push_str(" WHERE b.deleted_at IS NULL");
    let mut view = query.view.clone();
    let mut author = query.author.clone();
    let mut tag = query.tag.clone();
    let mut manual_shelf_id: Option<String> = None;
    if let Some(shelf_id) = query.shelf_id.clone().filter(|id| !id.is_empty()) {
        let smart: Option<(i64, Option<String>)> = conn
            .query_row(
                "SELECT is_smart_shelf, smart_filter_json FROM shelves WHERE id = ?1 AND deleted_at IS NULL",
                params![shelf_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(map_sql)?;
        if let Some((is_smart, filter)) = smart {
            if is_smart == 1 {
                apply_smart_filter(filter.as_deref(), &mut view, &mut author, &mut tag);
            } else {
                sql.push_str(" AND EXISTS (SELECT 1 FROM shelf_items si WHERE si.shelf_id = ? AND si.book_id = b.id)");
                args.push(shelf_id.clone());
                manual_shelf_id = Some(shelf_id);
            }
        }
    }
    if ranked {
        // FTS daje ranking i przedrostki słów, LIKE łapie fragmenty w środku słowa (np. kawałek ISBN).
        sql.push_str(" AND (f.book_id IS NOT NULL OR b.search_text LIKE ? ESCAPE '\\')");
        args.push(like_pattern(&folded));
    }
    match view.as_str() {
        "paper" => sql.push_str(" AND EXISTS(SELECT 1 FROM book_editions e JOIN physical_copies p ON p.book_edition_id = e.id WHERE e.book_id = b.id)"),
        "ebook" => sql.push_str(" AND EXISTS(SELECT 1 FROM book_editions e JOIN digital_copies d ON d.book_edition_id = e.id WHERE e.book_id = b.id)"),
        "loaned" => sql.push_str(" AND EXISTS(SELECT 1 FROM book_editions e JOIN physical_copies p ON p.book_edition_id = e.id WHERE e.book_id = b.id AND p.loaned_to IS NOT NULL AND p.loaned_to != '')"),
        "overdue" => sql.push_str(" AND EXISTS(SELECT 1 FROM book_editions e JOIN physical_copies p ON p.book_edition_id = e.id WHERE e.book_id = b.id AND p.loaned_to IS NOT NULL AND p.loaned_to != '' AND p.return_due_date IS NOT NULL AND p.return_due_date != '' AND p.return_due_date < date('now', 'localtime'))"),
        "finished" => sql.push_str(" AND EXISTS(SELECT 1 FROM reading_progress r WHERE r.book_id = b.id AND r.finished_at IS NOT NULL)"),
        "unread" => sql.push_str(" AND NOT EXISTS(SELECT 1 FROM reading_progress r WHERE r.book_id = b.id AND r.finished_at IS NOT NULL)"),
        _ => {}
    }
    if let Some(author) = author.filter(|v| !v.trim().is_empty()) {
        sql.push_str(" AND (EXISTS(SELECT 1 FROM json_each(b.authors) WHERE value = ?) OR b.search_text LIKE ? ESCAPE '\\')");
        args.push(author.trim().to_string());
        args.push(like_pattern(&fold(&author)));
    }
    if let Some(series) = query.series.clone().filter(|v| !v.trim().is_empty()) {
        sql.push_str(" AND b.series_name = ?");
        args.push(series.trim().to_string());
    }
    if let Some(tag) = tag.filter(|v| !v.trim().is_empty()) {
        sql.push_str(" AND (EXISTS(SELECT 1 FROM json_each(b.tags) WHERE value = ?) OR EXISTS(SELECT 1 FROM json_each(b.genres) WHERE value = ?))");
        args.push(tag.trim().to_string());
        args.push(tag.trim().to_string());
    }
    if let Some(year) = query.year {
        sql.push_str(" AND b.publication_year = ?");
        args.push(year.to_string());
    }
    let order = match query.sort.as_str() {
        "author" => " ORDER BY b.authors COLLATE NOCASE, b.title COLLATE NOCASE".to_string(),
        "created" => " ORDER BY b.created_at DESC".to_string(),
        "rating" => " ORDER BY b.average_rating IS NULL, b.average_rating DESC, b.title COLLATE NOCASE".to_string(),
        "published" => " ORDER BY b.publication_year IS NULL, b.publication_year DESC, b.title COLLATE NOCASE".to_string(),
        "relevance" if ranked => " ORDER BY f.rank IS NULL, f.rank, b.title COLLATE NOCASE".to_string(),
        "shelf" if manual_shelf_id.is_some() => {
            args.push(manual_shelf_id.clone().unwrap_or_default());
            " ORDER BY (SELECT si.position FROM shelf_items si WHERE si.shelf_id = ? AND si.book_id = b.id), b.title COLLATE NOCASE".to_string()
        }
        _ => " ORDER BY b.title COLLATE NOCASE".to_string(),
    };
    sql.push_str(&order);
    let mut stmt = conn.prepare(&sql).map_err(map_sql)?;
    let mut cards = stmt
        .query_map(params_from_iter(args.iter()), map_card)
        .map_err(map_sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql)?;
    for card in &mut cards {
        card.cover_file = card
            .cover_path
            .as_deref()
            .filter(|value| !value.is_empty())
            .and_then(|value| resolve_path(paths, value))
            .filter(|path| path.is_file())
            .map(|path| path.display().to_string());
    }
    Ok(cards)
}

/// Zapytanie FTS5: każde słowo jako przedrostek, dłuższe słowa także bez dwóch ostatnich liter
/// (polska odmiana: „Tadeusza” znajdzie „Tadeusz”).
fn fts_query(folded: &str) -> String {
    folded
        .split_whitespace()
        .map(|word| word.chars().filter(|ch| ch.is_alphanumeric()).collect::<String>())
        .filter(|word| !word.is_empty())
        .map(|word| {
            let shorter = stem(&word);
            if shorter.is_empty() {
                format!("\"{word}\"*")
            } else {
                format!("(\"{word}\"* OR \"{shorter}\"*)")
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

pub fn get_book(conn: &Connection, id: &str) -> Result<BookDetails, String> {
    let book = load_book(conn, id)?.ok_or_else(|| "Nie ma takiej książki.".to_string())?;
    Ok(BookDetails {
        editions: load_editions(conn, id)?,
        physical: load_physical(conn, id)?,
        digital: load_digital(conn, id)?,
        shelves: load_book_shelves(conn, id)?,
        progress: load_progress(conn, id)?,
        book,
    })
}

pub fn save_book(
    conn: &Connection,
    draft: BookDraft,
    paths: &StorePaths,
    cover: Option<Vec<u8>>,
) -> Result<SaveResult, String> {
    let title = draft.title.trim().to_string();
    if title.is_empty() {
        return Err("Wpisz tytuł.".into());
    }
    let authors: Vec<String> = draft.authors.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    if authors.is_empty() {
        return Err("Wpisz co najmniej jednego autora, każdego w osobnej linii.".into());
    }
    let isbn13 = optional_isbn(draft.isbn13.as_deref())?;
    let isbn10 = optional_isbn(draft.isbn10.as_deref())?;
    for isbn in [isbn13.as_deref(), isbn10.as_deref()].into_iter().flatten() {
        if let Some(existing) = find_book_by_isbn(conn, isbn)? {
            if draft.id.as_deref() != Some(existing.as_str()) {
                if draft.id.is_some() {
                    // Edycja nie może po cichu przełączyć się na inną książkę.
                    return Err("Ten ISBN jest już przy innej książce.".into());
                }
                return Ok(SaveResult { book_id: existing, created: false });
            }
        }
    }
    let key = dedupe_key(&title, &authors);
    if let Some(existing) = find_book_by_key(conn, &key)? {
        if draft.id.as_deref() != Some(existing.as_str()) {
            if draft.id.is_some() {
                return Err("Książka o tym tytule i autorze już jest w katalogu.".into());
            }
            return Ok(SaveResult { book_id: existing, created: false });
        }
    }
    let tx = conn.unchecked_transaction().map_err(map_sql)?;
    let stamp = now();
    let created = draft.id.is_none();
    let id = draft.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
    let authors_json = serde_json::to_string(&authors).map_err(|e| e.to_string())?;
    let original_title = clean_opt(draft.original_title.clone());
    let subtitle = clean_opt(draft.subtitle.clone());
    let publisher = clean_opt(draft.publisher.clone());
    let language = clean_opt(draft.language.clone());
    let description = clean_opt(draft.description.clone());
    let series_name = clean_opt(draft.series_name.clone());
    let series_number = clean_opt(draft.series_number.clone());
    let translators = clean_list(&draft.translators);
    let genres = clean_list(&draft.genres);
    let tags = clean_list(&draft.tags);
    let source = draft.metadata_source.clone().unwrap_or_else(|| "manual".into());
    let mut fields = serde_json::Map::new();
    for name in ["title", "authors", "publisher", "publicationYear", "description", "language", "seriesName", "isbn"] {
        fields.insert(name.into(), serde_json::Value::String(source.clone()));
    }
    if created {
        tx.execute(
            "INSERT INTO books (id, title, original_title, subtitle, authors, translators, publisher, publication_year, language, description, series_name, series_number, genres, tags, average_rating, rating_source, field_sources, manually_edited_fields, dedupe_key, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,'[]',?18,?19,?19)",
            params![
                id, title, original_title, subtitle, authors_json,
                serde_json::to_string(&translators).unwrap(), publisher, draft.publication_year,
                language, description, series_name,
                series_number, serde_json::to_string(&genres).unwrap(), serde_json::to_string(&tags).unwrap(),
                draft.average_rating, draft.rating_source.clone().or_else(|| draft.metadata_source.clone()),
                serde_json::Value::Object(fields).to_string(), key, stamp
            ],
        ).map_err(map_sql)?;
    } else {
        let previous = load_book(&tx, &id)?.ok_or_else(|| "Nie ma takiej książki.".to_string())?;
        let mut manual = previous.manually_edited_fields.clone();
        let changes = [
            ("title", previous.title != title),
            ("authors", previous.authors != authors),
            ("subtitle", previous.subtitle != subtitle),
            ("publisher", previous.publisher != publisher),
            ("publicationYear", previous.publication_year != draft.publication_year),
            ("description", previous.description != description),
            ("language", previous.language != language),
            ("seriesName", previous.series_name != series_name),
            ("tags", previous.tags != tags),
            ("genres", previous.genres != genres),
        ];
        for (name, changed) in changes {
            if changed && !manual.iter().any(|field| field == name) {
                manual.push(name.to_string());
            }
        }
        tx.execute(
            "UPDATE books SET title=?1, original_title=?2, subtitle=?3, authors=?4, translators=?5, publisher=?6, publication_year=?7, language=?8, description=?9, series_name=?10, series_number=?11, genres=?12, tags=?13, average_rating=COALESCE(?14, average_rating), rating_source=COALESCE(?15, rating_source), manually_edited_fields=?16, dedupe_key=?17, updated_at=?18 WHERE id=?19",
            params![
                title, original_title, subtitle, authors_json,
                serde_json::to_string(&translators).unwrap(), publisher, draft.publication_year,
                language, description, series_name,
                series_number, serde_json::to_string(&genres).unwrap(), serde_json::to_string(&tags).unwrap(),
                draft.average_rating, draft.rating_source.clone(), serde_json::to_string(&manual).unwrap(), key, stamp, id
            ],
        ).map_err(map_sql)?;
    }
    let edition_id = upsert_edition(&tx, &id, &draft, &isbn10, &isbn13, &stamp)?;
    if let Some(bytes) = cover {
        let old: Option<String> = tx.query_row("SELECT cover_path FROM books WHERE id=?1", params![id], |row| row.get(0)).map_err(map_sql)?;
        let relative = write_cover(paths, &authors, &title, &id, &bytes, old.as_deref())?;
        tx.execute("UPDATE books SET cover_path=?1 WHERE id=?2", params![relative, id]).map_err(map_sql)?;
        tx.execute("UPDATE book_editions SET cover_path=?1 WHERE id=?2", params![relative, edition_id]).map_err(map_sql)?;
    }
    tx.commit().map_err(map_sql)?;
    rebuild_search(conn, &id)?;
    write_metadata_file(conn, paths, &id)?;
    Ok(SaveResult { book_id: id, created })
}

pub fn delete_book(conn: &Connection, id: &str) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE books SET deleted_at=?1, updated_at=?1, dedupe_key=NULL WHERE id=?2 AND deleted_at IS NULL",
            params![now(), id],
        )
        .map_err(map_sql)?;
    if changed == 0 {
        return Err("Nie ma takiej książki.".into());
    }
    rebuild_search(conn, id)?;
    Ok(())
}

pub fn save_physical(conn: &Connection, draft: PhysicalDraft) -> Result<String, String> {
    let edition = first_edition(conn, &draft.book_id)?;
    let stamp = now();
    let id = draft.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
    if draft.id.is_some() {
        conn.execute(
            "UPDATE physical_copies SET shelf_id=?1, location_note=?2, acquisition_date=?3, price=?4, condition=?5, owner=?6, loaned_to=?7, loaned_date=?8, return_due_date=?9, notes=?10, barcode_value=?11, updated_at=?12 WHERE id=?13",
            params![empty_to_none(draft.shelf_id.clone()), clean_opt(draft.location_note), clean_opt(draft.acquisition_date), clean_opt(draft.price), clean_opt(draft.condition), clean_opt(draft.owner), clean_opt(draft.loaned_to.clone()), clean_opt(draft.loaned_date), clean_opt(draft.return_due_date), clean_opt(draft.notes), clean_opt(draft.barcode_value), stamp, id],
        ).map_err(map_sql)?;
    } else {
        conn.execute(
            "INSERT INTO physical_copies (id, book_edition_id, shelf_id, location_note, acquisition_date, price, condition, owner, loaned_to, loaned_date, return_due_date, notes, barcode_value, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?14)",
            params![id, edition, empty_to_none(draft.shelf_id.clone()), clean_opt(draft.location_note), clean_opt(draft.acquisition_date), clean_opt(draft.price), clean_opt(draft.condition), clean_opt(draft.owner), clean_opt(draft.loaned_to), clean_opt(draft.loaned_date), clean_opt(draft.return_due_date), clean_opt(draft.notes), clean_opt(draft.barcode_value), stamp],
        ).map_err(map_sql)?;
    }
    if let Some(shelf_id) = empty_to_none(draft.shelf_id) {
        place_on_shelf(conn, &shelf_id, &draft.book_id)?;
    }
    touch_book(conn, &draft.book_id)?;
    rebuild_search(conn, &draft.book_id)?;
    Ok(id)
}

pub fn delete_physical(conn: &Connection, id: &str) -> Result<(), String> {
    let book_id: Option<String> = conn
        .query_row(
            "SELECT e.book_id FROM physical_copies p JOIN book_editions e ON e.id = p.book_edition_id WHERE p.id=?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sql)?;
    conn.execute("DELETE FROM physical_copies WHERE id=?1", params![id]).map_err(map_sql)?;
    if let Some(book_id) = book_id {
        touch_book(conn, &book_id)?;
        rebuild_search(conn, &book_id)?;
    }
    Ok(())
}

pub fn save_progress(conn: &Connection, draft: ProgressDraft) -> Result<String, String> {
    if let Some(rating) = draft.rating {
        if !(1..=5).contains(&rating) {
            return Err("Ocena musi być od 1 do 5.".into());
        }
    }
    if let Some(progress) = draft.progress_percent {
        if !(0.0..=100.0).contains(&progress) {
            return Err("Postęp musi być od 0 do 100.".into());
        }
    }
    let stamp = now();
    let id = draft.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
    if draft.id.is_some() {
        conn.execute(
            "UPDATE reading_progress SET format=?1, progress_percent=?2, current_page=?3, current_chapter=?4, started_at=?5, finished_at=?6, rating=?7, review=?8, notes=?9 WHERE id=?10",
            params![clean_opt(draft.format), draft.progress_percent, draft.current_page, clean_opt(draft.current_chapter), clean_opt(draft.started_at), clean_opt(draft.finished_at.clone()), draft.rating, clean_opt(draft.review), clean_opt(draft.notes), id],
        ).map_err(map_sql)?;
    } else {
        conn.execute(
            "INSERT INTO reading_progress (id, book_id, format, progress_percent, current_page, current_chapter, started_at, finished_at, rating, review, notes) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![id, draft.book_id, clean_opt(draft.format), draft.progress_percent, draft.current_page, clean_opt(draft.current_chapter), clean_opt(draft.started_at), clean_opt(draft.finished_at.clone()), draft.rating, clean_opt(draft.review), clean_opt(draft.notes)],
        ).map_err(map_sql)?;
    }
    if let Some(rating) = draft.rating {
        conn.execute(
            "UPDATE books SET average_rating=?1, rating_source='user' WHERE id=?2",
            params![rating as f64, draft.book_id],
        ).map_err(map_sql)?;
    } else if draft.id.is_some() {
        // Pusta ocena przy istniejącym wpisie znaczy: usuń moją ocenę. Ocena z serwisu zostaje.
        conn.execute(
            "UPDATE books SET average_rating=CASE WHEN rating_source='user' THEN NULL ELSE average_rating END,
                              rating_source=CASE WHEN rating_source='user' THEN NULL ELSE rating_source END WHERE id=?1",
            params![draft.book_id],
        ).map_err(map_sql)?;
    }
    // Każda zmiana postępu przesuwa datę książki, żeby trafiła do synchronizacji.
    conn.execute("UPDATE books SET updated_at=?1 WHERE id=?2", params![stamp, draft.book_id]).map_err(map_sql)?;
    rebuild_search(conn, &draft.book_id)?;
    Ok(id)
}

pub fn list_shelves(conn: &Connection) -> Result<Vec<ShelfRecord>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.name, s.description, s.color, s.icon, s.sort_order, s.is_smart_shelf, s.smart_filter_json, s.created_at, s.updated_at,
                    (SELECT COUNT(*) FROM shelf_items si JOIN books b ON b.id = si.book_id WHERE si.shelf_id = s.id AND b.deleted_at IS NULL),
                    s.deleted_at
             FROM shelves s WHERE s.deleted_at IS NULL ORDER BY s.sort_order, s.name",
        )
        .map_err(map_sql)?;
    let rows = stmt.query_map([], map_shelf).map_err(map_sql)?.collect::<Result<Vec<_>, _>>().map_err(map_sql)?;
    Ok(rows)
}

pub fn save_shelf(conn: &Connection, draft: ShelfDraft) -> Result<String, String> {
    let name = draft.name.trim().to_string();
    if name.is_empty() {
        return Err("Wpisz nazwę półki.".into());
    }
    let stamp = now();
    let id = draft.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
    if draft.id.is_some() {
        conn.execute(
            "UPDATE shelves SET name=?1, description=?2, color=?3, is_smart_shelf=?4, smart_filter_json=?5, updated_at=?6 WHERE id=?7",
            params![name, clean_opt(draft.description), clean_opt(draft.color), draft.is_smart_shelf as i64, clean_opt(draft.smart_filter_json), stamp, id],
        ).map_err(map_sql)?;
    } else {
        let order: i64 = conn.query_row("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM shelves", [], |row| row.get(0)).map_err(map_sql)?;
        conn.execute(
            "INSERT INTO shelves (id, name, description, color, sort_order, is_smart_shelf, smart_filter_json, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)",
            params![id, name, clean_opt(draft.description), clean_opt(draft.color), order, draft.is_smart_shelf as i64, clean_opt(draft.smart_filter_json), stamp],
        ).map_err(map_sql)?;
    }
    Ok(id)
}

pub fn delete_shelf(conn: &Connection, id: &str) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(map_sql)?;
    let mut affected = book_ids_on_shelf(&tx, id)?;
    let mut stmt = tx
        .prepare("SELECT DISTINCT e.book_id FROM physical_copies p JOIN book_editions e ON e.id = p.book_edition_id WHERE p.shelf_id=?1")
        .map_err(map_sql)?;
    let physical: Vec<String> = stmt.query_map(params![id], |row| row.get(0)).map_err(map_sql)?.collect::<Result<_, _>>().map_err(map_sql)?;
    drop(stmt);
    affected.extend(physical);
    tx.execute("DELETE FROM shelf_items WHERE shelf_id=?1", params![id]).map_err(map_sql)?;
    tx.execute("UPDATE physical_copies SET shelf_id=NULL WHERE shelf_id=?1", params![id]).map_err(map_sql)?;
    // Półka zostaje jako nagrobek, żeby usunięcie dotarło na inne komputery.
    let stamp = now();
    tx.execute("UPDATE shelves SET deleted_at=?1, updated_at=?1 WHERE id=?2", params![stamp, id]).map_err(map_sql)?;
    for book_id in affected {
        touch_book(&tx, &book_id)?;
    }
    tx.commit().map_err(map_sql)?;
    Ok(())
}

fn book_ids_on_shelf(conn: &Connection, shelf_id: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT book_id FROM shelf_items WHERE shelf_id=?1 AND book_id IS NOT NULL")
        .map_err(map_sql)?;
    let ids = stmt.query_map(params![shelf_id], |row| row.get(0)).map_err(map_sql)?.collect::<Result<Vec<String>, _>>().map_err(map_sql)?;
    Ok(ids)
}

fn manual_shelf(conn: &Connection, shelf_id: &str) -> Result<(), String> {
    let smart: Option<i64> = conn
        .query_row("SELECT is_smart_shelf FROM shelves WHERE id=?1 AND deleted_at IS NULL", params![shelf_id], |row| row.get(0))
        .optional()
        .map_err(map_sql)?;
    match smart {
        None => Err("Nie ma takiej półki.".into()),
        Some(1) => Err("Półki z filtrem układają się same.".into()),
        Some(_) => Ok(()),
    }
}

pub fn add_to_shelf(conn: &Connection, shelf_id: &str, book_id: &str) -> Result<(), String> {
    manual_shelf(conn, shelf_id)?;
    load_book(conn, book_id)?.filter(|book| book.deleted_at.is_none()).ok_or_else(|| "Nie ma takiej książki.".to_string())?;
    place_on_shelf(conn, shelf_id, book_id)?;
    touch_book(conn, book_id)
}

pub fn remove_from_shelf(conn: &Connection, shelf_id: &str, book_id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM shelf_items WHERE shelf_id=?1 AND book_id=?2", params![shelf_id, book_id]).map_err(map_sql)?;
    touch_book(conn, book_id)
}

pub fn reorder_shelf(conn: &Connection, shelf_id: &str, book_ids: &[String]) -> Result<(), String> {
    manual_shelf(conn, shelf_id)?;
    let tx = conn.unchecked_transaction().map_err(map_sql)?;
    let before = book_ids_on_shelf(&tx, shelf_id)?;
    tx.execute("DELETE FROM shelf_items WHERE shelf_id=?1", params![shelf_id]).map_err(map_sql)?;
    let mut seen = HashSet::new();
    let book_ids: Vec<&String> = book_ids.iter().filter(|id| seen.insert(id.as_str())).collect();
    for (index, book_id) in book_ids.iter().enumerate() {
        tx.execute(
            "INSERT INTO shelf_items (id, shelf_id, book_id, position, added_at) VALUES (?1,?2,?3,?4,?5)",
            params![Uuid::new_v4().to_string(), shelf_id, book_id, index as i64, now()],
        ).map_err(map_sql)?;
    }
    let touched: HashSet<&str> = before.iter().map(String::as_str).chain(book_ids.iter().map(|id| id.as_str())).collect();
    for book_id in touched {
        touch_book(&tx, book_id)?;
    }
    tx.commit().map_err(map_sql)?;
    Ok(())
}

pub fn place_on_shelf(conn: &Connection, shelf_id: &str, book_id: &str) -> Result<(), String> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM shelf_items WHERE shelf_id=?1 AND book_id=?2",
            params![shelf_id, book_id],
            |row| row.get(0),
        )
        .map_err(map_sql)?;
    if exists > 0 {
        return Ok(());
    }
    let position: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM shelf_items WHERE shelf_id=?1",
            params![shelf_id],
            |row| row.get(0),
        )
        .map_err(map_sql)?;
    conn.execute(
        "INSERT INTO shelf_items (id, shelf_id, book_id, position, added_at) VALUES (?1,?2,?3,?4,?5)",
        params![Uuid::new_v4().to_string(), shelf_id, book_id, position, now()],
    )
    .map_err(map_sql)?;
    Ok(())
}

pub fn list_files(conn: &Connection, paths: &StorePaths) -> Result<Vec<FileRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT d.id, e.book_id, b.title, d.file_name, d.file_path, d.file_format, d.file_size, d.file_hash_sha256
             FROM digital_copies d
             JOIN book_editions e ON e.id = d.book_edition_id
             JOIN books b ON b.id = e.book_id
             WHERE b.deleted_at IS NULL
             ORDER BY b.title, d.file_name",
        )
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(map_sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql)?;
    let mut files = Vec::new();
    for (id, book_id, title, name, path, format, size, _hash) in rows {
        // Lista sprawdza tylko obecność i rozmiar. Pełny skrót SHA-256 liczy „Sprawdź bibliotekę”.
        let status = match resolve_path(paths, &path).and_then(|abs| fs::metadata(abs).ok()) {
            Some(meta) if meta.is_file() && meta.len() as i64 == size => "present".to_string(),
            Some(meta) if meta.is_file() => "changed".to_string(),
            _ => "missing".to_string(),
        };
        files.push(FileRow {
            id,
            book_id,
            book_title: title,
            file_name: name,
            file_path: path,
            file_format: format,
            file_size: size,
            status,
        });
    }
    Ok(files)
}

pub fn register_digital(
    conn: &Connection,
    book_id: &str,
    source: &Path,
    paths: &StorePaths,
) -> Result<String, String> {
    let library = paths.library.as_ref().ok_or_else(|| "Najpierw wybierz katalog biblioteki w ustawieniach.".to_string())?;
    let hash = crate::infrastructure::files::hash_file(source)?;
    if let Some(title) = find_title_by_hash(conn, &hash)? {
        return Err(format!("Ten plik jest już przy książce „{title}”."));
    }
    let book = load_book(conn, book_id)?.ok_or_else(|| "Nie ma takiej książki.".to_string())?;
    let author = book.authors.first().map(String::as_str).unwrap_or("Nieznany autor");
    let dir = crate::infrastructure::files::book_directory(library, author, &book.title, book_id)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file_name = source
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "plik".into());
    let safe_name = safe_file_name(&file_name);
    let target = dir.join(&safe_name);
    if target.exists() && crate::infrastructure::files::hash_file(&target).ok().as_deref() != Some(hash.as_str()) {
        return Err(format!("W folderze książki jest już inny plik o nazwie „{safe_name}”."));
    }
    if source != target {
        crate::infrastructure::files::copy_atomic(source, &target)?;
    }
    let relative = relative_to(library, &target)?;
    let edition = first_edition(conn, book_id)?;
    let format = target
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| "bin".into());
    let size = fs::metadata(&target).map_err(|e| e.to_string())?.len() as i64;
    let id = Uuid::new_v4().to_string();
    if let Err(error) = conn.execute(
        "INSERT INTO digital_copies (id, book_edition_id, file_path, file_name, file_size, file_hash_sha256, file_format, added_at, last_verified_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)",
        params![id, edition, relative, safe_name, size, hash, format, now()],
    ) {
        // Nie zostawiamy w bibliotece kopii pliku, której nie ma w bazie.
        if source != target {
            let _ = fs::remove_file(&target);
        }
        return Err(map_sql(error));
    }
    touch_book(conn, book_id)?;
    rebuild_search(conn, book_id)?;
    write_metadata_file(conn, paths, book_id)?;
    Ok(id)
}

/// Pliki do pełnej weryfikacji: wiersz, pełna ścieżka i zapisany skrót. Skróty liczy się poza blokadą bazy.
pub fn verify_targets(conn: &Connection, paths: &StorePaths) -> Result<Vec<(FileRow, Option<PathBuf>, String)>, String> {
    let rows = list_files(conn, paths)?;
    let mut out = Vec::new();
    for row in rows {
        let hash: String = conn
            .query_row("SELECT file_hash_sha256 FROM digital_copies WHERE id=?1", params![row.id], |r| r.get(0))
            .map_err(map_sql)?;
        let absolute = resolve_path(paths, &row.file_path);
        out.push((row, absolute, hash));
    }
    Ok(out)
}

pub fn mark_verified(conn: &Connection, ids: &[String]) -> Result<(), String> {
    let stamp = now();
    for id in ids {
        conn.execute("UPDATE digital_copies SET last_verified_at=?1 WHERE id=?2", params![stamp, id]).map_err(map_sql)?;
    }
    Ok(())
}

pub struct CatalogFile {
    pub title: String,
    pub author: String,
    pub absolute: PathBuf,
    pub format: String,
}

pub fn catalog_files(conn: &Connection, book_id: &str, paths: &StorePaths) -> Result<Vec<CatalogFile>, String> {
    let book = load_book(conn, book_id)?.ok_or_else(|| "Nie ma takiej książki.".to_string())?;
    let author = book.authors.first().cloned().unwrap_or_else(|| "Nieznany autor".into());
    let mut stmt = conn
        .prepare(
            "SELECT d.file_path, d.file_format FROM digital_copies d
             JOIN book_editions e ON e.id = d.book_edition_id
             WHERE e.book_id=?1",
        )
        .map_err(map_sql)?;
    let rows = stmt
        .query_map(params![book_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(map_sql)?;
    let mut out = Vec::new();
    for row in rows {
        let (relative, format) = row.map_err(map_sql)?;
        let Some(absolute) = resolve_path(paths, &relative) else { continue };
        if absolute.is_file() {
            out.push(CatalogFile { title: book.title.clone(), author: author.clone(), absolute, format });
        }
    }
    Ok(out)
}

pub fn title_for_hash(conn: &Connection, hash: &str) -> Result<Option<String>, String> {
    find_title_by_hash(conn, hash)
}

pub fn absolute_digital_path(conn: &Connection, id: &str, paths: &StorePaths) -> Result<PathBuf, String> {
    let relative: String = conn
        .query_row("SELECT file_path FROM digital_copies WHERE id=?1", params![id], |row| row.get(0))
        .map_err(|_| "Nie ma takiego pliku.".to_string())?;
    resolve_path(paths, &relative).ok_or_else(|| "Najpierw wybierz katalog biblioteki.".to_string())
}

pub fn snapshot(conn: &Connection) -> Result<Snapshot, String> {
    Ok(Snapshot {
        version: 1,
        books: all_books(conn)?,
        editions: all_editions(conn)?,
        shelves: all_shelves(conn)?,
        physical: all_physical(conn)?,
        digital: all_digital(conn)?,
        shelf_items: all_shelf_items(conn)?,
        progress: all_progress(conn)?,
    })
}

pub fn import_snapshot(conn: &Connection, snap: &Snapshot) -> Result<ImportReport, String> {
    import_snapshot_with(conn, snap, false)
}

/// `force` przyjmuje książki ze snapshotu nawet wtedy, gdy lokalna data jest nowsza.
/// Używa tego synchronizacja, gdy trójstronne porównanie już ustaliło, że wygrywa strona zdalna.
pub fn import_snapshot_with(conn: &Connection, snap: &Snapshot, force: bool) -> Result<ImportReport, String> {
    let tx = conn.unchecked_transaction().map_err(map_sql)?;
    tx.execute_batch("PRAGMA defer_foreign_keys = ON;").map_err(map_sql)?;
    let mut report = ImportReport::default();
    let mut taken: HashSet<String> = HashSet::new();
    for book in &snap.books {
        let local = load_book(&tx, &book.id)?;
        let take = match &local {
            None => true,
            Some(current) => force || book.updated_at > current.updated_at,
        };
        if !take {
            continue;
        }
        if book.deleted_at.is_none() && clashes_with_other_book(&tx, snap, book)? {
            report.skipped += 1;
            report.skipped_titles.push(book.title.clone());
            continue;
        }
        if local.is_some() {
            report.updated += 1;
            tx.execute("DELETE FROM physical_copies WHERE book_edition_id IN (SELECT id FROM book_editions WHERE book_id=?1)", params![book.id]).map_err(map_sql)?;
            tx.execute("DELETE FROM digital_copies WHERE book_edition_id IN (SELECT id FROM book_editions WHERE book_id=?1)", params![book.id]).map_err(map_sql)?;
            tx.execute("DELETE FROM shelf_items WHERE book_id=?1", params![book.id]).map_err(map_sql)?;
            tx.execute("DELETE FROM reading_progress WHERE book_id=?1", params![book.id]).map_err(map_sql)?;
            tx.execute("DELETE FROM book_editions WHERE book_id=?1", params![book.id]).map_err(map_sql)?;
            update_book_row(&tx, book)?;
        } else {
            report.inserted += 1;
            insert_book_row(&tx, book)?;
        }
        taken.insert(book.id.clone());
    }
    for shelf in &snap.shelves {
        let local: Option<String> = tx
            .query_row("SELECT updated_at FROM shelves WHERE id=?1", params![shelf.id], |row| row.get(0))
            .optional()
            .map_err(map_sql)?;
        if local.as_deref().is_some_and(|current| current >= shelf.updated_at.as_str()) {
            continue;
        }
        tx.execute(
            "INSERT INTO shelves (id, name, description, color, icon, sort_order, is_smart_shelf, smart_filter_json, created_at, updated_at, deleted_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, description=excluded.description, color=excluded.color, is_smart_shelf=excluded.is_smart_shelf, smart_filter_json=excluded.smart_filter_json, updated_at=excluded.updated_at, deleted_at=excluded.deleted_at",
            params![shelf.id, shelf.name, shelf.description, shelf.color, shelf.icon, shelf.sort_order, shelf.is_smart_shelf as i64, shelf.smart_filter_json, shelf.created_at, shelf.updated_at, shelf.deleted_at],
        ).map_err(map_sql)?;
        if shelf.deleted_at.is_some() {
            tx.execute("DELETE FROM shelf_items WHERE shelf_id=?1", params![shelf.id]).map_err(map_sql)?;
            tx.execute("UPDATE physical_copies SET shelf_id=NULL WHERE shelf_id=?1", params![shelf.id]).map_err(map_sql)?;
        }
    }
    // Dzieci tylko dla przyjętych książek. Inaczej starszy eksport nadpisałby nowsze dane lokalne.
    let edition_book: std::collections::HashMap<&str, &str> = snap.editions.iter().map(|e| (e.id.as_str(), e.book_id.as_str())).collect();
    let accepted = |edition_id: &str| edition_book.get(edition_id).is_some_and(|book_id| taken.contains(*book_id));
    for edition in snap.editions.iter().filter(|e| taken.contains(&e.book_id)) {
        insert_edition_row(&tx, edition)?;
    }
    for copy in snap.physical.iter().filter(|c| accepted(&c.book_edition_id)) {
        let mut copy = copy.clone();
        if let Some(shelf_id) = &copy.shelf_id {
            if !live_shelf(&tx, shelf_id)? {
                copy.shelf_id = None;
            }
        }
        insert_physical_row(&tx, &copy)?;
    }
    for file in snap.digital.iter().filter(|f| accepted(&f.book_edition_id)) {
        // Ścieżka z obcego pliku musi być względna i nie może wychodzić poza bibliotekę.
        let Some(relative) = safe_relative(&file.file_path) else { continue };
        let owner = edition_book.get(file.book_edition_id.as_str()).copied().unwrap_or_default();
        let clash: Option<String> = tx
            .query_row(
                "SELECT d.id FROM digital_copies d JOIN book_editions e ON e.id = d.book_edition_id JOIN books b ON b.id = e.book_id
                 WHERE d.file_hash_sha256=?1 AND d.id != ?2 AND b.deleted_at IS NULL AND b.id != ?3 LIMIT 1",
                params![file.file_hash_sha256, file.id, owner],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_sql)?;
        if clash.is_some() {
            continue;
        }
        let mut file = file.clone();
        file.file_path = relative.to_string_lossy().replace('\\', "/");
        insert_digital_row(&tx, &file)?;
    }
    for item in &snap.shelf_items {
        let Some(book_id) = item.book_id.as_deref() else { continue };
        if !taken.contains(book_id) || !live_shelf(&tx, &item.shelf_id)? {
            continue;
        }
        tx.execute(
            "INSERT OR REPLACE INTO shelf_items (id, shelf_id, book_id, book_edition_id, position, added_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![item.id, item.shelf_id, item.book_id, item.book_edition_id, item.position, item.added_at],
        ).map_err(map_sql)?;
    }
    for progress in snap.progress.iter().filter(|p| taken.contains(&p.book_id)) {
        insert_progress_row(&tx, progress)?;
    }
    for book_id in &taken {
        rebuild_search(&tx, book_id)?;
    }
    tx.commit().map_err(map_sql)?;
    Ok(report)
}

fn live_shelf(conn: &Connection, shelf_id: &str) -> Result<bool, String> {
    let found: Option<i64> = conn
        .query_row("SELECT 1 FROM shelves WHERE id=?1 AND deleted_at IS NULL", params![shelf_id], |row| row.get(0))
        .optional()
        .map_err(map_sql)?;
    Ok(found.is_some())
}

/// Czy przychodząca książka zderza się z inną, żywą książką w katalogu (tytuł i autor albo ISBN).
fn clashes_with_other_book(conn: &Connection, snap: &Snapshot, book: &BookRecord) -> Result<bool, String> {
    let key = dedupe_key(&book.title, &book.authors);
    if find_book_by_key(conn, &key)?.is_some_and(|id| id != book.id) {
        return Ok(true);
    }
    for edition in snap.editions.iter().filter(|e| e.book_id == book.id) {
        for isbn in [edition.isbn13.as_deref(), edition.isbn10.as_deref()].into_iter().flatten().filter(|v| !v.is_empty()) {
            if find_book_by_isbn(conn, isbn)?.is_some_and(|id| id != book.id) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub fn list_backups(conn: &Connection) -> Result<Vec<BackupRow>, String> {
    let mut stmt = conn
        .prepare("SELECT id, type, destination, status, started_at, finished_at, size_bytes, checksum, error_message FROM backup_jobs ORDER BY started_at DESC LIMIT 100")
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(BackupRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                destination: row.get(2)?,
                status: row.get(3)?,
                started_at: row.get(4)?,
                finished_at: row.get(5)?,
                size_bytes: row.get(6)?,
                checksum: row.get(7)?,
                error_message: row.get(8)?,
            })
        })
        .map_err(map_sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql)?;
    Ok(rows)
}

pub fn record_backup(conn: &Connection, row: &BackupRow) -> Result<(), String> {
    conn.execute(
        "INSERT INTO backup_jobs (id, type, destination, status, started_at, finished_at, size_bytes, checksum, error_message) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![row.id, row.kind, row.destination, row.status, row.started_at, row.finished_at, row.size_bytes, row.checksum, row.error_message],
    )
    .map_err(map_sql)?;
    Ok(())
}

pub fn list_conflicts(conn: &Connection) -> Result<Vec<ConflictRow>, String> {
    let mut stmt = conn
        .prepare("SELECT id, book_id, summary, local_updated_at, remote_updated_at, created_at FROM sync_conflicts WHERE resolved_at IS NULL ORDER BY created_at DESC")
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ConflictRow {
                id: row.get(0)?,
                book_id: row.get(1)?,
                summary: row.get(2)?,
                local_updated_at: row.get(3)?,
                remote_updated_at: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(map_sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql)?;
    Ok(rows)
}

pub fn add_conflict(conn: &Connection, book_id: Option<&str>, summary: &str, local_at: Option<&str>, remote_at: Option<&str>) -> Result<(), String> {
    conn.execute(
        "INSERT INTO sync_conflicts (id, book_id, summary, local_updated_at, remote_updated_at, created_at) VALUES (?1,?2,?3,?4,?5,?6)",
        params![Uuid::new_v4().to_string(), book_id, summary, local_at, remote_at, now()],
    )
    .map_err(map_sql)?;
    Ok(())
}

pub fn resolve_conflict(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("UPDATE sync_conflicts SET resolved_at=?1 WHERE id=?2", params![now(), id]).map_err(map_sql)?;
    Ok(())
}

pub fn book_count(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM books WHERE deleted_at IS NULL", [], |row| row.get(0)).map_err(map_sql)
}

pub fn schema_version(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COALESCE(MAX(version), 0) FROM schema_migrations", [], |row| row.get(0)).map_err(map_sql)
}

fn upsert_edition(conn: &Connection, book_id: &str, draft: &BookDraft, isbn10: &Option<String>, isbn13: &Option<String>, stamp: &str) -> Result<String, String> {
    let external = serde_json::json!({
        "source": draft.metadata_source,
        "id": draft.external_id,
    });
    if let Some(edition_id) = &draft.edition_id {
        conn.execute(
            "UPDATE book_editions SET isbn10=?1, isbn13=?2, publisher=?3, publication_date=?4, page_count=?5, metadata_source=?6, external_ids=?7, updated_at=?8 WHERE id=?9",
            params![isbn10, isbn13, clean_opt(draft.publisher.clone()), draft.publication_year.map(|y| y.to_string()), draft.page_count, draft.metadata_source, external.to_string(), stamp, edition_id],
        ).map_err(map_sql)?;
        return Ok(edition_id.clone());
    }
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM book_editions WHERE book_id=?1 ORDER BY created_at LIMIT 1",
            params![book_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sql)?;
    if let Some(edition_id) = existing {
        conn.execute(
            "UPDATE book_editions SET isbn10=?1, isbn13=?2, publisher=?3, publication_date=?4, page_count=?5, metadata_source=?6, external_ids=?7, updated_at=?8 WHERE id=?9",
            params![isbn10, isbn13, clean_opt(draft.publisher.clone()), draft.publication_year.map(|y| y.to_string()), draft.page_count, draft.metadata_source, external.to_string(), stamp, edition_id],
        ).map_err(map_sql)?;
        return Ok(edition_id);
    }
    let edition_id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO book_editions (id, book_id, isbn10, isbn13, publisher, publication_date, page_count, format, external_ids, metadata_source, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,'book',?8,?9,?10,?10)",
        params![edition_id, book_id, isbn10, isbn13, clean_opt(draft.publisher.clone()), draft.publication_year.map(|y| y.to_string()), draft.page_count, external.to_string(), draft.metadata_source, stamp],
    ).map_err(map_sql)?;
    Ok(edition_id)
}

pub(crate) fn first_edition(conn: &Connection, book_id: &str) -> Result<String, String> {
    if let Some(id) = conn
        .query_row("SELECT id FROM book_editions WHERE book_id=?1 ORDER BY created_at LIMIT 1", params![book_id], |row| row.get(0))
        .optional()
        .map_err(map_sql)?
    {
        return Ok(id);
    }
    let id = Uuid::new_v4().to_string();
    let stamp = now();
    conn.execute(
        "INSERT INTO book_editions (id, book_id, format, external_ids, created_at, updated_at) VALUES (?1,?2,'book','{}',?3,?3)",
        params![id, book_id, stamp],
    )
    .map_err(map_sql)?;
    Ok(id)
}

pub(crate) fn rebuild_search(conn: &Connection, book_id: &str) -> Result<(), String> {
    let book = load_book(conn, book_id)?.ok_or_else(|| "Nie ma takiej książki.".to_string())?;
    let editions = load_editions(conn, book_id)?;
    let physical = load_physical(conn, book_id)?;
    let digital = load_digital(conn, book_id)?;
    let progress = load_progress(conn, book_id)?;
    let mut parts = vec![
        book.title,
        book.authors.join(" "),
        book.translators.join(" "),
        book.original_title.unwrap_or_default(),
        book.subtitle.unwrap_or_default(),
        book.publisher.unwrap_or_default(),
        book.description.unwrap_or_default(),
        book.series_name.unwrap_or_default(),
        book.series_number.unwrap_or_default(),
        book.genres.join(" "),
        book.tags.join(" "),
        book.language.unwrap_or_default(),
    ];
    for edition in editions {
        parts.push(edition.isbn10.unwrap_or_default());
        parts.push(edition.isbn13.unwrap_or_default());
        parts.push(edition.publisher.unwrap_or_default());
    }
    for copy in physical {
        parts.push(copy.notes.unwrap_or_default());
        parts.push(copy.location_note.unwrap_or_default());
        parts.push(copy.loaned_to.unwrap_or_default());
    }
    for file in digital {
        parts.push(file.file_name);
    }
    for item in progress {
        parts.push(item.notes.unwrap_or_default());
        parts.push(item.review.unwrap_or_default());
    }
    let search = fold(&parts.join(" "));
    conn.execute("UPDATE books SET search_text=?1 WHERE id=?2", params![search, book_id]).map_err(map_sql)?;
    conn.execute("DELETE FROM books_fts WHERE book_id=?1", params![book_id]).map_err(map_sql)?;
    if book.deleted_at.is_none() {
        conn.execute("INSERT INTO books_fts (book_id, body) VALUES (?1, ?2)", params![book_id, search]).map_err(map_sql)?;
    }
    Ok(())
}

/// Przelicza indeks wyszukiwania dla całego katalogu (po migracji albo przywróceniu kopii).
pub fn reindex_all(conn: &Connection) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(map_sql)?;
    tx.execute("DELETE FROM books_fts", []).map_err(map_sql)?;
    let mut stmt = tx.prepare("SELECT id FROM books WHERE deleted_at IS NULL").map_err(map_sql)?;
    let ids: Vec<String> = stmt.query_map([], |row| row.get(0)).map_err(map_sql)?.collect::<Result<_, _>>().map_err(map_sql)?;
    drop(stmt);
    for id in ids {
        rebuild_search(&tx, &id)?;
    }
    tx.commit().map_err(map_sql)?;
    Ok(())
}

/// Zamienia zapisane ścieżki bezwzględne leżące w katalogu biblioteki na względne.
/// Starsze wersje zapisywały je bezwzględnie przez błąd w porównaniu ścieżek w Windows.
pub fn normalize_paths(conn: &Connection) -> Result<(), String> {
    let Some(library) = setting(conn, "library_dir")? else { return Ok(()) };
    let root = PathBuf::from(&library);
    for (table, column) in [("digital_copies", "file_path"), ("books", "cover_path"), ("book_editions", "cover_path")] {
        let mut stmt = conn
            .prepare(&format!("SELECT id, {column} FROM {table} WHERE {column} IS NOT NULL AND {column} != ''"))
            .map_err(map_sql)?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(map_sql)?
            .collect::<Result<_, _>>()
            .map_err(map_sql)?;
        drop(stmt);
        for (id, stored) in rows {
            let path = Path::new(&stored);
            if !path.is_absolute() {
                continue;
            }
            if let Some(relative) = relative_inside(&root, path) {
                conn.execute(&format!("UPDATE {table} SET {column}=?1 WHERE id=?2"), params![relative, id]).map_err(map_sql)?;
            }
        }
    }
    Ok(())
}

pub struct EnrichTarget {
    pub id: String,
    pub title: String,
    pub author: String,
    pub isbn: Option<String>,
    pub has_cover: bool,
}

pub fn books_to_enrich(conn: &Connection) -> Result<Vec<EnrichTarget>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT b.id, b.title, b.authors, b.cover_path,
                (SELECT COALESCE(e.isbn13, e.isbn10) FROM book_editions e WHERE e.book_id = b.id AND (e.isbn13 IS NOT NULL OR e.isbn10 IS NOT NULL) LIMIT 1)
             FROM books b
             WHERE b.deleted_at IS NULL
               AND (b.cover_path IS NULL OR b.cover_path = '' OR b.description IS NULL OR b.description = '' OR b.publisher IS NULL OR b.publisher = '')",
        )
        .map_err(map_sql)?;
    let rows = stmt
        .query_map([], |row| {
            let authors: String = row.get(2)?;
            let parsed = parse_string_list(&authors, "");
            Ok(EnrichTarget {
                id: row.get(0)?,
                title: row.get(1)?,
                author: parsed.into_iter().next().unwrap_or_default(),
                has_cover: row.get::<_, Option<String>>(3)?.is_some_and(|path| !path.is_empty()),
                isbn: row.get(4)?,
            })
        })
        .map_err(map_sql)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(map_sql)?);
    }
    Ok(out)
}

pub fn merge_hit(conn: &Connection, paths: &StorePaths, book_id: &str, hit: &crate::domain::MetadataHit, cover: Option<Vec<u8>>) -> Result<bool, String> {
    let book = load_book(conn, book_id)?.ok_or_else(|| "Nie ma takiej książki.".to_string())?;
    let locked = |name: &str| book.manually_edited_fields.iter().any(|field| field == name);
    let stamp = now();
    let mut changed = false;
    let description = if book.description.as_ref().is_none_or(|v| v.trim().is_empty()) && !locked("description") { hit.description.clone() } else { None };
    let publisher = if book.publisher.as_ref().is_none_or(|v| v.trim().is_empty()) && !locked("publisher") { hit.publisher.clone() } else { None };
    let year = if book.publication_year.is_none() && !locked("publicationYear") { hit.publication_year } else { None };
    let language = if book.language.as_ref().is_none_or(|v| v.trim().is_empty()) && !locked("language") { hit.language.clone() } else { None };
    let series = if book.series_name.as_ref().is_none_or(|v| v.trim().is_empty()) && !locked("seriesName") { hit.series_name.clone() } else { None };
    let subtitle = if book.subtitle.as_ref().is_none_or(|v| v.trim().is_empty()) { hit.subtitle.clone() } else { None };
    let genres = if book.genres.is_empty() && !locked("genres") && !hit.genres.is_empty() { Some(serde_json::to_string(&hit.genres).unwrap()) } else { None };
    if description.is_some() || publisher.is_some() || year.is_some() || language.is_some() || series.is_some() || subtitle.is_some() || genres.is_some() {
        conn.execute(
            "UPDATE books SET description=COALESCE(NULLIF(description, ''), ?1), publisher=COALESCE(NULLIF(publisher, ''), ?2), publication_year=COALESCE(publication_year, ?3), language=COALESCE(NULLIF(language, ''), ?4), series_name=COALESCE(NULLIF(series_name, ''), ?5), subtitle=COALESCE(NULLIF(subtitle, ''), ?6), genres=CASE WHEN genres IS NULL OR genres='[]' THEN COALESCE(?7, genres) ELSE genres END, average_rating=COALESCE(average_rating, ?8), updated_at=?9 WHERE id=?10",
            params![description, publisher, year, language, series, subtitle, genres, hit.rating, stamp, book_id],
        ).map_err(map_sql)?;
        changed = true;
    }
    if book.cover_path.as_ref().is_none_or(|v| v.trim().is_empty()) {
        if let Some(bytes) = cover {
            let relative = write_cover(paths, &book.authors, &book.title, book_id, &bytes, book.cover_path.as_deref())?;
            conn.execute("UPDATE books SET cover_path=?1, updated_at=?2 WHERE id=?3", params![relative, stamp, book_id]).map_err(map_sql)?;
            conn.execute("UPDATE book_editions SET cover_path=?1, updated_at=?2 WHERE book_id=?3", params![relative, stamp, book_id]).map_err(map_sql)?;
            changed = true;
        }
    }
    let edition = first_edition(conn, book_id)?;
    let current_isbn: Option<String> = conn
        .query_row("SELECT COALESCE(isbn13, isbn10) FROM book_editions WHERE id=?1", params![edition], |row| row.get(0))
        .optional()
        .map_err(map_sql)?
        .flatten();
    if current_isbn.as_ref().is_none_or(|v| v.is_empty()) {
        let isbn13_free = hit.isbn13.as_deref().map(|value| find_book_by_isbn(conn, value)).transpose()?.flatten().is_none();
        let isbn10_free = hit.isbn10.as_deref().map(|value| find_book_by_isbn(conn, value)).transpose()?.flatten().is_none();
        if isbn13_free && isbn10_free && (hit.isbn13.is_some() || hit.isbn10.is_some()) {
            conn.execute(
                "UPDATE book_editions SET isbn13=COALESCE(isbn13, ?1), isbn10=COALESCE(isbn10, ?2), page_count=COALESCE(page_count, ?3), publisher=COALESCE(publisher, ?4), metadata_source=?5, updated_at=?6 WHERE id=?7",
                params![hit.isbn13, hit.isbn10, hit.page_count, hit.publisher, hit.source, stamp, edition],
            ).map_err(map_sql)?;
            changed = true;
        }
    }
    if changed {
        rebuild_search(conn, book_id)?;
    }
    Ok(changed)
}

pub fn attach_cover(conn: &Connection, paths: &StorePaths, book_id: &str, bytes: &[u8]) -> Result<(), String> {
    let book = load_book(conn, book_id)?.ok_or_else(|| "Nie ma takiej książki.".to_string())?;
    let relative = write_cover(paths, &book.authors, &book.title, book_id, bytes, book.cover_path.as_deref())?;
    conn.execute("UPDATE books SET cover_path=?1, updated_at=?2 WHERE id=?3", params![relative, now(), book_id]).map_err(map_sql)?;
    conn.execute("UPDATE book_editions SET cover_path=?1, updated_at=?2 WHERE book_id=?3", params![relative, now(), book_id]).map_err(map_sql)?;
    Ok(())
}

pub(crate) fn write_cover(paths: &StorePaths, authors: &[String], title: &str, book_id: &str, bytes: &[u8], old: Option<&str>) -> Result<String, String> {
    let extension = match crate::infrastructure::metadata::image_type(bytes) {
        Some("image/png") => "png",
        Some("image/gif") => "gif",
        Some("image/webp") => "webp",
        Some(_) => "jpg",
        None => return Err("Ten plik nie jest obrazem okładki.".into()),
    };
    let (target, stored) = if let Some(library) = &paths.library {
        let author = authors.first().map(String::as_str).unwrap_or("Nieznany autor");
        let dir = crate::infrastructure::files::book_directory(library, author, title, book_id)?;
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let target = dir.join(format!("cover.{extension}"));
        crate::infrastructure::files::write_atomic(&target, bytes)?;
        let stored = relative_to(library, &target)?;
        (target, stored)
    } else {
        fs::create_dir_all(&paths.covers).map_err(|e| e.to_string())?;
        let target = paths.covers.join(format!("{book_id}.{extension}"));
        crate::infrastructure::files::write_atomic(&target, bytes)?;
        let stored = target.display().to_string();
        (target, stored)
    };
    // Stara okładka w innym formacie zostałaby sierotą.
    if let Some(previous) = old.filter(|value| !value.is_empty()).and_then(|value| resolve_path(paths, value)) {
        if previous != target && previous.is_file() {
            let _ = fs::remove_file(previous);
        }
    }
    Ok(stored)
}

pub(crate) fn write_metadata_file(conn: &Connection, paths: &StorePaths, book_id: &str) -> Result<(), String> {
    let Some(library) = &paths.library else { return Ok(()) };
    let Some(book) = load_book(conn, book_id)? else { return Ok(()) };
    let author = book.authors.first().map(String::as_str).unwrap_or("Nieznany autor");
    let dir = crate::infrastructure::files::book_directory(library, author, &book.title, book_id)?;
    if !dir.exists() {
        return Ok(());
    }
    let payload = serde_json::json!({ "bookId": book_id, "title": book.title, "authors": book.authors });
    crate::infrastructure::files::write_atomic(&dir.join("metadata.json"), payload.to_string().as_bytes())?;
    Ok(())
}

pub(crate) fn load_book(conn: &Connection, id: &str) -> Result<Option<BookRecord>, String> {
    conn.query_row(
        "SELECT id, title, original_title, subtitle, authors, translators, publisher, publication_year, language, description, cover_path, series_name, series_number, genres, tags, average_rating, rating_source, field_sources, manually_edited_fields, created_at, updated_at, deleted_at, purged_at FROM books WHERE id=?1",
        params![id],
        map_book,
    )
    .optional()
    .map_err(map_sql)
}

pub(crate) fn map_book(row: &rusqlite::Row<'_>) -> rusqlite::Result<BookRecord> {
    let title: String = row.get(1)?;
    Ok(BookRecord {
        id: row.get(0)?,
        title: title.clone(),
        original_title: row.get(2)?,
        subtitle: row.get(3)?,
        authors: parse_string_list(&row.get::<_, String>(4)?, &title),
        translators: parse_string_list(&row.get::<_, String>(5).unwrap_or_else(|_| "[]".into()), &title),
        publisher: row.get(6)?,
        publication_year: row.get(7)?,
        language: row.get(8)?,
        description: row.get(9)?,
        cover_path: row.get(10)?,
        series_name: row.get(11)?,
        series_number: row.get(12)?,
        genres: parse_string_list(&row.get::<_, String>(13).unwrap_or_else(|_| "[]".into()), &title),
        tags: parse_string_list(&row.get::<_, String>(14).unwrap_or_else(|_| "[]".into()), &title),
        average_rating: row.get(15)?,
        rating_source: row.get(16)?,
        field_sources: serde_json::from_str(&row.get::<_, String>(17).unwrap_or_else(|_| "{}".into())).unwrap_or(serde_json::json!({})),
        manually_edited_fields: parse_string_list(&row.get::<_, String>(18).unwrap_or_else(|_| "[]".into()), &title),
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
        deleted_at: row.get(21)?,
        purged_at: row.get(22)?,
    })
}

pub(crate) fn parse_string_list(raw: &str, title: &str) -> Vec<String> {
    match serde_json::from_str::<Vec<String>>(raw) {
        Ok(values) => values,
        Err(_) => vec![format!("uszkodzone dane autora przy „{title}”")],
    }
}

pub(crate) fn map_card(row: &rusqlite::Row<'_>) -> rusqlite::Result<BookCard> {
    let title: String = row.get(1)?;
    let authors_raw: String = row.get(2)?;
    let tags_raw: String = row.get(7)?;
    Ok(BookCard {
        id: row.get(0)?,
        title: title.clone(),
        authors: parse_string_list(&authors_raw, &title),
        publication_year: row.get(3)?,
        cover_path: row.get(4)?,
        series_name: row.get(5)?,
        series_number: row.get(6)?,
        tags: parse_string_list(&tags_raw, &title),
        created_at: row.get(8)?,
        average_rating: row.get(9)?,
        has_paper: row.get::<_, i64>(10)? == 1,
        has_digital: row.get::<_, i64>(11)? == 1,
        loaned_to: row.get(12)?,
        finished: row.get::<_, i64>(13)? == 1,
        loan_due: row.get(14)?,
        cover_file: None,
        updated_at: row.get(15)?,
    })
}

pub(crate) fn load_editions(conn: &Connection, book_id: &str) -> Result<Vec<EditionRecord>, String> {
    let mut stmt = conn.prepare("SELECT id, book_id, isbn10, isbn13, edition_name, publisher, publication_date, page_count, cover_path, format, external_ids, metadata_source, created_at, updated_at FROM book_editions WHERE book_id=?1").map_err(map_sql)?;
    let rows = stmt.query_map(params![book_id], |row| {
        Ok(EditionRecord {
            id: row.get(0)?, book_id: row.get(1)?, isbn10: row.get(2)?, isbn13: row.get(3)?, edition_name: row.get(4)?, publisher: row.get(5)?, publication_date: row.get(6)?, page_count: row.get(7)?, cover_path: row.get(8)?, format: row.get(9)?,
            external_ids: serde_json::from_str(&row.get::<_, String>(10)?).unwrap_or(serde_json::json!({})),
            metadata_source: row.get(11)?, created_at: row.get(12)?, updated_at: row.get(13)?,
        })
    }).map_err(map_sql)?.collect::<Result<Vec<_>, _>>().map_err(map_sql);
    rows
}

fn load_physical(conn: &Connection, book_id: &str) -> Result<Vec<PhysicalRecord>, String> {
    let mut stmt = conn.prepare("SELECT p.id, p.book_edition_id, p.shelf_id, p.location_note, p.acquisition_date, p.price, p.condition, p.owner, p.loaned_to, p.loaned_date, p.return_due_date, p.notes, p.barcode_value, p.created_at, p.updated_at FROM physical_copies p JOIN book_editions e ON e.id = p.book_edition_id WHERE e.book_id=?1").map_err(map_sql)?;
    let rows = stmt.query_map(params![book_id], map_physical).map_err(map_sql)?.collect::<Result<Vec<_>, _>>().map_err(map_sql);
    rows
}

fn map_physical(row: &rusqlite::Row<'_>) -> rusqlite::Result<PhysicalRecord> {
    Ok(PhysicalRecord {
        id: row.get(0)?, book_edition_id: row.get(1)?, shelf_id: row.get(2)?, location_note: row.get(3)?, acquisition_date: row.get(4)?, price: row.get(5)?, condition: row.get(6)?, owner: row.get(7)?, loaned_to: row.get(8)?, loaned_date: row.get(9)?, return_due_date: row.get(10)?, notes: row.get(11)?, barcode_value: row.get(12)?, created_at: row.get(13)?, updated_at: row.get(14)?,
    })
}

pub(crate) fn load_digital(conn: &Connection, book_id: &str) -> Result<Vec<DigitalRecord>, String> {
    let mut stmt = conn.prepare("SELECT d.id, d.book_edition_id, d.file_path, d.file_name, d.file_size, d.file_hash_sha256, d.file_format, d.drm_status, d.added_at, d.last_verified_at, d.notes FROM digital_copies d JOIN book_editions e ON e.id = d.book_edition_id WHERE e.book_id=?1").map_err(map_sql)?;
    let rows = stmt.query_map(params![book_id], |row| {
        Ok(DigitalRecord { id: row.get(0)?, book_edition_id: row.get(1)?, file_path: row.get(2)?, file_name: row.get(3)?, file_size: row.get(4)?, file_hash_sha256: row.get(5)?, file_format: row.get(6)?, drm_status: row.get(7)?, added_at: row.get(8)?, last_verified_at: row.get(9)?, notes: row.get(10)?, status: None })
    }).map_err(map_sql)?.collect::<Result<Vec<_>, _>>().map_err(map_sql);
    rows
}

fn load_progress(conn: &Connection, book_id: &str) -> Result<Vec<ProgressRecord>, String> {
    let mut stmt = conn.prepare("SELECT id, book_id, format, progress_percent, current_page, current_chapter, started_at, finished_at, rating, review, notes FROM reading_progress WHERE book_id=?1").map_err(map_sql)?;
    let rows = stmt.query_map(params![book_id], map_progress).map_err(map_sql)?.collect::<Result<Vec<_>, _>>().map_err(map_sql);
    rows
}

fn map_progress(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProgressRecord> {
    Ok(ProgressRecord { id: row.get(0)?, book_id: row.get(1)?, format: row.get(2)?, progress_percent: row.get(3)?, current_page: row.get(4)?, current_chapter: row.get(5)?, started_at: row.get(6)?, finished_at: row.get(7)?, rating: row.get(8)?, review: row.get(9)?, notes: row.get(10)? })
}

fn load_book_shelves(conn: &Connection, book_id: &str) -> Result<Vec<ShelfRecord>, String> {
    let mut stmt = conn.prepare("SELECT s.id, s.name, s.description, s.color, s.icon, s.sort_order, s.is_smart_shelf, s.smart_filter_json, s.created_at, s.updated_at, 0, s.deleted_at FROM shelves s JOIN shelf_items si ON si.shelf_id = s.id WHERE si.book_id=?1 AND s.deleted_at IS NULL").map_err(map_sql)?;
    let rows = stmt.query_map(params![book_id], map_shelf).map_err(map_sql)?.collect::<Result<Vec<_>, _>>().map_err(map_sql);
    rows
}

fn map_shelf(row: &rusqlite::Row<'_>) -> rusqlite::Result<ShelfRecord> {
    Ok(ShelfRecord {
        id: row.get(0)?, name: row.get(1)?, description: row.get(2)?, color: row.get(3)?, icon: row.get(4)?, sort_order: row.get(5)?, is_smart_shelf: row.get::<_, i64>(6)? == 1, smart_filter_json: row.get(7)?, created_at: row.get(8)?, updated_at: row.get(9)?, book_count: row.get(10)?, deleted_at: row.get(11)?,
    })
}

fn all_shelves(conn: &Connection) -> Result<Vec<ShelfRecord>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name, description, color, icon, sort_order, is_smart_shelf, smart_filter_json, created_at, updated_at, 0, deleted_at FROM shelves")
        .map_err(map_sql)?;
    let rows = stmt.query_map([], map_shelf).map_err(map_sql)?.collect::<Result<Vec<_>, _>>().map_err(map_sql);
    rows
}

fn all_books(conn: &Connection) -> Result<Vec<BookRecord>, String> {
    let mut stmt = conn.prepare("SELECT id, title, original_title, subtitle, authors, translators, publisher, publication_year, language, description, cover_path, series_name, series_number, genres, tags, average_rating, rating_source, field_sources, manually_edited_fields, created_at, updated_at, deleted_at, purged_at FROM books").map_err(map_sql)?;
    let rows = stmt.query_map([], map_book).map_err(map_sql)?.collect::<Result<Vec<_>, _>>().map_err(map_sql);
    rows
}

fn all_editions(conn: &Connection) -> Result<Vec<EditionRecord>, String> {
    let mut stmt = conn.prepare("SELECT id, book_id, isbn10, isbn13, edition_name, publisher, publication_date, page_count, cover_path, format, external_ids, metadata_source, created_at, updated_at FROM book_editions").map_err(map_sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(EditionRecord { id: row.get(0)?, book_id: row.get(1)?, isbn10: row.get(2)?, isbn13: row.get(3)?, edition_name: row.get(4)?, publisher: row.get(5)?, publication_date: row.get(6)?, page_count: row.get(7)?, cover_path: row.get(8)?, format: row.get(9)?, external_ids: serde_json::from_str(&row.get::<_, String>(10)?).unwrap_or(serde_json::json!({})), metadata_source: row.get(11)?, created_at: row.get(12)?, updated_at: row.get(13)? })
    }).map_err(map_sql)?.collect::<Result<Vec<_>, _>>().map_err(map_sql);
    rows
}

fn all_physical(conn: &Connection) -> Result<Vec<PhysicalRecord>, String> {
    let mut stmt = conn.prepare("SELECT id, book_edition_id, shelf_id, location_note, acquisition_date, price, condition, owner, loaned_to, loaned_date, return_due_date, notes, barcode_value, created_at, updated_at FROM physical_copies").map_err(map_sql)?;
    let rows = stmt.query_map([], map_physical).map_err(map_sql)?.collect::<Result<Vec<_>, _>>().map_err(map_sql);
    rows
}

fn all_digital(conn: &Connection) -> Result<Vec<DigitalRecord>, String> {
    let mut stmt = conn.prepare("SELECT id, book_edition_id, file_path, file_name, file_size, file_hash_sha256, file_format, drm_status, added_at, last_verified_at, notes FROM digital_copies").map_err(map_sql)?;
    let rows = stmt.query_map([], |row| Ok(DigitalRecord { id: row.get(0)?, book_edition_id: row.get(1)?, file_path: row.get(2)?, file_name: row.get(3)?, file_size: row.get(4)?, file_hash_sha256: row.get(5)?, file_format: row.get(6)?, drm_status: row.get(7)?, added_at: row.get(8)?, last_verified_at: row.get(9)?, notes: row.get(10)?, status: None })).map_err(map_sql)?.collect::<Result<Vec<_>, _>>().map_err(map_sql);
    rows
}

fn all_shelf_items(conn: &Connection) -> Result<Vec<ShelfItemRecord>, String> {
    let mut stmt = conn.prepare("SELECT id, shelf_id, book_id, book_edition_id, position, added_at FROM shelf_items").map_err(map_sql)?;
    let rows = stmt.query_map([], |row| Ok(ShelfItemRecord { id: row.get(0)?, shelf_id: row.get(1)?, book_id: row.get(2)?, book_edition_id: row.get(3)?, position: row.get(4)?, added_at: row.get(5)? })).map_err(map_sql)?.collect::<Result<Vec<_>, _>>().map_err(map_sql);
    rows
}

fn all_progress(conn: &Connection) -> Result<Vec<ProgressRecord>, String> {
    let mut stmt = conn.prepare("SELECT id, book_id, format, progress_percent, current_page, current_chapter, started_at, finished_at, rating, review, notes FROM reading_progress").map_err(map_sql)?;
    let rows = stmt.query_map([], map_progress).map_err(map_sql)?.collect::<Result<Vec<_>, _>>().map_err(map_sql);
    rows
}

fn insert_book_row(conn: &Connection, book: &BookRecord) -> Result<(), String> {
    conn.execute(
        "INSERT INTO books (id, title, original_title, subtitle, authors, translators, publisher, publication_year, language, description, cover_path, series_name, series_number, genres, tags, average_rating, rating_source, field_sources, manually_edited_fields, dedupe_key, search_text, created_at, updated_at, deleted_at, purged_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)",
        params![book.id, book.title, book.original_title, book.subtitle, serde_json::to_string(&book.authors).unwrap(), serde_json::to_string(&book.translators).unwrap(), book.publisher, book.publication_year, book.language, book.description, foreign_cover(book.cover_path.as_deref()), book.series_name, book.series_number, serde_json::to_string(&book.genres).unwrap(), serde_json::to_string(&book.tags).unwrap(), book.average_rating, book.rating_source, book.field_sources.to_string(), serde_json::to_string(&book.manually_edited_fields).unwrap(), book.deleted_at.is_none().then(|| dedupe_key(&book.title, &book.authors)), fold(&book.title), book.created_at, book.updated_at, book.deleted_at, book.purged_at],
    ).map_err(map_sql)?;
    Ok(())
}

/// Okładka z obcego snapshotu: tylko ścieżka względna wewnątrz biblioteki.
fn foreign_cover(stored: Option<&str>) -> Option<String> {
    stored.and_then(safe_relative).map(|path| path.to_string_lossy().replace('\\', "/"))
}

fn update_book_row(conn: &Connection, book: &BookRecord) -> Result<(), String> {
    conn.execute("DELETE FROM books WHERE id=?1", params![book.id]).map_err(map_sql)?;
    insert_book_row(conn, book)
}

fn insert_edition_row(conn: &Connection, edition: &EditionRecord) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO book_editions (id, book_id, isbn10, isbn13, edition_name, publisher, publication_date, page_count, cover_path, format, external_ids, metadata_source, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![edition.id, edition.book_id, edition.isbn10, edition.isbn13, edition.edition_name, edition.publisher, edition.publication_date, edition.page_count, foreign_cover(edition.cover_path.as_deref()), edition.format, edition.external_ids.to_string(), edition.metadata_source, edition.created_at, edition.updated_at],
    ).map_err(map_sql)?;
    Ok(())
}

fn insert_physical_row(conn: &Connection, copy: &PhysicalRecord) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO physical_copies (id, book_edition_id, shelf_id, location_note, acquisition_date, price, condition, owner, loaned_to, loaned_date, return_due_date, notes, barcode_value, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![copy.id, copy.book_edition_id, copy.shelf_id, copy.location_note, copy.acquisition_date, copy.price, copy.condition, copy.owner, copy.loaned_to, copy.loaned_date, copy.return_due_date, copy.notes, copy.barcode_value, copy.created_at, copy.updated_at],
    ).map_err(map_sql)?;
    Ok(())
}

fn insert_digital_row(conn: &Connection, file: &DigitalRecord) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO digital_copies (id, book_edition_id, file_path, file_name, file_size, file_hash_sha256, file_format, drm_status, added_at, last_verified_at, notes) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![file.id, file.book_edition_id, file.file_path, file.file_name, file.file_size, file.file_hash_sha256, file.file_format, file.drm_status, file.added_at, file.last_verified_at, file.notes],
    ).map_err(map_sql)?;
    Ok(())
}

fn insert_progress_row(conn: &Connection, item: &ProgressRecord) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO reading_progress (id, book_id, format, progress_percent, current_page, current_chapter, started_at, finished_at, rating, review, notes) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![item.id, item.book_id, item.format, item.progress_percent, item.current_page, item.current_chapter, item.started_at, item.finished_at, item.rating, item.review, item.notes],
    ).map_err(map_sql)?;
    Ok(())
}

pub(crate) fn find_book_by_isbn(conn: &Connection, isbn: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT e.book_id FROM book_editions e JOIN books b ON b.id = e.book_id WHERE b.deleted_at IS NULL AND (e.isbn13=?1 OR e.isbn10=?1) LIMIT 1",
        params![isbn],
        |row| row.get(0),
    )
    .optional()
    .map_err(map_sql)
}

pub(crate) fn find_book_by_key(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT id FROM books WHERE deleted_at IS NULL AND dedupe_key=?1 LIMIT 1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(map_sql)
}

pub(crate) fn find_title_by_hash(conn: &Connection, hash: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT b.title FROM digital_copies d JOIN book_editions e ON e.id = d.book_edition_id JOIN books b ON b.id = e.book_id WHERE d.file_hash_sha256=?1 AND b.deleted_at IS NULL LIMIT 1",
        params![hash],
        |row| row.get(0),
    )
    .optional()
    .map_err(map_sql)
}

pub(crate) fn touch_book(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("UPDATE books SET updated_at=?1 WHERE id=?2", params![now(), id]).map_err(map_sql)?;
    Ok(())
}

pub(crate) fn setting(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM app_settings WHERE key=?1", params![key], |row| row.get(0)).optional().map_err(map_sql)
}

pub(crate) fn put_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute("INSERT INTO app_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, value]).map_err(map_sql)?;
    Ok(())
}

fn flag(conn: &Connection, key: &str, default: bool) -> Result<bool, String> {
    Ok(match setting(conn, key)? {
        Some(value) => value != "0",
        None => default,
    })
}

fn optional_isbn(raw: Option<&str>) -> Result<Option<String>, String> {
    match raw.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(value) => validate_isbn(value).map(Some),
    }
}

pub(crate) fn clean_opt(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn empty_to_none(value: Option<String>) -> Option<String> {
    clean_opt(value)
}

fn clean_list(values: &[String]) -> Vec<String> {
    values.iter().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect()
}

fn like_pattern(folded: &str) -> String {
    format!("%{}%", folded.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
}

fn stem(folded: &str) -> String {
    if folded.chars().count() > 5 {
        folded.chars().take(folded.chars().count() - 2).collect()
    } else {
        String::new()
    }
}

fn apply_smart_filter(filter: Option<&str>, view: &mut String, author: &mut Option<String>, tag: &mut Option<String>) {
    let Some(raw) = filter else { return };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else { return };
    if value.get("loaned").and_then(|v| v.as_bool()) == Some(true) {
        *view = "loaned".into();
    }
    if value.get("digital").and_then(|v| v.as_bool()) == Some(true) {
        *view = "ebook".into();
    }
    if value.get("unread").and_then(|v| v.as_bool()) == Some(true) {
        *view = "unread".into();
    }
    if let Some(value) = value.get("author").and_then(|v| v.as_str()) {
        *author = Some(value.to_string());
    }
    if let Some(value) = value.get("tag").and_then(|v| v.as_str()) {
        *tag = Some(value.to_string());
    }
}

/// Pełna ścieżka zapisanego pliku. Ścieżki bezwzględne są przyjmowane tylko wewnątrz katalogu
/// biblioteki albo katalogu okładek, a względne nie mogą wychodzić w górę katalogu.
pub fn resolve_path(paths: &StorePaths, stored: &str) -> Option<PathBuf> {
    let path = Path::new(stored);
    if path.is_absolute() || stored.contains(':') {
        let allowed = paths.library.as_ref().is_some_and(|root| path_within(root, path)) || path_within(&paths.covers, path);
        return allowed.then(|| path.to_path_buf());
    }
    let relative = safe_relative(stored)?;
    paths.library.as_ref().map(|root| root.join(relative))
}

fn relative_to(root: &Path, target: &Path) -> Result<String, String> {
    relative_inside(root, target).ok_or_else(|| "Plik musi leżeć w katalogu biblioteki.".to_string())
}

fn safe_file_name(name: &str) -> String {
    let cleaned = safe_component(name);
    if cleaned == "bez nazwy" {
        return "plik".into();
    }
    let replaced: String = name
        .chars()
        .map(|ch| if ['\\', '/', ':', '*', '?', '"', '<', '>', '|'].contains(&ch) || ch.is_control() { '_' } else { ch })
        .collect();
    let replaced = replaced.trim().trim_end_matches(['.', ' ']).to_string();
    // „CON.epub” też jest w Windows zarezerwowane, więc sprawdzamy rdzeń przed pierwszą kropką.
    let stem = replaced.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    if crate::domain::is_reserved_name(&stem) {
        format!("_{replaced}")
    } else {
        replaced
    }
}

#[cfg(test)]
pub mod tests_support {
    use crate::domain::BookRecord;

    pub fn record(id: &str, updated_at: &str) -> BookRecord {
        BookRecord {
            id: id.into(), title: "T".into(), original_title: None, subtitle: None, authors: vec!["A".into()],
            translators: vec![], publisher: None, publication_year: None, language: None, description: None,
            cover_path: None, series_name: None, series_number: None, genres: vec![], tags: vec![],
            average_rating: None, rating_source: None, field_sources: serde_json::json!({}),
            manually_edited_fields: vec![], created_at: "2020".into(), updated_at: updated_at.into(),
            deleted_at: None, purged_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db::Database;

    fn temp_db() -> (Database, TempfileDir) {
        let dir = std::env::temp_dir().join(format!("bib-store-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&dir.join("t.sqlite")).unwrap();
        (db, TempfileDir(dir))
    }

    struct TempfileDir(std::path::PathBuf);
    impl Drop for TempfileDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn duplicate_title_and_author_returns_existing() {
        let (db, _dir) = temp_db();
        let paths = StorePaths { library: None, covers: std::env::temp_dir() };
        let draft = sample("Pan Tadeusz", vec!["Mickiewicz, Adam".into()]);
        let first = db.with(|conn| save_book(conn, draft.clone_draft(), &paths, None)).unwrap();
        let second = db.with(|conn| save_book(conn, draft, &paths, None)).unwrap();
        assert!(first.created);
        assert!(!second.created);
        assert_eq!(first.book_id, second.book_id);
        let cards = db.with(|conn| list_books(conn, &ListQuery { q: "zolc".into(), view: String::new(), shelf_id: None, author: None, series: None, tag: None, year: None, sort: "title".into() }, &paths)).unwrap();
        assert!(cards.is_empty());
        let found = db.with(|conn| list_books(conn, &ListQuery { q: "tadeusz".into(), view: String::new(), shelf_id: None, author: None, series: None, tag: None, year: None, sort: "title".into() }, &paths)).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].authors, vec!["Mickiewicz, Adam".to_string()]);
    }

    #[test]
    fn files_are_stored_relative_to_the_library() {
        let (db, dir) = temp_db();
        let library = dir.0.join("Biblioteka");
        fs::create_dir_all(&library).unwrap();
        let paths = StorePaths { library: Some(library.clone()), covers: dir.0.join("covers") };
        let source = dir.0.join("Lalka.epub");
        fs::write(&source, b"epub").unwrap();
        let book = db.with(|conn| save_book(conn, sample("Lalka", vec!["Prus".into()]), &paths, None)).unwrap().book_id;
        db.with(|conn| register_digital(conn, &book, &source, &paths)).unwrap();
        let details = db.with(|conn| get_book(conn, &book)).unwrap();
        let stored = &details.digital[0].file_path;
        assert!(!Path::new(stored).is_absolute(), "ścieżka pliku musi być względna, jest: {stored}");
        assert!(resolve_path(&paths, stored).unwrap().is_file());
        // Ten sam plik po usunięciu książki można dodać do innej.
        db.with(|conn| delete_book(conn, &book)).unwrap();
        let other = db.with(|conn| save_book(conn, sample("Inna", vec!["Prus".into()]), &paths, None)).unwrap().book_id;
        db.with(|conn| register_digital(conn, &other, &source, &paths)).unwrap();
    }

    #[test]
    fn resolve_path_rejects_paths_outside_the_library() {
        let paths = StorePaths { library: Some(PathBuf::from(r"C:\Biblioteka")), covers: PathBuf::from(r"C:\Dane\covers") };
        assert!(resolve_path(&paths, r"C:\Windows\notepad.exe").is_none());
        assert!(resolve_path(&paths, "../../Windows/notepad.exe").is_none());
        assert!(resolve_path(&paths, r"C:\Biblioteka\A\b.epub").is_some());
        assert!(resolve_path(&paths, r"C:\Dane\covers\x.jpg").is_some());
        assert_eq!(resolve_path(&paths, "A/b.epub"), Some(PathBuf::from(r"C:\Biblioteka").join("A/b.epub")));
    }

    #[test]
    fn editing_into_another_book_is_an_error_not_a_silent_switch() {
        let (db, _dir) = temp_db();
        let paths = StorePaths { library: None, covers: std::env::temp_dir() };
        let first = db.with(|conn| save_book(conn, sample("Pan Tadeusz", vec!["Mickiewicz".into()]), &paths, None)).unwrap().book_id;
        let second = db.with(|conn| save_book(conn, sample("Dziady", vec!["Mickiewicz".into()]), &paths, None)).unwrap().book_id;
        let mut edit = sample("Pan Tadeusz", vec!["Mickiewicz".into()]);
        edit.id = Some(second.clone());
        let result = db.with(|conn| save_book(conn, edit, &paths, None));
        assert!(result.is_err());
        assert_ne!(first, second);
        // Zmiana tylko opisu oznacza jako ręczne tylko opis.
        let mut edit = sample("Dziady", vec!["Mickiewicz".into()]);
        edit.id = Some(second.clone());
        edit.description = Some("Dramat".into());
        db.with(|conn| save_book(conn, edit, &paths, None)).unwrap();
        let book = db.with(|conn| get_book(conn, &second)).unwrap().book;
        assert_eq!(book.manually_edited_fields, vec!["description".to_string()]);
    }

    #[test]
    fn normalize_paths_rewrites_old_absolute_paths() {
        let (db, dir) = temp_db();
        let library = dir.0.join("Lib");
        fs::create_dir_all(&library).unwrap();
        db.with(|conn| {
            save_settings(conn, SettingsPatch {
                library_dir: Some(library.display().to_string()), theme: None, open_library: None, google_books: None, national_library: None, lubimy_czytac: None, upolujebooka: None,
                locale: None, auto_backup: None, backup_dir: None, sync_provider: None, sync_dir: None, webdav_url: None, webdav_user: None, reading_goal: None,
            })?;
            let paths = StorePaths { library: Some(library.clone()), covers: dir.0.clone() };
            let id = save_book(conn, sample("Stara", vec!["A".into()]), &paths, None)?.book_id;
            let absolute = library.join("A").join("Stara").join("cover.jpg").display().to_string();
            conn.execute("UPDATE books SET cover_path=?1 WHERE id=?2", params![absolute, id]).unwrap();
            normalize_paths(conn)?;
            let stored: String = conn.query_row("SELECT cover_path FROM books WHERE id=?1", params![id], |r| r.get(0)).unwrap();
            assert_eq!(stored, "A/Stara/cover.jpg");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn search_finds_inflected_words_and_shelf_counts_skip_deleted_books() {
        let (db, _dir) = temp_db();
        let paths = StorePaths { library: None, covers: std::env::temp_dir() };
        let id = db.with(|conn| save_book(conn, sample("Pan Tadeusz", vec!["Mickiewicz, Adam".into()]), &paths, None)).unwrap().book_id;
        let query = |q: &str| ListQuery { q: q.into(), view: String::new(), shelf_id: None, author: None, series: None, tag: None, year: None, sort: "relevance".into() };
        assert_eq!(db.with(|conn| list_books(conn, &query("tadeusza"), &paths)).unwrap().len(), 1);
        assert_eq!(db.with(|conn| list_books(conn, &query("mick tad"), &paths)).unwrap().len(), 1);
        let shelf = db.with(|conn| save_shelf(conn, crate::domain::ShelfDraft { id: None, name: "Moja".into(), description: None, color: None, is_smart_shelf: false, smart_filter_json: None })).unwrap();
        db.with(|conn| add_to_shelf(conn, &shelf, &id)).unwrap();
        let count = |db: &Database| db.with(list_shelves).unwrap().into_iter().find(|s| s.id == shelf).unwrap().book_count;
        assert_eq!(count(&db), 1);
        db.with(|conn| delete_book(conn, &id)).unwrap();
        assert_eq!(count(&db), 0);
        assert!(db.with(|conn| list_books(conn, &query("tadeusz"), &paths)).unwrap().is_empty());
    }

    impl BookDraft {
        fn clone_draft(&self) -> Self {
            BookDraft {
                id: self.id.clone(),
                title: self.title.clone(),
                original_title: self.original_title.clone(),
                subtitle: self.subtitle.clone(),
                authors: self.authors.clone(),
                translators: self.translators.clone(),
                publisher: self.publisher.clone(),
                publication_year: self.publication_year,
                language: self.language.clone(),
                description: self.description.clone(),
                series_name: self.series_name.clone(),
                series_number: self.series_number.clone(),
                genres: self.genres.clone(),
                tags: self.tags.clone(),
                isbn10: self.isbn10.clone(),
                isbn13: self.isbn13.clone(),
                page_count: self.page_count,
                cover_url: self.cover_url.clone(),
                metadata_source: self.metadata_source.clone(),
                external_id: self.external_id.clone(),
                edition_id: self.edition_id.clone(),
                average_rating: self.average_rating,
                rating_source: self.rating_source.clone(),
            }
        }
    }

    fn sample(title: &str, authors: Vec<String>) -> BookDraft {
        BookDraft {
            id: None, title: title.into(), original_title: None, subtitle: None, authors, translators: vec![],
            publisher: None, publication_year: Some(1834), language: Some("pol".into()), description: None,
            series_name: None, series_number: None, genres: vec![], tags: vec!["epopeja".into()],
            isbn10: None, isbn13: None, page_count: None, cover_url: None, metadata_source: Some("manual".into()),
            external_id: None, edition_id: None, average_rating: None, rating_source: None,
        }
    }
}
