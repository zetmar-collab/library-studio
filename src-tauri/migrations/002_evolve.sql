ALTER TABLE books ADD COLUMN dedupe_key TEXT;
ALTER TABLE books ADD COLUMN search_text TEXT;

CREATE TABLE shelf_items_v2 (
  id TEXT PRIMARY KEY,
  shelf_id TEXT NOT NULL REFERENCES shelves(id),
  book_id TEXT REFERENCES books(id),
  book_edition_id TEXT REFERENCES book_editions(id),
  position INTEGER NOT NULL,
  added_at TEXT NOT NULL,
  CHECK (
    (book_id IS NOT NULL AND book_edition_id IS NULL)
    OR (book_id IS NULL AND book_edition_id IS NOT NULL)
  )
);

INSERT INTO shelf_items_v2 (id, shelf_id, book_id, book_edition_id, position, added_at)
SELECT lower(hex(randomblob(16))), shelf_id, book_id, book_edition_id, position, added_at
FROM shelf_items;

DROP TABLE shelf_items;
ALTER TABLE shelf_items_v2 RENAME TO shelf_items;

CREATE UNIQUE INDEX IF NOT EXISTS idx_shelf_book
  ON shelf_items(shelf_id, book_id) WHERE book_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_shelf_edition
  ON shelf_items(shelf_id, book_edition_id) WHERE book_edition_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_books_dedupe_live
  ON books(dedupe_key) WHERE deleted_at IS NULL AND dedupe_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS sync_conflicts (
  id TEXT PRIMARY KEY,
  book_id TEXT,
  summary TEXT NOT NULL,
  local_updated_at TEXT,
  remote_updated_at TEXT,
  created_at TEXT NOT NULL,
  resolved_at TEXT
);
