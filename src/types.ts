export type ViewId = "library" | "book" | "add" | "shelves" | "files" | "reader" | "backup" | "settings" | "stats" | "tools";

export type BookCard = {
  id: string;
  title: string;
  authors: string[];
  publicationYear: number | null;
  coverPath: string | null;
  seriesName: string | null;
  seriesNumber: string | null;
  tags: string[];
  createdAt: string;
  averageRating: number | null;
  hasPaper: boolean;
  hasDigital: boolean;
  loanedTo: string | null;
  finished: boolean;
  loanDue: string | null;
  coverFile: string | null;
  updatedAt: string;
};

export type Edition = {
  id: string;
  bookId: string;
  isbn10: string | null;
  isbn13: string | null;
  editionName: string | null;
  publisher: string | null;
  publicationDate: string | null;
  pageCount: number | null;
  coverPath: string | null;
  format: string | null;
  metadataSource: string | null;
};

export type PhysicalCopy = {
  id: string;
  bookEditionId: string;
  shelfId: string | null;
  locationNote: string | null;
  acquisitionDate: string | null;
  price: string | null;
  condition: string | null;
  owner: string | null;
  loanedTo: string | null;
  loanedDate: string | null;
  returnDueDate: string | null;
  notes: string | null;
  barcodeValue: string | null;
};

export type DigitalCopy = {
  id: string;
  bookEditionId: string;
  filePath: string;
  fileName: string;
  fileSize: number;
  fileHashSha256: string;
  fileFormat: string;
  status: string | null;
};

export type Shelf = {
  id: string;
  name: string;
  description: string | null;
  color: string | null;
  sortOrder: number;
  isSmartShelf: boolean;
  smartFilterJson: string | null;
  bookCount: number;
};

export type Progress = {
  id: string;
  bookId: string;
  format: string | null;
  progressPercent: number | null;
  currentPage: number | null;
  currentChapter: string | null;
  startedAt: string | null;
  finishedAt: string | null;
  rating: number | null;
  review: string | null;
  notes: string | null;
};

export type BookDetails = {
  book: {
    id: string;
    title: string;
    originalTitle: string | null;
    subtitle: string | null;
    authors: string[];
    translators: string[];
    publisher: string | null;
    publicationYear: number | null;
    language: string | null;
    description: string | null;
    coverPath: string | null;
    seriesName: string | null;
    seriesNumber: string | null;
    genres: string[];
    tags: string[];
    averageRating: number | null;
    ratingSource: string | null;
    createdAt: string;
    updatedAt: string;
  };
  editions: Edition[];
  physical: PhysicalCopy[];
  digital: DigitalCopy[];
  shelves: Shelf[];
  progress: Progress[];
};

export type BookDraft = {
  id?: string;
  title: string;
  originalTitle?: string;
  subtitle?: string;
  authors: string[];
  translators: string[];
  publisher?: string;
  publicationYear?: number | null;
  language?: string;
  description?: string;
  seriesName?: string;
  seriesNumber?: string;
  genres: string[];
  tags: string[];
  isbn10?: string;
  isbn13?: string;
  pageCount?: number | null;
  coverUrl?: string;
  metadataSource?: string;
  externalId?: string;
  editionId?: string;
  averageRating?: number | null;
  ratingSource?: string;
};

export type CoverCandidate = {
  source: string;
  title: string;
  authors: string[];
  url: string;
};

export type MetadataHit = {
  source: string;
  externalId: string;
  title: string;
  subtitle: string | null;
  authors: string[];
  publisher: string | null;
  publicationYear: number | null;
  publicationDate: string | null;
  language: string | null;
  description: string | null;
  pageCount: number | null;
  isbn10: string | null;
  isbn13: string | null;
  coverUrl: string | null;
  seriesName: string | null;
  genres: string[];
  rating: number | null;
  infoUrl: string | null;
};

export type Settings = {
  libraryDir: string | null;
  theme: string;
  openLibrary: boolean;
  googleBooks: boolean;
  nationalLibrary: boolean;
  lubimyCzytac: boolean;
  upolujebooka: boolean;
  googleBooksHasKey: boolean;
  locale: string;
  autoBackup: boolean;
  backupDir: string | null;
  syncProvider: string;
  syncDir: string | null;
  webdavUrl: string | null;
  webdavUser: string | null;
  webdavHasPassword: boolean;
  readingGoal: number | null;
};

export type ListQuery = {
  q?: string;
  view?: string;
  shelfId?: string;
  author?: string;
  series?: string;
  tag?: string;
  year?: number | null;
  sort?: string;
};

export type FileRow = {
  id: string;
  bookId: string;
  bookTitle: string;
  fileName: string;
  filePath: string;
  fileFormat: string;
  fileSize: number;
  status: string;
};

export type BackupRow = {
  id: string;
  kind: string;
  destination: string;
  status: string;
  startedAt: string;
  finishedAt: string | null;
  sizeBytes: number | null;
  checksum: string | null;
  errorMessage: string | null;
};

export type ConflictRow = {
  id: string;
  bookId: string | null;
  summary: string;
  localUpdatedAt: string | null;
  remoteUpdatedAt: string | null;
  createdAt: string;
};

export type ReaderDevice = {
  kind: string;
  name: string;
  root: string;
  formats: string[];
};

export type ReaderFile = {
  relativePath: string;
  fileName: string;
  size: number;
  format: string;
  alreadyInCatalog: boolean;
};

export type TransferNote = {
  code: string;
  name: string;
  detail: string;
};

export type TransferReport = {
  copied: number;
  skipped: number;
  added: number;
  notes: TransferNote[];
};

export type JobProgress = {
  job: string;
  current: number;
  total: number;
  label: string;
  done: boolean;
};

export { errorText } from "./i18n";

export type SyncReport = { pushed: number; pulled: number; conflicts: number; skipped: number; message: string };

export type TrashRow = { id: string; title: string; authors: string[]; deletedAt: string; files: number };

export type DuplicateBook = {
  id: string;
  title: string;
  authors: string[];
  publicationYear: number | null;
  isbn: string | null;
  files: number;
  copies: number;
};

export type FileRepair = { bookId: string; oldTitle: string; oldAuthors: string[]; newTitle: string; newAuthors: string[] };

export type DuplicateGroup = { reason: "isbn" | "title"; books: DuplicateBook[] };

export type Count = { name: string; count: number };

export type Stats = {
  year: number;
  goal: number | null;
  finished: number;
  pages: number;
  byMonth: number[];
  totalBooks: number;
  paper: number;
  ebooks: number;
  loaned: number;
  overdue: number;
  unread: number;
  topGenres: Count[];
  topAuthors: Count[];
  byYear: Count[];
};

export type Facets = { authors: Count[]; series: Count[]; tags: Count[]; years: Count[] };

export type ExportRow = {
  id: string;
  title: string;
  authors: string[];
  isbn13: string | null;
  isbn10: string | null;
  publisher: string | null;
  year: number | null;
  language: string | null;
  series: string | null;
  seriesNumber: string | null;
  genres: string[];
  tags: string[];
  shelves: string[];
  paper: boolean;
  ebook: boolean;
  location: string | null;
  barcode: string | null;
  loanedTo: string | null;
  returnDue: string | null;
  rating: number | null;
  finishedAt: string | null;
};

export type ImportRow = {
  title: string;
  authors: string[];
  isbn13: string | null;
  isbn10: string | null;
  publisher: string | null;
  year: number | null;
  language: string | null;
  description: string | null;
  seriesName: string | null;
  seriesNumber: string | null;
  genres: string[];
  tags: string[];
  pageCount: number | null;
  rating: number | null;
  finishedAt: string | null;
  toRead: boolean;
  files: string[];
  coverFile: string | null;
  duplicateOf: string | null;
};

export type ImportSummary = { added: number; skipped: number; files: number; errors: string[] };
