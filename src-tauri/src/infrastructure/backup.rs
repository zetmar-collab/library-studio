use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use chrono::{Datelike, Utc};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::domain::{BackupRow, Settings};
use crate::infrastructure::db::now;
use crate::infrastructure::files::copy_tree;
use crate::infrastructure::store::{self, record_backup};

pub fn backup_database(conn: &Connection, settings: &Settings, kind: &str) -> Result<BackupRow, String> {
    let dir = backup_dir(settings)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let tag = retention_tag();
    let destination = dir.join(format!("{kind}-{tag}-{stamp}.sqlite"));
    let mut target = Connection::open(&destination).map_err(|e| e.to_string())?;
    {
        let backup = rusqlite::backup::Backup::new(conn, &mut target).map_err(|e| e.to_string())?;
        backup.step(-1).map_err(|e| e.to_string())?;
    }
    target.close().map_err(|(_, e)| e.to_string())?;
    let checksum = file_sha256(&destination)?;
    fs::write(destination.with_extension("sha256"), &checksum).map_err(|e| e.to_string())?;
    prune(&dir, kind)?;
    let row = BackupRow {
        id: Uuid::new_v4().to_string(),
        kind: kind.into(),
        destination: destination.display().to_string(),
        status: "ok".into(),
        started_at: now(),
        finished_at: Some(now()),
        size_bytes: Some(fs::metadata(&destination).map(|m| m.len() as i64).unwrap_or(0)),
        checksum: Some(checksum),
        error_message: None,
    };
    record_backup(conn, &row)?;
    Ok(row)
}

pub fn backup_files(conn: &Connection, settings: &Settings, progress: &mut dyn FnMut(u64, u64, &str), cancel: &AtomicBool) -> Result<BackupRow, String> {
    let library = settings.library_dir.clone().ok_or_else(|| "Najpierw wybierz katalog biblioteki.".to_string())?;
    let dir = backup_dir(settings)?.join("pliki");
    let started = now();
    copy_tree(Path::new(&library), &dir, progress, cancel)?;
    let row = BackupRow {
        id: Uuid::new_v4().to_string(),
        kind: "files".into(),
        destination: dir.display().to_string(),
        status: "ok".into(),
        started_at: started,
        finished_at: Some(now()),
        size_bytes: None,
        checksum: None,
        error_message: None,
    };
    record_backup(conn, &row)?;
    Ok(row)
}

/// Kopia bieżącej bazy zapisywana przed przywróceniem, żeby dało się cofnąć pomyłkę.
pub fn safety_copy(conn: &Connection, data_dir: &Path) -> Result<PathBuf, String> {
    let dir = data_dir.join("kopie-przed-przywroceniem");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%3fZ");
    let target = dir.join(format!("przed-przywroceniem-{stamp}.sqlite"));
    let mut copy = Connection::open(&target).map_err(|e| e.to_string())?;
    {
        let backup = rusqlite::backup::Backup::new(conn, &mut copy).map_err(|e| e.to_string())?;
        backup.step(-1).map_err(|e| e.to_string())?;
    }
    copy.close().map_err(|(_, e)| e.to_string())?;
    let checksum = file_sha256(&target)?;
    fs::write(PathBuf::from(format!("{}.sha256", target.display())), checksum).map_err(|e| e.to_string())?;
    Ok(target)
}

pub fn restore_database(app_conn: &mut Connection, backup_path: &Path, data_dir: &Path) -> Result<PathBuf, String> {
    if !backup_path.exists() {
        return Err("Nie ma takiego pliku kopii.".into());
    }
    let checksum_path = PathBuf::from(format!("{}.sha256", backup_path.display()));
    if checksum_path.exists() {
        let expected = fs::read_to_string(&checksum_path).map_err(|e| e.to_string())?;
        let actual = file_sha256(backup_path)?;
        if expected.trim() != actual {
            return Err("Suma kontrolna kopii się nie zgadza. Przywracanie zostało zatrzymane.".into());
        }
    }
    let source = Connection::open_with_flags(backup_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
    let looks_like_library: i64 = source
        .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('books', 'schema_migrations')", [], |row| row.get(0))
        .map_err(|_| "Ten plik nie jest kopią bazy Library Studio.".to_string())?;
    if looks_like_library < 2 {
        return Err("Ten plik nie jest kopią bazy Library Studio.".into());
    }
    let safety = safety_copy(app_conn, data_dir)?;
    {
        let backup = rusqlite::backup::Backup::new(&source, app_conn).map_err(|e| e.to_string())?;
        backup.step(-1).map_err(|e| e.to_string())?;
    }
    // Kopia mogła powstać w starszej wersji programu: migracje, ścieżki i indeks od razu, bez restartu.
    crate::infrastructure::db::migrate(app_conn)?;
    app_conn
        .execute("INSERT INTO app_settings (key, value) VALUES ('reindex_needed', '1') ON CONFLICT(key) DO UPDATE SET value='1'", [])
        .map_err(|e| e.to_string())?;
    crate::infrastructure::db::after_open(app_conn)?;
    Ok(safety)
}

pub fn due(settings: &Settings, conn: &Connection) -> Result<bool, String> {
    if !settings.auto_backup || settings.backup_dir.is_none() {
        return Ok(false);
    }
    let latest: Option<String> = conn
        .query_row(
            "SELECT MAX(started_at) FROM backup_jobs WHERE type = 'database' AND status = 'ok'",
            [],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    let Some(latest) = latest else { return Ok(true) };
    // Znaczniki są w RFC 3339, z milisekundami albo bez (starsze wpisy).
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&latest) else { return Ok(true) };
    Ok(Utc::now() - parsed.with_timezone(&Utc) > chrono::Duration::hours(24))
}

fn backup_dir(settings: &Settings) -> Result<PathBuf, String> {
    settings
        .backup_dir
        .clone()
        .map(PathBuf::from)
        .ok_or_else(|| "Wybierz katalog kopii w ustawieniach.".to_string())
}

fn retention_tag() -> &'static str {
    let today = Utc::now().date_naive();
    if today.day() == 1 {
        "monthly"
    } else if today.weekday().num_days_from_monday() == 6 {
        "weekly"
    } else {
        "daily"
    }
}

fn prune(dir: &Path, kind: &str) -> Result<(), String> {
    let mut daily = Vec::new();
    let mut weekly = Vec::new();
    let mut monthly = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with(kind) || !name.ends_with(".sqlite") {
                continue;
            }
            let bucket = if name.contains("-monthly-") {
                &mut monthly
            } else if name.contains("-weekly-") {
                &mut weekly
            } else {
                &mut daily
            };
            bucket.push(entry.path());
        }
    }
    for (files, keep) in [(&mut daily, 7usize), (&mut weekly, 4), (&mut monthly, 6)] {
        files.sort();
        let overflow = files.len().saturating_sub(keep);
        for path in files.iter().take(overflow) {
            let _ = fs::remove_file(path);
            let _ = fs::remove_file(PathBuf::from(format!("{}.sha256", path.display())));
        }
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn write_export(conn: &Connection, folder: &Path, library: Option<&Path>, progress: &mut dyn FnMut(u64, u64, &str), cancel: &AtomicBool) -> Result<(), String> {
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let snap = store::snapshot(conn)?;
    let json = serde_json::to_vec_pretty(&snap).map_err(|e| e.to_string())?;
    fs::write(folder.join("library.json"), json).map_err(|e| e.to_string())?;
    if let Some(library) = library {
        copy_tree(library, &folder.join("files"), progress, cancel)?;
    }
    Ok(())
}

pub fn read_export(folder: &Path) -> Result<crate::domain::Snapshot, String> {
    let path = folder.join("library.json");
    let text = fs::read_to_string(path).map_err(|_| "W tym katalogu nie ma pliku library.json.".to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("Nie można odczytać eksportu: {e}"))
}

pub fn copy_export_files(folder: &Path, library: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let source = folder.join("files");
    if source.exists() {
        let mut noop = |_: u64, _: u64, _: &str| {};
        copy_tree(&source, library, &mut noop, cancel)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::BookDraft;
    use crate::infrastructure::db::Database;
    use crate::infrastructure::store::{save_book, StorePaths};

    fn draft() -> BookDraft {
        BookDraft {
            id: None,
            title: "Lalka".into(),
            original_title: None,
            subtitle: None,
            authors: vec!["Bolesław Prus".into()],
            translators: vec![],
            publisher: None,
            publication_year: Some(1890),
            language: None,
            description: None,
            series_name: None,
            series_number: None,
            genres: vec![],
            tags: vec![],
            isbn10: None,
            isbn13: None,
            page_count: None,
            cover_url: None,
            metadata_source: Some("manual".into()),
            external_id: None,
            edition_id: None,
            average_rating: None,
            rating_source: None,
        }
    }

    #[test]
    fn backup_roundtrip_keeps_the_book() {
        let dir = std::env::temp_dir().join(format!("bib-bak-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&dir.join("app.sqlite")).unwrap();
        db.with(|conn| {
            save_book(conn, draft(), &StorePaths { library: None, covers: dir.clone() }, None)?;
            let mut settings = store::get_settings(conn)?;
            settings.backup_dir = Some(dir.join("kopie").display().to_string());
            let row = backup_database(conn, &settings, "database")?;
            let restored = Database::open(&dir.join("restored.sqlite")).unwrap();
            restored.with_mut(|target| {
                let safety = restore_database(target, Path::new(&row.destination), &dir)?;
                assert!(safety.exists(), "przed przywróceniem powinna powstać kopia bieżącej bazy");
                assert!(store::book_count(target)? >= 1);
                Ok(())
            })
        })
        .unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn restore_refuses_a_file_that_is_not_a_library() {
        let dir = std::env::temp_dir().join(format!("bib-bad-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let other = dir.join("inna.sqlite");
        Connection::open(&other).unwrap().execute_batch("CREATE TABLE t (x INTEGER);").unwrap();
        let db = Database::open(&dir.join("app.sqlite")).unwrap();
        let result = db.with_mut(|conn| restore_database(conn, &other, &dir).map(|_| ()));
        assert!(result.is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn due_reads_both_timestamp_formats() {
        let dir = std::env::temp_dir().join(format!("bib-due-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&dir.join("app.sqlite")).unwrap();
        db.with(|conn| {
            let mut settings = store::get_settings(conn)?;
            settings.auto_backup = true;
            settings.backup_dir = Some(dir.display().to_string());
            for stamp in [now(), Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()] {
                conn.execute("DELETE FROM backup_jobs", []).unwrap();
                conn.execute("INSERT INTO backup_jobs (id, type, destination, status, started_at) VALUES ('x', 'database', 'd', 'ok', ?1)", [stamp]).unwrap();
                assert!(!due(&settings, conn)?, "świeża kopia nie powinna wymuszać kolejnej");
            }
            Ok(())
        })
        .unwrap();
        let _ = fs::remove_dir_all(dir);
    }
}
