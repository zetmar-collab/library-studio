import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type {
  BackupRow,
  BookCard,
  BookDetails,
  BookDraft,
  ConflictRow,
  FileRow,
  ListQuery,
  CoverCandidate,
  MetadataHit,
  PhysicalCopy,
  Progress,
  ReaderDevice,
  ReaderFile,
  Settings,
  Shelf,
  TransferReport,
  TrashRow,
  DuplicateGroup,
  Stats,
  Facets,
  ExportRow,
  ImportRow,
  ImportSummary,
  SyncReport,
} from "./types";

export function inDesktop(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!inDesktop()) {
    throw new Error("Ta czynność działa w oknie aplikacji.");
  }
  return invoke<T>(command, args);
}

export const api = {
  status: () => call<{ dbPath: string; schemaVersion: number; bookCount: number }>("app_status"),
  settings: () => call<Settings>("get_settings"),
  saveSettings: (patch: Partial<Settings>) => call<Settings>("save_settings", { patch }),
  setSecret: (account: string, value: string) => call<void>("set_secret", { account, value }),
  pickFolder: () => call<string | null>("pick_folder"),
  pickFile: () => call<string | null>("pick_file"),
  pickCsvFile: () => call<string | null>("pick_csv_file"),
  pickBackupFile: () => call<string | null>("pick_backup_file"),
  listBooks: (query: ListQuery) => call<BookCard[]>("list_books", { query }),
  getBook: (id: string) => call<BookDetails>("get_book", { id }),
  saveBook: (draft: BookDraft) => call<{ bookId: string; created: boolean }>("save_book", { draft }),
  enrichBook: (id: string) => call<{ filled: number; checked: number }>("enrich_book", { id }),
  enrichLibrary: () => call<{ filled: number; checked: number; failed: number }>("enrich_library"),
  deleteBook: (id: string) => call<void>("delete_book", { id }),
  searchMetadata: (query: { isbn?: string; title?: string; author?: string }) =>
    call<{ hits: MetadataHit[]; errors: string[] }>("search_metadata", { query }),
  searchCovers: (query: { isbn?: string; title?: string; author?: string }) =>
    call<CoverCandidate[]>("search_covers", { query }),
  previewCover: (url: string) => call<string>("preview_cover", { url }),
  attachCover: (bookId: string, url: string) => call<void>("attach_cover", { bookId, url }),
  savePhysical: (draft: Partial<PhysicalCopy> & { bookId: string }) => call<string>("save_physical", { draft }),
  deletePhysical: (id: string) => call<void>("delete_physical", { id }),
  saveProgress: (draft: Partial<Progress> & { bookId: string }) => call<string>("save_progress", { draft }),
  shelves: () => call<Shelf[]>("list_shelves"),
  saveShelf: (draft: Partial<Shelf> & { name: string; isSmartShelf: boolean }) => call<string>("save_shelf", { draft }),
  deleteShelf: (id: string) => call<void>("delete_shelf", { id }),
  reorderShelf: (shelfId: string, bookIds: string[]) => call<void>("reorder_shelf", { shelfId, bookIds }),
  addToShelf: (shelfId: string, bookId: string) => call<void>("add_to_shelf", { shelfId, bookId }),
  removeFromShelf: (shelfId: string, bookId: string) => call<void>("remove_from_shelf", { shelfId, bookId }),
  addFile: (bookId: string, sourcePath: string) => call<string>("add_file", { bookId, sourcePath }),
  files: () => call<FileRow[]>("list_files"),
  verifyFiles: () => call<FileRow[]>("verify_files"),
  openFile: (id: string) => call<void>("open_file", { id }),
  cover: (bookId: string) => call<string | null>("cover_data", { bookId }),
  backups: () => call<BackupRow[]>("list_backups"),
  backup: (kind: "database" | "files" | "full") => call<BackupRow>("backup_now", { kind }),
  restore: (path: string) => call<string>("restore_backup", { path }),
  exportLibrary: (folder: string) => call<void>("export_library", { folder }),
  importLibrary: (folder: string) => call<string>("import_library", { folder }),
  sync: () => call<SyncReport>("sync_now"),
  conflicts: () => call<ConflictRow[]>("list_conflicts"),
  resolveConflict: (id: string, keepLocal: boolean) => call<void>("resolve_conflict", { id, keepLocal }),
  cancel: () => call<void>("cancel_job"),
  readers: () => call<ReaderDevice[]>("list_readers"),
  openReaderFolder: (path: string) => call<ReaderDevice>("open_reader_folder", { path }),
  readerBooks: (root: string) => call<ReaderFile[]>("list_reader_books", { root }),
  sendToReader: (root: string, bookIds: string[]) => call<TransferReport>("send_to_reader", { root, bookIds }),
  importFromReader: (root: string, relativePaths: string[]) => call<TransferReport>("import_from_reader", { root, relativePaths }),
  trash: () => call<TrashRow[]>("list_trash"),
  restoreBook: (id: string) => call<void>("restore_book", { id }),
  purgeBook: (id: string) => call<number>("purge_book", { id }),
  emptyTrash: () => call<number>("empty_trash"),
  duplicates: () => call<DuplicateGroup[]>("find_duplicates"),
  ignoreDuplicates: (ids: string[]) => call<void>("ignore_duplicates", { ids }),
  mergeBooks: (keepId: string, mergeId: string) => call<void>("merge_books", { keepId, mergeId }),
  stats: (year: number) => call<Stats>("reading_stats", { year }),
  facets: () => call<Facets>("list_facets"),
  exportCsv: (folder: string) => call<string>("export_csv", { folder }),
  printRows: () => call<ExportRow[]>("print_rows"),
  previewImport: (kind: "calibre" | "goodreads" | "csv", path: string) => call<ImportRow[]>("preview_import", { kind, path }),
  commitImport: (rows: ImportRow[]) => call<ImportSummary>("commit_import", { rows }),
};

/** Adres okładki z dysku przez protokół asset; parametr v odświeża obraz po zmianie okładki. */
export function coverSrc(file: string | null, version?: string): string | null {
  if (!file || !inDesktop()) return null;
  return `${convertFileSrc(file)}${version ? `?v=${encodeURIComponent(version)}` : ""}`;
}
