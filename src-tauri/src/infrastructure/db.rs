use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../../migrations/001_init.sql")),
    (2, include_str!("../../migrations/002_evolve.sql")),
    (3, include_str!("../../migrations/003_trash_search.sql")),
];

pub const LATEST_SCHEMA: i64 = 3;

pub struct Database {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(map_sql)?;
        safety_copy_before_migration(&conn, path)?;
        prepare(&conn)?;
        let db = Self {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
        };
        db.with(|conn| {
            after_open(conn)?;
            seed_shelves(conn)?;
            Ok(())
        })?;
        Ok(db)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
        let conn = self.lock()?;
        f(&conn)
    }

    pub fn with_mut<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T, String>) -> Result<T, String> {
        let mut conn = self.lock()?;
        f(&mut conn)
    }

    pub fn lock(&self) -> Result<MutexGuard<'_, Connection>, String> {
        match self.conn.lock() {
            Ok(guard) => Ok(guard),
            Err(poisoned) => {
                eprintln!("mutex bazy był zatruty, połączenie zostało odzyskane");
                Ok(poisoned.into_inner())
            }
        }
    }
}

/// Przed migracją istniejącej bazy zapisuje jej kopię obok pliku bazy.
fn safety_copy_before_migration(conn: &Connection, path: &Path) -> Result<(), String> {
    let current = schema_version(conn)?;
    if current == 0 || current >= LATEST_SCHEMA {
        return Ok(());
    }
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let target = path.with_file_name(format!("biblioteka-przed-v{LATEST_SCHEMA}-{stamp}.sqlite"));
    let mut copy = Connection::open(&target).map_err(map_sql)?;
    {
        let backup = rusqlite::backup::Backup::new(conn, &mut copy).map_err(map_sql)?;
        backup.step(-1).map_err(map_sql)?;
    }
    Ok(())
}

/// Wspólna część dla otwarcia bazy i dla przywrócenia kopii.
pub fn after_open(conn: &Connection) -> Result<(), String> {
    migrate(conn)?;
    backfill(conn)?;
    crate::infrastructure::store::normalize_paths(conn)?;
    let reindex: Option<String> = conn
        .query_row("SELECT value FROM app_settings WHERE key = 'reindex_needed'", [], |row| row.get(0))
        .optional()
        .map_err(map_sql)?;
    if reindex.as_deref() == Some("1") {
        crate::infrastructure::store::reindex_all(conn)?;
        conn.execute("DELETE FROM app_settings WHERE key = 'reindex_needed'", []).map_err(map_sql)?;
    }
    conn.execute(
        "DELETE FROM metadata_cache WHERE expires_at < ?1",
        params![now()],
    )
    .map_err(map_sql)?;
    Ok(())
}

fn prepare(conn: &Connection) -> Result<(), String> {
    conn.pragma_update(None, "foreign_keys", "ON").map_err(map_sql)?;
    conn.pragma_update(None, "busy_timeout", 5000_i64).map_err(map_sql)?;
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    migrate(conn)
}

pub fn migrate(conn: &Connection) -> Result<(), String> {
    let current = schema_version(conn)?;
    for (version, sql) in MIGRATIONS {
        if current < *version {
            let tx = conn.unchecked_transaction().map_err(map_sql)?;
            tx.execute_batch(sql).map_err(map_sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![version, now()],
            )
            .map_err(map_sql)?;
            tx.commit().map_err(map_sql)?;
        }
    }
    Ok(())
}

pub fn schema_version(conn: &Connection) -> Result<i64, String> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |row| row.get(0),
        )
        .map_err(map_sql)?;
    if exists == 0 {
        return Ok(0);
    }
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )
    .map_err(map_sql)
}

fn backfill(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT id, title, authors FROM books WHERE dedupe_key IS NULL OR search_text IS NULL")
        .map_err(map_sql)?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(map_sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql)?;
    drop(stmt);
    for (id, title, authors_json) in rows {
        let authors = serde_json::from_str::<Vec<String>>(&authors_json).unwrap_or_default();
        let key = crate::domain::dedupe_key(&title, &authors);
        let search = crate::domain::fold(&format!("{title} {}", authors.join(" ")));
        let unique = unique_key(conn, &key, &id)?;
        conn.execute(
            "UPDATE books SET dedupe_key = ?1, search_text = ?2 WHERE id = ?3",
            params![unique, search, id],
        )
        .map_err(map_sql)?;
    }
    Ok(())
}

fn unique_key(conn: &Connection, key: &str, id: &str) -> Result<String, String> {
    let taken: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM books WHERE dedupe_key = ?1 AND id != ?2 AND deleted_at IS NULL",
            params![key, id],
            |row| row.get(0),
        )
        .map_err(map_sql)?;
    if taken == 0 {
        Ok(key.to_string())
    } else {
        Ok(format!("{key}#{id}"))
    }
}

fn seed_shelves(conn: &Connection) -> Result<(), String> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM shelves", [], |row| row.get(0))
        .map_err(map_sql)?;
    if count > 0 {
        return Ok(());
    }
    let stamp = now();
    let rows = [
        ("Do przeczytania", 0, "{}"),
        ("Pożyczone", 1, r#"{"loaned":true}"#),
        ("E-booki", 1, r#"{"digital":true}"#),
        ("Nieprzeczytane", 1, r#"{"unread":true}"#),
    ];
    for (index, (name, smart, filter)) in rows.iter().enumerate() {
        conn.execute(
            "INSERT INTO shelves (id, name, sort_order, is_smart_shelf, smart_filter_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![
                uuid::Uuid::new_v4().to_string(),
                name,
                index as i64,
                smart,
                filter,
                stamp
            ],
        )
        .map_err(map_sql)?;
    }
    Ok(())
}

/// Znacznik czasu z milisekundami. Sekundy nie wystarczały: dwie zmiany w tej samej sekundzie
/// na dwóch komputerach miały ten sam znacznik i synchronizacja uznawała je za identyczne.
pub fn now() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn map_sql(error: rusqlite::Error) -> String {
    let text = error.to_string();
    // SQLite podaje w komunikacie kolumny (tabela.kolumna), a nie nazwę indeksu.
    if text.contains("idx_books_dedupe") || text.contains("books.dedupe_key") {
        return "Ta książka jest już w katalogu.".into();
    }
    if text.contains("book_editions.isbn") {
        return "Ten ISBN jest już przy innej książce.".into();
    }
    if text.contains("digital_copies.file_hash_sha256") {
        return "Ten plik jest już w bibliotece.".into();
    }
    if text.contains("database is locked") || text.contains("SQLITE_BUSY") {
        return "Baza jest zajęta. Spróbuj za chwilę.".into();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn fresh_database_reaches_latest_version() {
        let dir = std::env::temp_dir().join(format!("bib-mig-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&dir.join("t.sqlite")).unwrap();
        let version = db
            .with(|conn| schema_version(conn))
            .unwrap();
        assert_eq!(version, LATEST_SCHEMA);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn upgrades_existing_version_one() {
        let dir = std::env::temp_dir().join(format!("bib-up-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../../migrations/001_init.sql")).unwrap();
        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (1, 'x')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO books (id, title, authors, created_at, updated_at) VALUES ('b1', 'Stara', '[]', 't', 't')",
            [],
        )
        .unwrap();
        drop(conn);
        let db = Database::open(&path).unwrap();
        db.with(|conn| {
            let version = schema_version(conn)?;
            assert_eq!(version, LATEST_SCHEMA);
            let key: Option<String> = conn
                .query_row("SELECT dedupe_key FROM books WHERE id = 'b1'", [], |row| row.get(0))
                .map_err(map_sql)?;
            assert!(key.is_some());
            Ok(())
        })
        .unwrap();
        let copies = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("biblioteka-przed-v"))
            .count();
        assert_eq!(copies, 1, "przed migracją powinna powstać kopia bazy");
        let _ = fs::remove_dir_all(dir);
    }
}
