CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY,
  applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS books (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  original_title TEXT,
  subtitle TEXT,
  authors TEXT NOT NULL DEFAULT '[]',
  translators TEXT NOT NULL DEFAULT '[]',
  publisher TEXT,
  publication_year INTEGER,
  language TEXT,
  description TEXT,
  cover_path TEXT,
  series_name TEXT,
  series_number TEXT,
  genres TEXT NOT NULL DEFAULT '[]',
  tags TEXT NOT NULL DEFAULT '[]',
  average_rating REAL,
  rating_source TEXT,
  field_sources TEXT NOT NULL DEFAULT '{}',
  manually_edited_fields TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  deleted_at TEXT
);

CREATE TABLE IF NOT EXISTS book_editions (
  id TEXT PRIMARY KEY,
  book_id TEXT NOT NULL REFERENCES books(id),
  isbn10 TEXT,
  isbn13 TEXT,
  edition_name TEXT,
  publisher TEXT,
  publication_date TEXT,
  page_count INTEGER,
  cover_path TEXT,
  format TEXT,
  external_ids TEXT NOT NULL DEFAULT '{}',
  metadata_source TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS shelves (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  description TEXT,
  color TEXT,
  icon TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0,
  is_smart_shelf INTEGER NOT NULL DEFAULT 0,
  smart_filter_json TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS physical_copies (
  id TEXT PRIMARY KEY,
  book_edition_id TEXT NOT NULL REFERENCES book_editions(id),
  shelf_id TEXT REFERENCES shelves(id),
  location_note TEXT,
  acquisition_date TEXT,
  price TEXT,
  condition TEXT,
  owner TEXT,
  loaned_to TEXT,
  loaned_date TEXT,
  return_due_date TEXT,
  notes TEXT,
  barcode_value TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS digital_copies (
  id TEXT PRIMARY KEY,
  book_edition_id TEXT NOT NULL REFERENCES book_editions(id),
  file_path TEXT NOT NULL,
  file_name TEXT NOT NULL,
  file_size INTEGER NOT NULL,
  file_hash_sha256 TEXT NOT NULL,
  file_format TEXT NOT NULL,
  drm_status TEXT,
  added_at TEXT NOT NULL,
  last_verified_at TEXT,
  notes TEXT
);

CREATE TABLE IF NOT EXISTS shelf_items (
  shelf_id TEXT NOT NULL REFERENCES shelves(id),
  book_id TEXT REFERENCES books(id),
  book_edition_id TEXT REFERENCES book_editions(id),
  position INTEGER NOT NULL,
  added_at TEXT NOT NULL,
  PRIMARY KEY (shelf_id, position),
  CHECK (
    (book_id IS NOT NULL AND book_edition_id IS NULL)
    OR (book_id IS NULL AND book_edition_id IS NOT NULL)
  )
);

CREATE TABLE IF NOT EXISTS reading_progress (
  id TEXT PRIMARY KEY,
  book_id TEXT NOT NULL REFERENCES books(id),
  format TEXT,
  progress_percent REAL,
  current_page INTEGER,
  current_chapter TEXT,
  started_at TEXT,
  finished_at TEXT,
  rating INTEGER,
  review TEXT,
  notes TEXT
);

CREATE TABLE IF NOT EXISTS metadata_cache (
  id TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  external_id TEXT,
  isbn TEXT,
  query_hash TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  fetched_at TEXT NOT NULL,
  expires_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_state (
  id TEXT PRIMARY KEY,
  device_id TEXT NOT NULL,
  provider TEXT NOT NULL,
  last_sync_at TEXT,
  status TEXT NOT NULL,
  error_message TEXT,
  conflict_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS backup_jobs (
  id TEXT PRIMARY KEY,
  type TEXT NOT NULL CHECK (type IN ('database', 'files', 'full')),
  destination TEXT NOT NULL,
  status TEXT NOT NULL,
  started_at TEXT NOT NULL,
  finished_at TEXT,
  size_bytes INTEGER,
  checksum TEXT,
  error_message TEXT
);

CREATE TABLE IF NOT EXISTS app_settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_books_title ON books(title);
CREATE INDEX IF NOT EXISTS idx_books_deleted ON books(deleted_at);
CREATE INDEX IF NOT EXISTS idx_editions_book ON book_editions(book_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_editions_isbn13
  ON book_editions(isbn13) WHERE isbn13 IS NOT NULL AND isbn13 != '';
CREATE UNIQUE INDEX IF NOT EXISTS idx_editions_isbn10
  ON book_editions(isbn10) WHERE isbn10 IS NOT NULL AND isbn10 != '';
CREATE UNIQUE INDEX IF NOT EXISTS idx_digital_hash ON digital_copies(file_hash_sha256);
CREATE INDEX IF NOT EXISTS idx_metadata_query ON metadata_cache(source, query_hash);
