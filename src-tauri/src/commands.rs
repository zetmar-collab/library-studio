//! Komendy wywoływane z okna. Wszystko, co dotyka bazy, plików albo sieci, działa w osobnym wątku
//! (`spawn_blocking`): komenda bez `async` w Tauri 2 blokowałaby wątek główny okna, a wtedy nie dochodziły
//! zdarzenia postępu ani przycisk „Przerwij”.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use tauri::{AppHandle, Emitter, Manager};

use crate::domain::*;
use crate::infrastructure::backup::{self, copy_export_files, read_export, write_export};
use crate::infrastructure::catalog;
use crate::infrastructure::files::{hash_file, CANCELLED};
use crate::infrastructure::importer;
use crate::infrastructure::metadata::{self, cover_data_url, download_cover};
use crate::infrastructure::reader;
use crate::infrastructure::secrets::{self, has_secret};
use crate::infrastructure::store::{self, StorePaths};
use crate::infrastructure::sync::{self, base_path};
use crate::AppState;

type Res<T> = Result<T, String>;

/// Uruchamia pracę poza wątkiem okna.
async fn run<T: Send + 'static>(app: AppHandle, job: impl FnOnce(&AppHandle, &AppState) -> Res<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        job(&app, state.inner())
    })
    .await
    .map_err(|e| format!("Zadanie zostało przerwane: {e}"))?
}

/// Pilnuje, żeby naraz szła tylko jedna długa operacja (kopia, synchronizacja, import…).
struct JobGuard<'a>(&'a AppState);

impl<'a> JobGuard<'a> {
    fn start(state: &'a AppState) -> Res<Self> {
        if state.job_running.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return Err("Trwa inna operacja. Poczekaj albo ją przerwij.".into());
        }
        state.cancel.store(false, Ordering::Relaxed);
        Ok(Self(state))
    }
}

impl Drop for JobGuard<'_> {
    fn drop(&mut self) {
        self.0.job_running.store(false, Ordering::Release);
    }
}

pub fn paths(state: &AppState) -> Res<StorePaths> {
    let library = state.db.with(store::get_settings)?.library_dir.map(PathBuf::from);
    Ok(StorePaths { library, covers: state.data_dir.join("covers") })
}

/// Pozwala oknu wczytywać okładki prosto z dysku (protokół asset), bez przesyłania base64.
pub fn allow_assets(app: &AppHandle, state: &AppState) {
    let scope = app.asset_protocol_scope();
    let _ = fs::create_dir_all(state.data_dir.join("covers"));
    let _ = scope.allow_directory(state.data_dir.join("covers"), true);
    if let Ok(Some(library)) = state.db.with(store::get_settings).map(|s| s.library_dir) {
        let _ = scope.allow_directory(PathBuf::from(library), true);
    }
}

fn emit(app: &AppHandle, job: &str, current: u64, total: u64, label: &str, done: bool) {
    let _ = app.emit("job-progress", JobProgress { job: job.into(), current, total, label: label.into(), done });
}

fn log_error<T>(state: &AppState, what: &str, result: Res<T>) -> Res<T> {
    if let Err(error) = &result {
        if error != CANCELLED {
            crate::infrastructure::log::write(&state.data_dir, "error", &format!("{what}: {error}"));
        }
    }
    result
}

// ---------- Ustawienia ----------

#[tauri::command]
pub async fn app_status(app: AppHandle) -> Res<serde_json::Value> {
    run(app, |_, state| {
        state.db.with(|conn| {
            Ok(serde_json::json!({
                "dbPath": state.db.path().display().to_string(),
                "schemaVersion": store::schema_version(conn)?,
                "bookCount": store::book_count(conn)?,
            }))
        })
    })
    .await
}

fn read_settings(state: &AppState) -> Res<Settings> {
    let mut settings = state.db.with(store::get_settings)?;
    settings.google_books_has_key = has_secret("google-books");
    settings.webdav_has_password = has_secret("webdav-password");
    Ok(settings)
}

#[tauri::command]
pub async fn get_settings(app: AppHandle) -> Res<Settings> {
    run(app, |_, state| read_settings(state)).await
}

#[tauri::command]
pub async fn save_settings(app: AppHandle, patch: SettingsPatch) -> Res<Settings> {
    run(app, move |app, state| {
        let library_changed = patch.library_dir.is_some();
        state.db.with(|conn| store::save_settings(conn, patch))?;
        let settings = read_settings(state)?;
        let _ = fs::write(state.data_dir.join("locale.txt"), &settings.locale);
        if library_changed {
            allow_assets(app, state);
            state.db.with(store::normalize_paths)?;
        }
        Ok(settings)
    })
    .await
}

#[tauri::command]
pub async fn set_secret(app: AppHandle, account: String, value: String) -> Res<()> {
    run(app, move |_, _| {
        if account != "google-books" && account != "webdav-password" {
            return Err("Nieznany sekret.".into());
        }
        secrets::set_secret(&account, &value)
    })
    .await
}

#[tauri::command]
pub fn pick_folder() -> Option<String> {
    rfd::FileDialog::new().pick_folder().map(|path| path.display().to_string())
}

#[tauri::command]
pub fn pick_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("E-book", &["epub", "pdf", "mobi", "azw3", "azw", "fb2", "djvu", "cbz"])
        .pick_file()
        .map(|path| path.display().to_string())
}

#[tauri::command]
pub fn pick_csv_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("CSV", &["csv", "txt"])
        .pick_file()
        .map(|path| path.display().to_string())
}

#[tauri::command]
pub fn pick_backup_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("SQLite", &["sqlite", "db"])
        .pick_file()
        .map(|path| path.display().to_string())
}

// ---------- Książki ----------

#[tauri::command]
pub async fn list_books(app: AppHandle, query: ListQuery) -> Res<Vec<BookCard>> {
    run(app, move |_, state| {
        let paths = paths(state)?;
        state.db.with(|conn| store::list_books(conn, &query, &paths))
    })
    .await
}

#[tauri::command]
pub async fn get_book(app: AppHandle, id: String) -> Res<BookDetails> {
    run(app, move |_, state| state.db.with(|conn| store::get_book(conn, &id))).await
}

#[tauri::command]
pub async fn save_book(app: AppHandle, mut draft: BookDraft) -> Res<SaveResult> {
    run(app, move |_, state| {
        // Szukamy uzupełnień tylko przy nowej książce wpisanej ręcznie. Gdy wybrano wynik wyszukiwania
        // albo to import, dane już są, a trzy zapytania sieciowe niepotrzebnie wydłużały zapis.
        let manual = draft.metadata_source.as_deref().map(|s| s.trim().is_empty() || s == "manual").unwrap_or(true);
        if draft.id.is_none() && manual {
            let settings = state.db.with(store::get_settings)?;
            let key = secrets::get_secret("google-books").unwrap_or(None);
            let author = draft.authors.iter().find(|name| !name.trim().is_empty()).cloned().unwrap_or_default();
            let isbn = draft.isbn13.clone().or(draft.isbn10.clone());
            if let Some(hit) = metadata::best_hit(&settings, key.as_deref(), &draft.title, &author, isbn.as_deref()) {
                metadata::fill_empty(&mut draft, &hit, false);
            }
        }
        let cover = match draft.cover_url.as_deref() {
            Some(url) if url.starts_with("https://") => download_cover(url).ok(),
            _ => None,
        };
        let paths = paths(state)?;
        state.db.with(|conn| store::save_book(conn, draft, &paths, cover))
    })
    .await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrichReport {
    pub filled: u32,
    pub checked: u32,
    pub failed: u32,
}

fn enrich_target(state: &AppState, target: &store::EnrichTarget) -> Res<bool> {
    let settings = state.db.with(store::get_settings)?;
    let key = secrets::get_secret("google-books").unwrap_or(None);
    let Some(hit) = metadata::best_hit(&settings, key.as_deref(), &target.title, &target.author, target.isbn.as_deref()) else {
        return Ok(false);
    };
    let cover = if target.has_cover { None } else { hit.cover_url.as_deref().and_then(|url| download_cover(url).ok()) };
    let paths = paths(state)?;
    state.db.with(|conn| store::merge_hit(conn, &paths, &target.id, &hit, cover))
}

#[tauri::command]
pub async fn enrich_book(app: AppHandle, id: String) -> Res<EnrichReport> {
    run(app, move |_, state| {
        let target = state.db.with(|conn| {
            let book = store::get_book(conn, &id)?;
            let author = book.book.authors.first().cloned().unwrap_or_default();
            let edition = book.editions.first();
            Ok(store::EnrichTarget {
                id: book.book.id,
                title: book.book.title,
                author,
                isbn: edition.and_then(|item| item.isbn13.clone().or(item.isbn10.clone())),
                has_cover: book.book.cover_path.as_ref().is_some_and(|path| !path.is_empty()),
            })
        })?;
        let filled = enrich_target(state, &target)?;
        Ok(EnrichReport { filled: u32::from(filled), checked: 1, failed: 0 })
    })
    .await
}

#[tauri::command]
pub async fn enrich_library(app: AppHandle) -> Res<EnrichReport> {
    run(app, |app, state| {
        let _guard = JobGuard::start(state)?;
        let targets = state.db.with(store::books_to_enrich)?;
        let total = targets.len() as u64;
        let mut report = EnrichReport { filled: 0, checked: 0, failed: 0 };
        for (index, target) in targets.iter().enumerate() {
            if state.cancel.load(Ordering::Relaxed) {
                break;
            }
            emit(app, "enrich", index as u64, total, &target.title, false);
            report.checked += 1;
            // Jedna nieudana książka nie zatrzymuje całego uzupełniania.
            match enrich_target(state, target) {
                Ok(true) => report.filled += 1,
                Ok(false) => {}
                Err(error) => {
                    report.failed += 1;
                    crate::infrastructure::log::write(&state.data_dir, "warn", &format!("uzupełnianie „{}”: {error}", target.title));
                }
            }
        }
        emit(app, "enrich", total, total, "Gotowe", true);
        Ok(report)
    })
    .await
}

#[tauri::command]
pub async fn delete_book(app: AppHandle, id: String) -> Res<()> {
    run(app, move |_, state| state.db.with(|conn| store::delete_book(conn, &id))).await
}

#[tauri::command]
pub async fn search_metadata(app: AppHandle, query: MetadataQuery) -> Res<MetadataSearch> {
    run(app, move |_, state| {
        let settings = state.db.with(store::get_settings)?;
        let key = secrets::get_secret("google-books").unwrap_or(None);
        metadata::search(&state.db, &settings, key.as_deref(), &query)
    })
    .await
}

#[tauri::command]
pub async fn search_covers(app: AppHandle, query: MetadataQuery) -> Res<Vec<CoverCandidate>> {
    run(app, move |_, state| {
        let settings = state.db.with(store::get_settings)?;
        let key = secrets::get_secret("google-books").unwrap_or(None);
        metadata::search_covers(&settings, key.as_deref(), &query)
    })
    .await
}

#[tauri::command]
pub async fn preview_cover(app: AppHandle, url: String) -> Res<String> {
    run(app, move |_, _| Ok(cover_data_url(&download_cover(&url)?))).await
}

#[tauri::command]
pub async fn attach_cover(app: AppHandle, book_id: String, url: String) -> Res<()> {
    run(app, move |_, state| {
        let bytes = download_cover(&url)?;
        let paths = paths(state)?;
        state.db.with(|conn| store::attach_cover(conn, &paths, &book_id, &bytes))
    })
    .await
}

#[tauri::command]
pub async fn save_physical(app: AppHandle, draft: PhysicalDraft) -> Res<String> {
    run(app, move |_, state| state.db.with(|conn| store::save_physical(conn, draft))).await
}

#[tauri::command]
pub async fn delete_physical(app: AppHandle, id: String) -> Res<()> {
    run(app, move |_, state| state.db.with(|conn| store::delete_physical(conn, &id))).await
}

#[tauri::command]
pub async fn save_progress(app: AppHandle, draft: ProgressDraft) -> Res<String> {
    run(app, move |_, state| state.db.with(|conn| store::save_progress(conn, draft))).await
}

// ---------- Półki ----------

#[tauri::command]
pub async fn list_shelves(app: AppHandle) -> Res<Vec<ShelfRecord>> {
    run(app, |_, state| state.db.with(store::list_shelves)).await
}

#[tauri::command]
pub async fn save_shelf(app: AppHandle, draft: ShelfDraft) -> Res<String> {
    run(app, move |_, state| state.db.with(|conn| store::save_shelf(conn, draft))).await
}

#[tauri::command]
pub async fn delete_shelf(app: AppHandle, id: String) -> Res<()> {
    run(app, move |_, state| state.db.with(|conn| store::delete_shelf(conn, &id))).await
}

#[tauri::command]
pub async fn reorder_shelf(app: AppHandle, shelf_id: String, book_ids: Vec<String>) -> Res<()> {
    run(app, move |_, state| state.db.with(|conn| store::reorder_shelf(conn, &shelf_id, &book_ids))).await
}

#[tauri::command]
pub async fn add_to_shelf(app: AppHandle, shelf_id: String, book_id: String) -> Res<()> {
    run(app, move |_, state| state.db.with(|conn| store::add_to_shelf(conn, &shelf_id, &book_id))).await
}

#[tauri::command]
pub async fn remove_from_shelf(app: AppHandle, shelf_id: String, book_id: String) -> Res<()> {
    run(app, move |_, state| state.db.with(|conn| store::remove_from_shelf(conn, &shelf_id, &book_id))).await
}

// ---------- Czytniki ----------

#[tauri::command]
pub async fn list_readers(app: AppHandle) -> Res<Vec<reader::ReaderDevice>> {
    run(app, |_, _| Ok(reader::detect_readers())).await
}

#[tauri::command]
pub async fn open_reader_folder(app: AppHandle, path: String) -> Res<reader::ReaderDevice> {
    run(app, move |_, _| reader::device_from_path(Path::new(&path))).await
}

#[tauri::command]
pub async fn list_reader_books(app: AppHandle, root: String) -> Res<Vec<reader::ReaderFile>> {
    run(app, move |_, state| state.db.with(|conn| reader::list_books(conn, Path::new(&root)))).await
}

#[tauri::command]
pub async fn send_to_reader(app: AppHandle, root: String, book_ids: Vec<String>) -> Res<reader::TransferReport> {
    run(app, move |app, state| {
        let _guard = JobGuard::start(state)?;
        let paths = paths(state)?;
        state.db.with(|conn| {
            reader::send_books(conn, &paths, Path::new(&root), &book_ids, &state.cancel, &mut |current, total, label| {
                emit(app, "reader", current, total, label, current == total);
            })
        })
    })
    .await
}

#[tauri::command]
pub async fn import_from_reader(app: AppHandle, root: String, relative_paths: Vec<String>) -> Res<reader::TransferReport> {
    run(app, move |app, state| {
        let _guard = JobGuard::start(state)?;
        let paths = paths(state)?;
        state.db.with(|conn| {
            reader::import_books(conn, &paths, Path::new(&root), &relative_paths, &state.cancel, &mut |current, total, label| {
                emit(app, "reader", current, total, label, current == total);
            })
        })
    })
    .await
}

// ---------- Pliki ----------

#[tauri::command]
pub async fn add_file(app: AppHandle, book_id: String, source_path: String) -> Res<String> {
    run(app, move |_, state| {
        let paths = paths(state)?;
        state.db.with(|conn| store::register_digital(conn, &book_id, Path::new(&source_path), &paths))
    })
    .await
}

#[tauri::command]
pub async fn list_files(app: AppHandle) -> Res<Vec<FileRow>> {
    run(app, |_, state| {
        let paths = paths(state)?;
        state.db.with(|conn| store::list_files(conn, &paths))
    })
    .await
}

#[tauri::command]
pub async fn verify_files(app: AppHandle) -> Res<Vec<FileRow>> {
    run(app, |app, state| {
        let _guard = JobGuard::start(state)?;
        let paths = paths(state)?;
        let targets = state.db.with(|conn| store::verify_targets(conn, &paths))?;
        let total = targets.len() as u64;
        let mut rows = Vec::new();
        let mut verified = Vec::new();
        // Skróty liczymy bez blokady bazy, więc reszta okna działa w tym czasie normalnie.
        for (index, (mut row, absolute, expected)) in targets.into_iter().enumerate() {
            if state.cancel.load(Ordering::Relaxed) {
                break;
            }
            emit(app, "verify", index as u64, total, &row.file_name, false);
            row.status = match absolute.filter(|path| path.is_file()) {
                None => "missing".into(),
                Some(path) => match hash_file(&path) {
                    Ok(hash) if hash == expected => {
                        verified.push(row.id.clone());
                        "present".into()
                    }
                    Ok(_) => "changed".into(),
                    Err(_) => "missing".into(),
                },
            };
            rows.push(row);
        }
        state.db.with(|conn| store::mark_verified(conn, &verified))?;
        emit(app, "verify", total, total, "Gotowe", true);
        Ok(rows)
    })
    .await
}

const OPENABLE: &[&str] = &["epub", "pdf", "mobi", "azw", "azw3", "azw4", "kfx", "fb2", "djvu", "cbz", "cbr", "txt", "rtf", "doc", "docx", "html", "htm", "prc", "zip"];

#[tauri::command]
pub async fn open_file(app: AppHandle, id: String) -> Res<()> {
    run(app, move |_, state| {
        let paths = paths(state)?;
        let path = state.db.with(|conn| store::absolute_digital_path(conn, &id, &paths))?;
        // Otwieramy tylko e-booki. Ścieżka z obcych danych nie może uruchomić programu.
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !OPENABLE.contains(&ext.as_str()) {
            return Err("Tego rodzaju pliku nie otwieram.".into());
        }
        if !path.is_file() {
            return Err("Nie ma takiego pliku.".into());
        }
        open::that_detached(&path).map_err(|e| format!("Nie można otworzyć pliku: {e}"))
    })
    .await
}

#[tauri::command]
pub async fn cover_data(app: AppHandle, book_id: String) -> Res<Option<String>> {
    run(app, move |_, state| {
        let relative: Option<String> = state.db.with(|conn| {
            conn.query_row("SELECT cover_path FROM books WHERE id=?1", [&book_id], |row| row.get(0))
                .map_err(crate::infrastructure::db::map_sql)
        })?;
        let Some(relative) = relative.filter(|v| !v.is_empty()) else { return Ok(None) };
        let paths = paths(state)?;
        let Some(path) = store::resolve_path(&paths, &relative).filter(|p| p.is_file()) else { return Ok(None) };
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        Ok(Some(cover_data_url(&bytes)))
    })
    .await
}

// ---------- Kopie, eksport, synchronizacja ----------

#[tauri::command]
pub async fn list_backups(app: AppHandle) -> Res<Vec<BackupRow>> {
    run(app, |_, state| state.db.with(store::list_backups)).await
}

#[tauri::command]
pub async fn backup_now(app: AppHandle, kind: String) -> Res<BackupRow> {
    run(app, move |app, state| {
        let _guard = JobGuard::start(state)?;
        let settings = state.db.with(store::get_settings)?;
        let result = state.db.with(|conn| {
            let mut progress = |current, total, label: &str| emit(app, "backup", current, total, label, false);
            match kind.as_str() {
                "files" => backup::backup_files(conn, &settings, &mut progress, &state.cancel),
                "full" => {
                    let row = backup::backup_database(conn, &settings, "database")?;
                    backup::backup_files(conn, &settings, &mut progress, &state.cancel)?;
                    Ok(row)
                }
                _ => backup::backup_database(conn, &settings, "database"),
            }
        });
        emit(app, "backup", 1, 1, "Gotowe", true);
        log_error(state, "kopia", result)
    })
    .await
}

#[tauri::command]
pub async fn restore_backup(app: AppHandle, path: String) -> Res<String> {
    run(app, move |_, state| {
        let _guard = JobGuard::start(state)?;
        let safety = state.db.with_mut(|conn| backup::restore_database(conn, Path::new(&path), &state.data_dir));
        let safety = log_error(state, "przywracanie", safety)?;
        crate::infrastructure::log::write(&state.data_dir, "info", &format!("przywrócono kopię {path}"));
        Ok(safety.display().to_string())
    })
    .await
}

#[tauri::command]
pub async fn export_library(app: AppHandle, folder: String) -> Res<()> {
    run(app, move |app, state| {
        let _guard = JobGuard::start(state)?;
        let paths = paths(state)?;
        let result = state.db.with(|conn| {
            let mut progress = |current, total, label: &str| emit(app, "export", current, total, label, false);
            write_export(conn, Path::new(&folder), paths.library.as_deref(), &mut progress, &state.cancel)
        });
        emit(app, "export", 1, 1, "Gotowe", true);
        log_error(state, "eksport", result)
    })
    .await
}

#[tauri::command]
pub async fn import_library(app: AppHandle, folder: String) -> Res<String> {
    run(app, move |app, state| {
        let _guard = JobGuard::start(state)?;
        let snap = read_export(Path::new(&folder))?;
        let settings = state.db.with(store::get_settings)?;
        // Pliki najpierw, żeby wpisy w bazie od razu wskazywały na istniejące pliki.
        if let Some(library) = &settings.library_dir {
            copy_export_files(Path::new(&folder), Path::new(library), &state.cancel)?;
        }
        let report = state.db.with(|conn| store::import_snapshot(conn, &snap))?;
        emit(app, "import", 1, 1, "Gotowe", true);
        Ok(format!("Dodano {}, zaktualizowano {}, pominięto {}.", report.inserted, report.updated, report.skipped))
    })
    .await
}

#[tauri::command]
pub async fn sync_now(app: AppHandle) -> Res<SyncReport> {
    run(app, |app, state| {
        let _guard = JobGuard::start(state)?;
        let settings = state.db.with(store::get_settings)?;
        let paths = paths(state)?;
        let base = base_path(&state.data_dir);
        let mut progress = |current, total, label: &str| emit(app, "sync", current, total, label, false);
        let report = if settings.sync_provider == "webdav" {
            let password = secrets::get_secret("webdav-password")?.unwrap_or_default();
            let cache = state.data_dir.join("webdav-cache");
            state.db.with(|conn| sync::sync_webdav(conn, &settings, &password, &cache, &paths, &base, &mut progress, &state.cancel))
        } else {
            match settings.sync_dir.clone().filter(|dir| !dir.trim().is_empty()) {
                None => Err("Wybierz folder synchronizacji.".to_string()),
                Some(folder) => state.db.with(|conn| sync::sync_folder(conn, Path::new(&folder), &base, &paths, &mut progress, &state.cancel)),
            }
        };
        emit(app, "sync", 1, 1, "Gotowe", true);
        log_error(state, "synchronizacja", report)
    })
    .await
}

#[tauri::command]
pub async fn list_conflicts(app: AppHandle) -> Res<Vec<ConflictRow>> {
    run(app, |_, state| state.db.with(store::list_conflicts)).await
}

#[tauri::command]
pub async fn resolve_conflict(app: AppHandle, id: String, keep_local: bool) -> Res<()> {
    run(app, move |_, state| {
        let _guard = JobGuard::start(state)?;
        let settings = state.db.with(store::get_settings)?;
        let webdav = settings.sync_provider == "webdav";
        let folder = if webdav {
            state.data_dir.join("webdav-cache")
        } else {
            PathBuf::from(settings.sync_dir.clone().filter(|d| !d.trim().is_empty()).ok_or_else(|| "Wybierz folder synchronizacji.".to_string())?)
        };
        state.db.with(|conn| sync::resolve(conn, &folder, &base_path(&state.data_dir), &id, keep_local))?;
        if webdav {
            // Bez wysłania na serwer kolejna synchronizacja pobrałaby starą wersję i cofnęła decyzję.
            let password = secrets::get_secret("webdav-password")?.unwrap_or_default();
            sync::webdav_upload_snapshot(&settings, &password, &folder)?;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn cancel_job(state: tauri::State<'_, AppState>) {
    state.cancel.store(true, Ordering::Relaxed);
}

// ---------- Kosz, duplikaty, statystyki ----------

#[tauri::command]
pub async fn list_trash(app: AppHandle) -> Res<Vec<catalog::TrashRow>> {
    run(app, |_, state| state.db.with(catalog::list_trash)).await
}

#[tauri::command]
pub async fn restore_book(app: AppHandle, id: String) -> Res<()> {
    run(app, move |_, state| state.db.with(|conn| catalog::restore_book(conn, &id))).await
}

#[tauri::command]
pub async fn purge_book(app: AppHandle, id: String) -> Res<u32> {
    run(app, move |_, state| {
        let paths = paths(state)?;
        state.db.with(|conn| catalog::purge_book(conn, &paths, &id))
    })
    .await
}

#[tauri::command]
pub async fn empty_trash(app: AppHandle) -> Res<u32> {
    run(app, |_, state| {
        let _guard = JobGuard::start(state)?;
        let paths = paths(state)?;
        state.db.with(|conn| catalog::empty_trash(conn, &paths))
    })
    .await
}

#[tauri::command]
pub async fn find_duplicates(app: AppHandle) -> Res<Vec<catalog::DuplicateGroup>> {
    run(app, |_, state| state.db.with(catalog::find_duplicates)).await
}

#[tauri::command]
pub async fn ignore_duplicates(app: AppHandle, ids: Vec<String>) -> Res<()> {
    run(app, move |_, state| state.db.with(|conn| catalog::ignore_duplicates(conn, &ids))).await
}

#[tauri::command]
pub async fn merge_books(app: AppHandle, keep_id: String, merge_id: String) -> Res<()> {
    run(app, move |_, state| {
        let paths = paths(state)?;
        state.db.with(|conn| catalog::merge_books(conn, &paths, &keep_id, &merge_id))
    })
    .await
}

#[tauri::command]
pub async fn reading_stats(app: AppHandle, year: i32) -> Res<catalog::Stats> {
    run(app, move |_, state| state.db.with(|conn| catalog::stats(conn, year))).await
}

#[tauri::command]
pub async fn list_facets(app: AppHandle) -> Res<catalog::Facets> {
    run(app, |_, state| state.db.with(catalog::facets)).await
}

#[tauri::command]
pub async fn export_csv(app: AppHandle, folder: String) -> Res<String> {
    run(app, move |_, state| state.db.with(|conn| catalog::export_csv(conn, Path::new(&folder))).map(|p| p.display().to_string())).await
}

#[tauri::command]
pub async fn print_rows(app: AppHandle) -> Res<Vec<catalog::ExportRow>> {
    run(app, |_, state| state.db.with(catalog::export_rows)).await
}

// ---------- Import z innych programów ----------

#[tauri::command]
pub async fn preview_import(app: AppHandle, kind: String, path: String) -> Res<Vec<importer::ImportRow>> {
    run(app, move |_, state| state.db.with(|conn| importer::preview(conn, &kind, Path::new(&path)))).await
}

#[tauri::command]
pub async fn commit_import(app: AppHandle, rows: Vec<importer::ImportRow>) -> Res<importer::ImportSummary> {
    run(app, move |app, state| {
        let _guard = JobGuard::start(state)?;
        let paths = paths(state)?;
        let total = rows.len() as u64;
        let mut summary = importer::ImportSummary::default();
        // Partiami, żeby blokada bazy nie trwała przez cały import i żeby dało się przerwać.
        for (index, chunk) in rows.chunks(25).enumerate() {
            if state.cancel.load(Ordering::Relaxed) {
                break;
            }
            emit(app, "import", (index * 25) as u64, total, chunk.first().map(|r| r.title.as_str()).unwrap_or(""), false);
            let part = state.db.with(|conn| importer::commit(conn, &paths, chunk))?;
            summary.added += part.added;
            summary.skipped += part.skipped;
            summary.files += part.files;
            summary.errors.extend(part.errors);
        }
        emit(app, "import", total, total, "Gotowe", true);
        Ok(summary)
    })
    .await
}
