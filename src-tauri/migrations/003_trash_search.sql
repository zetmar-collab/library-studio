-- ISBN i hash pliku muszą być unikalne tylko wśród książek, które nie są w koszu.
-- SQLite nie pozwala w indeksie częściowym zajrzeć do innej tabeli, więc pilnuje tego kod.
DROP INDEX IF EXISTS idx_editions_isbn13;
DROP INDEX IF EXISTS idx_editions_isbn10;
DROP INDEX IF EXISTS idx_digital_hash;
CREATE INDEX IF NOT EXISTS idx_editions_isbn13_any ON book_editions(isbn13);
CREATE INDEX IF NOT EXISTS idx_editions_isbn10_any ON book_editions(isbn10);
CREATE INDEX IF NOT EXISTS idx_digital_hash_any ON digital_copies(file_hash_sha256);

-- Półki usuwane miękko, żeby usunięcie przechodziło przez synchronizację.
ALTER TABLE shelves ADD COLUMN deleted_at TEXT;

-- Książka usunięta z kosza zostaje jako pusty nagrobek dla synchronizacji.
ALTER TABLE books ADD COLUMN purged_at TEXT;

-- Wyszukiwanie pełnotekstowe. Treść jest już złożona funkcją fold() po stronie Rusta.
CREATE VIRTUAL TABLE IF NOT EXISTS books_fts USING fts5(book_id UNINDEXED, body);

CREATE INDEX IF NOT EXISTS idx_physical_due ON physical_copies(return_due_date);

INSERT INTO app_settings (key, value) VALUES ('reindex_needed', '1')
  ON CONFLICT(key) DO UPDATE SET value = '1';

DELETE FROM metadata_cache WHERE expires_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now');
