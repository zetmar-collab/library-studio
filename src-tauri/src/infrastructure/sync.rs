use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{params, Connection, OptionalExtension};

use crate::domain::{BookRecord, Settings, ShelfRecord, Snapshot, SyncReport};
use crate::infrastructure::db::{map_sql, now};
use crate::infrastructure::files::{copy_tree, safe_relative, write_atomic, CANCELLED};
use crate::infrastructure::store::{self, StorePaths};

pub fn sync_folder(
    conn: &Connection,
    folder: &Path,
    base_path: &Path,
    paths: &StorePaths,
    progress: &mut dyn FnMut(u64, u64, &str),
    cancel: &AtomicBool,
) -> Result<SyncReport, String> {
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let local = store::snapshot(conn)?;
    let remote = read_remote(&folder.join("library.json"))?;
    let base = read_snapshot(base_path).unwrap_or_else(|_| empty_snapshot());
    let mut merged = remote.clone();
    let mut pushed = 0;
    let mut pulled = 0;
    let mut conflicts = 0;
    let mut skipped = 0;
    // Książki, dla których baza synchronizacji ma zostać po staremu (konflikt albo pominięcie).
    let mut unsettled: HashSet<String> = HashSet::new();
    let ids = union_ids(&local, &remote);
    let total = ids.len() as u64;
    let local_by_id: HashMap<&str, &BookRecord> = local.books.iter().map(|b| (b.id.as_str(), b)).collect();
    let remote_by_id: HashMap<&str, &BookRecord> = remote.books.iter().map(|b| (b.id.as_str(), b)).collect();
    let base_by_id: HashMap<&str, &BookRecord> = base.books.iter().map(|b| (b.id.as_str(), b)).collect();
    for (index, id) in ids.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let local_book = local_by_id.get(id.as_str()).copied();
        let remote_book = remote_by_id.get(id.as_str()).copied();
        let base_at = base_by_id.get(id.as_str()).map(|book| book.updated_at.as_str());
        match decide(local_book, remote_book, base_at) {
            Decision::Push => {
                if let Some(book) = local_book {
                    replace_book(&mut merged, book, &local);
                    pushed += 1;
                }
            }
            Decision::Pull => {
                let report = store::import_snapshot_with(conn, &snapshot_for(&remote, id), true)?;
                if report.skipped > 0 {
                    skipped += report.skipped;
                    unsettled.insert(id.clone());
                } else {
                    pulled += 1;
                }
            }
            Decision::Conflict => {
                conflicts += 1;
                unsettled.insert(id.clone());
                let summary = local_book.or(remote_book).map(|book| book.title.clone()).unwrap_or_else(|| id.clone());
                if !has_open_conflict(conn, id)? {
                    store::add_conflict(
                        conn,
                        Some(id),
                        &format!("Zmieniono „{summary}” w dwóch miejscach."),
                        local_book.map(|book| book.updated_at.as_str()),
                        remote_book.map(|book| book.updated_at.as_str()),
                    )?;
                }
            }
            Decision::Same => {}
        }
        progress(index as u64 + 1, total.max(1), id);
    }
    // Półki: nowsza wersja wygrywa w obie strony, łącznie z usunięciem.
    store::import_snapshot(conn, &Snapshot { shelves: remote.shelves.clone(), ..empty_snapshot() })?;
    merged.shelves = newest_shelves(&local.shelves, &remote.shelves);
    // Zdalny plik dostaje wszystko poza konfliktami, więc zmiany innych książek nie czekają na rozstrzygnięcie.
    write_snapshot(&folder.join("library.json"), &merged)?;
    let mut next_base = merged.clone();
    for id in &unsettled {
        next_base.books.retain(|book| &book.id != id);
        if let Some(old) = base_by_id.get(id.as_str()) {
            next_base.books.push((*old).clone());
        }
    }
    write_snapshot(base_path, &next_base)?;
    if let Some(library) = &paths.library {
        let remote_files = folder.join("files");
        copy_tree(&remote_files, library, &mut |_, _, _| {}, cancel)?;
        copy_tree(library, &remote_files, &mut |_, _, _| {}, cancel)?;
    }
    let message = if conflicts > 0 {
        "Są konflikty do rozstrzygnięcia.".to_string()
    } else if skipped > 0 {
        format!("Synchronizacja zakończona. Pominięto {skipped}, bo te książki są już w katalogu pod innym wpisem.")
    } else {
        "Synchronizacja zakończona.".to_string()
    };
    Ok(SyncReport { pushed, pulled, conflicts, skipped, message })
}

fn has_open_conflict(conn: &Connection, book_id: &str) -> Result<bool, String> {
    let found: Option<String> = conn
        .query_row("SELECT id FROM sync_conflicts WHERE book_id=?1 AND resolved_at IS NULL LIMIT 1", params![book_id], |row| row.get(0))
        .optional()
        .map_err(map_sql)?;
    Ok(found.is_some())
}

/// Rozstrzyga konflikt jednej książki. Zmienia w zdalnym pliku i w bazie synchronizacji tylko tę książkę,
/// żeby nie nadpisać zdalnych wersji innych książek, które też czekają na decyzję.
pub fn resolve(conn: &Connection, folder: &Path, base_path: &Path, conflict_id: &str, keep_local: bool) -> Result<(), String> {
    let conflicts = store::list_conflicts(conn)?;
    let conflict = conflicts.iter().find(|item| item.id == conflict_id).ok_or_else(|| "Nie ma takiego konfliktu.".to_string())?;
    let Some(book_id) = conflict.book_id.clone() else {
        store::resolve_conflict(conn, conflict_id)?;
        return Ok(());
    };
    let mut remote = read_remote(&folder.join("library.json"))?;
    let chosen = if keep_local {
        // Nowa data sprawia, że przy następnej synchronizacji lokalna wersja wygra po obu stronach.
        conn.execute("UPDATE books SET updated_at=?1 WHERE id=?2", params![now(), book_id]).map_err(map_sql)?;
        let local = store::snapshot(conn)?;
        let book = local.books.iter().find(|book| book.id == book_id).cloned().ok_or_else(|| "Nie ma takiej książki.".to_string())?;
        replace_book(&mut remote, &book, &local);
        snapshot_for(&local, &book_id)
    } else {
        let only = snapshot_for(&remote, &book_id);
        if only.books.is_empty() {
            return Err("W zdalnym pliku nie ma już tej książki.".into());
        }
        let report = store::import_snapshot_with(conn, &only, true)?;
        if report.skipped > 0 {
            return Err("Nie można przyjąć zdalnej wersji: w katalogu jest już inna książka o tym tytule i autorze albo ISBN.".into());
        }
        only
    };
    for item in conflicts.iter().filter(|item| item.book_id.as_deref() == Some(book_id.as_str())) {
        store::resolve_conflict(conn, &item.id)?;
    }
    write_snapshot(&folder.join("library.json"), &remote)?;
    let mut base = read_snapshot(base_path).unwrap_or_else(|_| empty_snapshot());
    if let Some(book) = chosen.books.iter().find(|book| book.id == book_id) {
        replace_book(&mut base, book, &chosen);
    }
    write_snapshot(base_path, &base)
}

pub fn sync_webdav(
    conn: &Connection,
    settings: &Settings,
    password: &str,
    cache: &Path,
    paths: &StorePaths,
    base_path: &Path,
    progress: &mut dyn FnMut(u64, u64, &str),
    cancel: &AtomicBool,
) -> Result<SyncReport, String> {
    let dav = WebDav::from_settings(settings, password)?;
    fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    let cached_json = cache.join("library.json");
    match dav.get("library.json")? {
        Some(bytes) => write_atomic(&cached_json, &bytes)?,
        None => {
            let _ = fs::remove_file(&cached_json);
        }
    }
    if cached_json.exists() {
        let remote = read_remote(&cached_json)?;
        let total = remote.digital.len() as u64;
        for (index, file) in remote.digital.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
            // Ścieżka z serwera nie może wyjść poza folder podręczny.
            let Some(relative) = safe_relative(&file.file_path) else { continue };
            let target = cache.join("files").join(&relative);
            if fs::metadata(&target).is_ok_and(|meta| meta.len() as i64 == file.file_size) {
                continue;
            }
            progress(index as u64, total, &file.file_name);
            if let Some(bytes) = dav.get(&format!("files/{}", url_path(&relative)))? {
                write_atomic(&target, &bytes)?;
            }
        }
    }
    let report = sync_folder(conn, cache, base_path, paths, progress, cancel)?;
    upload_snapshot(&dav, cache)?;
    upload_files(&dav, cache, progress, cancel)?;
    Ok(report)
}

/// Wysyła lokalny `library.json` z folderu podręcznego na serwer (np. po rozstrzygnięciu konfliktu).
pub fn webdav_upload_snapshot(settings: &Settings, password: &str, cache: &Path) -> Result<(), String> {
    let dav = WebDav::from_settings(settings, password)?;
    upload_snapshot(&dav, cache)
}

fn upload_snapshot(dav: &WebDav, cache: &Path) -> Result<(), String> {
    let bytes = fs::read(cache.join("library.json")).map_err(|e| e.to_string())?;
    dav.put("library.json", bytes)
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct UploadManifest {
    files: HashMap<String, (u64, u64)>,
}

/// Wysyła tylko pliki nowe albo zmienione od ostatniej wysyłki (rozmiar i czas modyfikacji).
fn upload_files(dav: &WebDav, cache: &Path, progress: &mut dyn FnMut(u64, u64, &str), cancel: &AtomicBool) -> Result<(), String> {
    let files_root = cache.join("files");
    if !files_root.exists() {
        return Ok(());
    }
    let manifest_path = cache.join("webdav-wyslane.json");
    let mut manifest: UploadManifest = fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let entries: Vec<_> = walkdir::WalkDir::new(&files_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && !e.file_name().to_string_lossy().ends_with(".part"))
        .collect();
    let total = entries.len() as u64;
    let mut result = Ok(());
    for (index, entry) in entries.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            result = Err(CANCELLED.to_string());
            break;
        }
        let relative = entry.path().strip_prefix(&files_root).map_err(|e| e.to_string())?;
        let key = relative.to_string_lossy().replace('\\', "/");
        let meta = entry.metadata().map_err(|e| e.to_string())?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let stamp = (meta.len(), modified);
        if manifest.files.get(&key) == Some(&stamp) {
            continue;
        }
        progress(index as u64, total, &key);
        let bytes = fs::read(entry.path()).map_err(|e| e.to_string())?;
        if let Err(error) = dav.put(&format!("files/{}", url_path(relative)), bytes) {
            result = Err(error);
            break;
        }
        manifest.files.insert(key, stamp);
    }
    // Manifest zapisujemy także po błędzie, żeby kolejna próba nie wysyłała wszystkiego od nowa.
    if let Ok(text) = serde_json::to_vec(&manifest) {
        let _ = write_atomic(&manifest_path, &text);
    }
    result
}

/// Ścieżka względna jako segmenty adresu URL, z kodowaniem znaków spoza ASCII.
fn url_path(relative: &Path) -> String {
    relative
        .components()
        .map(|part| percent_segment(&part.as_os_str().to_string_lossy()))
        .collect::<Vec<_>>()
        .join("/")
}

fn percent_segment(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Adres WebDAV musi używać https. Wyjątek: serwer w sieci lokalnej (NAS) albo na tym komputerze.
pub fn validate_webdav_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "Adres WebDAV jest nieprawidłowy.".to_string())?;
    match parsed.scheme() {
        "https" => Ok(()),
        "http" => {
            let host = parsed.host_str().unwrap_or("").trim_matches(['[', ']']).to_lowercase();
            let local_name = host == "localhost" || host.ends_with(".local") || host.ends_with(".lan") || host.ends_with(".home.arpa");
            let local_ip = host.parse::<std::net::IpAddr>().is_ok_and(|ip| match ip {
                std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
                std::net::IpAddr::V6(ip) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
            });
            if local_name || local_ip {
                Ok(())
            } else {
                Err("Adres WebDAV musi zaczynać się od https://. Przez http hasło szłoby otwartym tekstem.".into())
            }
        }
        _ => Err("Adres WebDAV musi zaczynać się od https://.".into()),
    }
}

struct WebDav {
    base: String,
    user: String,
    password: String,
    client: reqwest::blocking::Client,
    collections: std::cell::RefCell<HashSet<String>>,
}

impl WebDav {
    fn from_settings(settings: &Settings, password: &str) -> Result<Self, String> {
        let base = settings.webdav_url.clone().filter(|url| !url.trim().is_empty()).ok_or_else(|| "Podaj adres WebDAV.".to_string())?;
        validate_webdav_url(&base)?;
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            base,
            user: settings.webdav_user.clone().unwrap_or_default(),
            password: password.to_string(),
            client,
            collections: Default::default(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base.trim_end_matches('/'), path.trim_start_matches('/'))
    }

    fn get(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        let response = self
            .client
            .get(self.url(path))
            .basic_auth(&self.user, Some(&self.password))
            .send()
            .map_err(|e| e.to_string())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(format!("WebDAV GET {}: {}", path, response.status()));
        }
        Ok(Some(response.bytes().map_err(|e| e.to_string())?.to_vec()))
    }

    fn put(&self, path: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.ensure_collections(path)?;
        let response = self
            .client
            .put(self.url(path))
            .basic_auth(&self.user, Some(&self.password))
            .body(bytes)
            .send()
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("WebDAV PUT {}: {}", path, response.status()));
        }
        Ok(())
    }

    /// Tworzy brakujące katalogi. Każdy katalog tylko raz na synchronizację.
    fn ensure_collections(&self, path: &str) -> Result<(), String> {
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let mut acc = String::new();
        for part in parts.iter().take(parts.len().saturating_sub(1)) {
            acc = if acc.is_empty() { (*part).to_string() } else { format!("{acc}/{part}") };
            if !self.collections.borrow_mut().insert(acc.clone()) {
                continue;
            }
            let method = reqwest::Method::from_bytes(b"MKCOL").map_err(|e| e.to_string())?;
            // 405 znaczy, że katalog już jest, więc odpowiedź nie jest tu błędem.
            let _ = self.client.request(method, self.url(&acc)).basic_auth(&self.user, Some(&self.password)).send();
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Decision {
    Same,
    Push,
    Pull,
    Conflict,
}

fn decide(local: Option<&BookRecord>, remote: Option<&BookRecord>, base: Option<&str>) -> Decision {
    match (local, remote) {
        (None, None) => Decision::Same,
        (Some(_), None) => Decision::Push,
        (None, Some(_)) => Decision::Pull,
        (Some(local), Some(remote)) if local.updated_at == remote.updated_at => Decision::Same,
        (Some(local), Some(remote)) => {
            let local_changed = base != Some(local.updated_at.as_str());
            let remote_changed = base != Some(remote.updated_at.as_str());
            if local_changed && remote_changed {
                Decision::Conflict
            } else if local_changed {
                Decision::Push
            } else if remote_changed {
                Decision::Pull
            } else {
                Decision::Conflict
            }
        }
    }
}

fn union_ids(local: &Snapshot, remote: &Snapshot) -> Vec<String> {
    let mut seen = HashSet::new();
    local
        .books
        .iter()
        .chain(remote.books.iter())
        .filter(|book| seen.insert(book.id.clone()))
        .map(|book| book.id.clone())
        .collect()
}

fn newest_shelves(local: &[ShelfRecord], remote: &[ShelfRecord]) -> Vec<ShelfRecord> {
    let mut by_id: HashMap<String, ShelfRecord> = HashMap::new();
    for shelf in remote.iter().chain(local.iter()) {
        match by_id.get(&shelf.id) {
            Some(existing) if existing.updated_at >= shelf.updated_at => {}
            _ => {
                by_id.insert(shelf.id.clone(), shelf.clone());
            }
        }
    }
    let mut shelves: Vec<ShelfRecord> = by_id.into_values().collect();
    shelves.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.name.cmp(&b.name)));
    shelves
}

fn replace_book(target: &mut Snapshot, book: &BookRecord, source: &Snapshot) {
    target.books.retain(|item| item.id != book.id);
    target.books.push(book.clone());
    let old_editions: HashSet<String> = target.editions.iter().filter(|e| e.book_id == book.id).map(|e| e.id.clone()).collect();
    let new_editions: HashSet<String> = source.editions.iter().filter(|e| e.book_id == book.id).map(|e| e.id.clone()).collect();
    let editions: HashSet<&String> = old_editions.iter().chain(new_editions.iter()).collect();
    target.editions.retain(|e| e.book_id != book.id);
    target.editions.extend(source.editions.iter().filter(|e| e.book_id == book.id).cloned());
    target.physical.retain(|p| !editions.contains(&p.book_edition_id));
    target.physical.extend(source.physical.iter().filter(|p| new_editions.contains(&p.book_edition_id)).cloned());
    target.digital.retain(|d| !editions.contains(&d.book_edition_id));
    target.digital.extend(source.digital.iter().filter(|d| new_editions.contains(&d.book_edition_id)).cloned());
    target.progress.retain(|p| p.book_id != book.id);
    target.progress.extend(source.progress.iter().filter(|p| p.book_id == book.id).cloned());
    target.shelf_items.retain(|s| s.book_id.as_deref() != Some(book.id.as_str()));
    target.shelf_items.extend(source.shelf_items.iter().filter(|s| s.book_id.as_deref() == Some(book.id.as_str())).cloned());
}

fn snapshot_for(source: &Snapshot, book_id: &str) -> Snapshot {
    let mut snap = empty_snapshot();
    if let Some(book) = source.books.iter().find(|book| book.id == book_id) {
        replace_book(&mut snap, book, source);
    }
    snap.shelves = source.shelves.clone();
    snap
}

fn empty_snapshot() -> Snapshot {
    Snapshot { version: 1, books: vec![], editions: vec![], shelves: vec![], physical: vec![], digital: vec![], shelf_items: vec![], progress: vec![] }
}

fn read_snapshot(path: &Path) -> Result<Snapshot, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Brak pliku to pusta biblioteka. Uszkodzony plik to błąd: inaczej nadpisalibyśmy zdalne dane.
fn read_remote(path: &Path) -> Result<Snapshot, String> {
    if !path.exists() {
        return Ok(empty_snapshot());
    }
    read_snapshot(path).map_err(|e| format!("Nie można odczytać library.json z folderu synchronizacji: {e}"))
}

fn write_snapshot(path: &Path, snap: &Snapshot) -> Result<(), String> {
    write_atomic(path, &serde_json::to_vec_pretty(snap).map_err(|e| e.to_string())?)
}

pub fn base_path(data_dir: &Path) -> PathBuf {
    data_dir.join("sync-base.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::BookDraft;
    use crate::infrastructure::db::Database;
    use crate::infrastructure::files::NEVER;
    use crate::infrastructure::store::save_book;

    fn draft(title: &str) -> BookDraft {
        BookDraft {
            id: None, title: title.into(), original_title: None, subtitle: None, authors: vec!["Autor".into()],
            translators: vec![], publisher: None, publication_year: None, language: None, description: None,
            series_name: None, series_number: None, genres: vec![], tags: vec![], isbn10: None, isbn13: None,
            page_count: None, cover_url: None, metadata_source: None, external_id: None, edition_id: None,
            average_rating: None, rating_source: None,
        }
    }

    struct Rig {
        dir: PathBuf,
        remote: PathBuf,
        paths: StorePaths,
        a: Database,
        b: Database,
    }

    impl Rig {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("bib-sync-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            Self {
                remote: dir.join("remote"),
                paths: StorePaths { library: None, covers: dir.clone() },
                a: Database::open(&dir.join("a.sqlite")).unwrap(),
                b: Database::open(&dir.join("b.sqlite")).unwrap(),
                dir,
            }
        }
        fn sync(&self, db: &Database, name: &str) -> SyncReport {
            let base = self.dir.join(format!("base-{name}.json"));
            db.with(|conn| sync_folder(conn, &self.remote, &base, &self.paths, &mut |_, _, _| {}, &NEVER)).unwrap()
        }
        fn set(&self, db: &Database, sql: &str, args: &[&str]) {
            db.with(|conn| conn.execute(sql, rusqlite::params_from_iter(args.iter())).map(|_| ()).map_err(map_sql)).unwrap();
        }
        fn title(&self, db: &Database, id: &str) -> String {
            db.with(|conn| conn.query_row("SELECT title FROM books WHERE id=?1", [id], |r| r.get(0)).map_err(map_sql)).unwrap()
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn second_database_receives_the_book_and_can_find_it_by_author() {
        let rig = Rig::new();
        rig.a.with(|conn| save_book(conn, draft("Solaris"), &rig.paths, None).map(|_| ())).unwrap();
        rig.sync(&rig.a, "a");
        let report = rig.sync(&rig.b, "b");
        assert!(report.pulled >= 1);
        let query = crate::domain::ListQuery { q: "autor".into(), view: String::new(), shelf_id: None, author: None, series: None, tag: None, year: None, sort: "title".into() };
        let found = rig.b.with(|conn| store::list_books(conn, &query, &rig.paths)).unwrap();
        assert_eq!(found.len(), 1, "książka z synchronizacji musi być wyszukiwalna po autorze");
    }

    #[test]
    fn deletion_reaches_the_other_database() {
        let rig = Rig::new();
        let id = rig.a.with(|conn| save_book(conn, draft("Lalka"), &rig.paths, None)).unwrap().book_id;
        rig.sync(&rig.a, "a");
        rig.sync(&rig.b, "b");
        rig.a.with(|conn| store::delete_book(conn, &id)).unwrap();
        rig.set(&rig.a, "UPDATE books SET updated_at='2999-01-01T00:00:00Z' WHERE id=?1", &[&id]);
        rig.sync(&rig.a, "a");
        rig.sync(&rig.b, "b");
        assert_eq!(rig.b.with(store::book_count).unwrap(), 0);
    }

    #[test]
    fn two_conflicts_resolve_without_losing_the_other_remote_version() {
        let rig = Rig::new();
        let first = rig.a.with(|conn| save_book(conn, draft("Pierwsza"), &rig.paths, None)).unwrap().book_id;
        let second = rig.a.with(|conn| save_book(conn, draft("Druga"), &rig.paths, None)).unwrap().book_id;
        rig.sync(&rig.a, "a");
        rig.sync(&rig.b, "b");
        for (db, label, stamp) in [(&rig.a, "A", "2990-01-01T00:00:00Z"), (&rig.b, "B", "2991-01-01T00:00:00Z")] {
            for id in [&first, &second] {
                rig.set(db, "UPDATE books SET title = title || ?1, updated_at = ?2 WHERE id = ?3", &[label, stamp, id]);
            }
        }
        rig.sync(&rig.b, "b");
        let report = rig.sync(&rig.a, "a");
        assert_eq!(report.conflicts, 2);
        // Kolejna synchronizacja nie dubluje konfliktów.
        rig.sync(&rig.a, "a");
        let conflicts = rig.a.with(store::list_conflicts).unwrap();
        assert_eq!(conflicts.len(), 2);
        let base = rig.dir.join("base-a.json");
        let of = |book: &str| conflicts.iter().find(|c| c.book_id.as_deref() == Some(book)).unwrap().id.clone();
        rig.a.with(|conn| resolve(conn, &rig.remote, &base, &of(&first), true)).unwrap();
        rig.a.with(|conn| resolve(conn, &rig.remote, &base, &of(&second), false)).unwrap();
        assert_eq!(rig.title(&rig.a, &first), "PierwszaA");
        assert_eq!(rig.title(&rig.a, &second), "DrugaB", "zdalna wersja drugiej książki nie może zniknąć");
        // Po rozstrzygnięciu kolejna synchronizacja niczego nie cofa.
        let after = rig.sync(&rig.a, "a");
        assert_eq!(after.conflicts, 0);
        assert_eq!(rig.title(&rig.a, &first), "PierwszaA");
        rig.sync(&rig.b, "b");
        assert_eq!(rig.title(&rig.b, &first), "PierwszaA");
        assert_eq!(rig.title(&rig.b, &second), "DrugaB");
    }

    #[test]
    fn clashing_book_is_skipped_instead_of_failing_the_sync() {
        let rig = Rig::new();
        rig.a.with(|conn| save_book(conn, draft("Wspólna"), &rig.paths, None).map(|_| ())).unwrap();
        rig.a.with(|conn| save_book(conn, draft("Tylko A"), &rig.paths, None).map(|_| ())).unwrap();
        rig.b.with(|conn| save_book(conn, draft("Wspólna"), &rig.paths, None).map(|_| ())).unwrap();
        rig.sync(&rig.a, "a");
        let report = rig.sync(&rig.b, "b");
        assert_eq!(report.skipped, 1);
        assert_eq!(rig.b.with(store::book_count).unwrap(), 2);
    }

    #[test]
    fn deleted_shelf_stays_deleted_after_sync() {
        let rig = Rig::new();
        rig.sync(&rig.a, "a");
        rig.sync(&rig.b, "b");
        let shelf = rig.a.with(|conn| store::save_shelf(conn, crate::domain::ShelfDraft { id: None, name: "Nowa".into(), description: None, color: None, is_smart_shelf: false, smart_filter_json: None })).unwrap();
        rig.sync(&rig.a, "a");
        rig.sync(&rig.b, "b");
        assert!(rig.b.with(store::list_shelves).unwrap().iter().any(|s| s.id == shelf));
        rig.a.with(|conn| store::delete_shelf(conn, &shelf)).unwrap();
        rig.set(&rig.a, "UPDATE shelves SET updated_at='2999-01-01T00:00:00Z' WHERE id=?1", &[&shelf]);
        rig.sync(&rig.a, "a");
        rig.sync(&rig.b, "b");
        assert!(!rig.b.with(store::list_shelves).unwrap().iter().any(|s| s.id == shelf));
    }

    #[test]
    fn webdav_requires_https_outside_the_local_network() {
        assert!(validate_webdav_url("https://dav.example.com/remote.php").is_ok());
        assert!(validate_webdav_url("http://192.168.1.10:5005/dav").is_ok());
        assert!(validate_webdav_url("http://nas.local/dav").is_ok());
        assert!(validate_webdav_url("http://dav.example.com/remote.php").is_err());
        assert!(validate_webdav_url("ftp://dav.example.com").is_err());
    }

    #[test]
    fn decide_matches_three_way_rules() {
        let mut a = crate::infrastructure::store::tests_support::record("x", "2020");
        let b = crate::infrastructure::store::tests_support::record("x", "2021");
        assert_eq!(decide(Some(&a), Some(&b), Some("2020")), Decision::Pull);
        assert_eq!(decide(Some(&b), Some(&a), Some("2020")), Decision::Push);
        a.updated_at = "2022".into();
        assert_eq!(decide(Some(&a), Some(&b), Some("2020")), Decision::Conflict);
    }
}
