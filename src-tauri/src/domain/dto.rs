use serde::{Deserialize, Serialize};
use serde_json::Value;

fn empty_vec() -> Vec<String> {
    Vec::new()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookCard {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub publication_year: Option<i64>,
    pub cover_path: Option<String>,
    pub series_name: Option<String>,
    pub series_number: Option<String>,
    pub tags: Vec<String>,
    pub created_at: String,
    pub average_rating: Option<f64>,
    pub has_paper: bool,
    pub has_digital: bool,
    pub loaned_to: Option<String>,
    pub finished: bool,
    /// Najbliższy termin zwrotu pożyczonego egzemplarza (RRRR-MM-DD).
    pub loan_due: Option<String>,
    /// Pełna ścieżka okładki do wyświetlenia przez protokół asset, jeśli plik istnieje.
    pub cover_file: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionRecord {
    pub id: String,
    pub book_id: String,
    pub isbn10: Option<String>,
    pub isbn13: Option<String>,
    pub edition_name: Option<String>,
    pub publisher: Option<String>,
    pub publication_date: Option<String>,
    pub page_count: Option<i64>,
    pub cover_path: Option<String>,
    pub format: Option<String>,
    pub external_ids: Value,
    pub metadata_source: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalRecord {
    pub id: String,
    pub book_edition_id: String,
    pub shelf_id: Option<String>,
    pub location_note: Option<String>,
    pub acquisition_date: Option<String>,
    pub price: Option<String>,
    pub condition: Option<String>,
    pub owner: Option<String>,
    pub loaned_to: Option<String>,
    pub loaned_date: Option<String>,
    pub return_due_date: Option<String>,
    pub notes: Option<String>,
    pub barcode_value: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DigitalRecord {
    pub id: String,
    pub book_edition_id: String,
    pub file_path: String,
    pub file_name: String,
    pub file_size: i64,
    pub file_hash_sha256: String,
    pub file_format: String,
    pub drm_status: Option<String>,
    pub added_at: String,
    pub last_verified_at: Option<String>,
    pub notes: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfRecord {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub sort_order: i64,
    pub is_smart_shelf: bool,
    pub smart_filter_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub book_count: i64,
    #[serde(default)]
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressRecord {
    pub id: String,
    pub book_id: String,
    pub format: Option<String>,
    pub progress_percent: Option<f64>,
    pub current_page: Option<i64>,
    pub current_chapter: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub rating: Option<i64>,
    pub review: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookRecord {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub original_title: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    #[serde(default = "empty_vec")]
    pub translators: Vec<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub publication_year: Option<i64>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub cover_path: Option<String>,
    #[serde(default)]
    pub series_name: Option<String>,
    #[serde(default)]
    pub series_number: Option<String>,
    #[serde(default = "empty_vec")]
    pub genres: Vec<String>,
    #[serde(default = "empty_vec")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub average_rating: Option<f64>,
    #[serde(default)]
    pub rating_source: Option<String>,
    pub field_sources: Value,
    pub manually_edited_fields: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub deleted_at: Option<String>,
    #[serde(default)]
    pub purged_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDetails {
    pub book: BookRecord,
    pub editions: Vec<EditionRecord>,
    pub physical: Vec<PhysicalRecord>,
    pub digital: Vec<DigitalRecord>,
    pub shelves: Vec<ShelfRecord>,
    pub progress: Vec<ProgressRecord>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDraft {
    pub id: Option<String>,
    pub title: String,
    pub original_title: Option<String>,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub translators: Vec<String>,
    pub publisher: Option<String>,
    pub publication_year: Option<i64>,
    pub language: Option<String>,
    pub description: Option<String>,
    pub series_name: Option<String>,
    pub series_number: Option<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub isbn10: Option<String>,
    pub isbn13: Option<String>,
    pub page_count: Option<i64>,
    pub cover_url: Option<String>,
    pub metadata_source: Option<String>,
    pub external_id: Option<String>,
    pub edition_id: Option<String>,
    pub average_rating: Option<f64>,
    pub rating_source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    pub book_id: String,
    pub created: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub view: String,
    #[serde(default)]
    pub shelf_id: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub series: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub year: Option<i64>,
    #[serde(default)]
    pub sort: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalDraft {
    pub id: Option<String>,
    pub book_id: String,
    pub shelf_id: Option<String>,
    pub location_note: Option<String>,
    pub acquisition_date: Option<String>,
    pub price: Option<String>,
    pub condition: Option<String>,
    pub owner: Option<String>,
    pub loaned_to: Option<String>,
    pub loaned_date: Option<String>,
    pub return_due_date: Option<String>,
    pub notes: Option<String>,
    pub barcode_value: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressDraft {
    pub id: Option<String>,
    pub book_id: String,
    pub format: Option<String>,
    pub progress_percent: Option<f64>,
    pub current_page: Option<i64>,
    pub current_chapter: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub rating: Option<i64>,
    pub review: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfDraft {
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub is_smart_shelf: bool,
    pub smart_filter_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataHit {
    pub source: String,
    pub external_id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub publisher: Option<String>,
    pub publication_year: Option<i64>,
    pub publication_date: Option<String>,
    pub language: Option<String>,
    pub description: Option<String>,
    pub page_count: Option<i64>,
    pub isbn10: Option<String>,
    pub isbn13: Option<String>,
    pub cover_url: Option<String>,
    pub series_name: Option<String>,
    pub genres: Vec<String>,
    pub rating: Option<f64>,
    pub info_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverCandidate {
    pub source: String,
    pub title: String,
    pub authors: Vec<String>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataSearch {
    pub hits: Vec<MetadataHit>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataQuery {
    #[serde(default)]
    pub isbn: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub library_dir: Option<String>,
    pub theme: String,
    pub open_library: bool,
    pub google_books: bool,
    pub national_library: bool,
    pub lubimy_czytac: bool,
    pub upolujebooka: bool,
    pub google_books_has_key: bool,
    pub locale: String,
    pub auto_backup: bool,
    pub backup_dir: Option<String>,
    pub sync_provider: String,
    pub sync_dir: Option<String>,
    pub webdav_url: Option<String>,
    pub webdav_user: Option<String>,
    pub webdav_has_password: bool,
    pub reading_goal: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub library_dir: Option<String>,
    pub theme: Option<String>,
    pub open_library: Option<bool>,
    pub google_books: Option<bool>,
    pub national_library: Option<bool>,
    #[serde(default)]
    pub lubimy_czytac: Option<bool>,
    #[serde(default)]
    pub upolujebooka: Option<bool>,
    pub locale: Option<String>,
    pub auto_backup: Option<bool>,
    pub backup_dir: Option<String>,
    pub sync_provider: Option<String>,
    pub sync_dir: Option<String>,
    pub webdav_url: Option<String>,
    pub webdav_user: Option<String>,
    #[serde(default)]
    pub reading_goal: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfItemRecord {
    pub id: String,
    pub shelf_id: String,
    pub book_id: Option<String>,
    pub book_edition_id: Option<String>,
    pub position: i64,
    pub added_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub version: u32,
    pub books: Vec<BookRecord>,
    pub editions: Vec<EditionRecord>,
    pub shelves: Vec<ShelfRecord>,
    pub physical: Vec<PhysicalRecord>,
    pub digital: Vec<DigitalRecord>,
    pub shelf_items: Vec<ShelfItemRecord>,
    pub progress: Vec<ProgressRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRow {
    pub id: String,
    pub book_id: String,
    pub book_title: String,
    pub file_name: String,
    pub file_path: String,
    pub file_format: String,
    pub file_size: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRow {
    pub id: String,
    pub kind: String,
    pub destination: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub size_bytes: Option<i64>,
    pub checksum: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictRow {
    pub id: String,
    pub book_id: Option<String>,
    pub summary: String,
    pub local_updated_at: Option<String>,
    pub remote_updated_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub job: String,
    pub current: u64,
    pub total: u64,
    pub label: String,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub pushed: u32,
    pub pulled: u32,
    pub conflicts: u32,
    #[serde(default)]
    pub skipped: u32,
    pub message: String,
}

/// Wynik wczytania eksportu albo danych z synchronizacji.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub inserted: u32,
    pub updated: u32,
    /// Książki pominięte, bo w katalogu jest już inna pozycja o tym samym tytule i autorze albo ISBN.
    pub skipped: u32,
    pub skipped_titles: Vec<String>,
}
