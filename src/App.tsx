import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, inDesktop } from "./api";
import { Shell } from "./Shell";
import { useI18n } from "./i18n";
import { typingInField, useUi } from "./ui";
import { errorText, type BookCard, type JobProgress, type Shelf, type ViewId } from "./types";
import { AddView } from "./views/AddView";
import { BackupView } from "./views/BackupView";
import { BookView } from "./views/BookView";
import { FilesView } from "./views/FilesView";
import { LibraryView, type LibraryFilters } from "./views/LibraryView";
import { ReaderView } from "./views/ReaderView";
import { SettingsView } from "./views/SettingsView";
import { ShelvesView } from "./views/ShelvesView";
import { StatsView } from "./views/StatsView";
import { ToolsView } from "./views/ToolsView";

const emptyFilters: LibraryFilters = { author: "", series: "", tag: "", year: "" };

export function App() {
  const { setLocale, t } = useI18n();
  const { notify, confirm, modalOpen } = useUi();
  const [view, setView] = useState<ViewId>("library");
  const [books, setBooks] = useState<BookCard[]>([]);
  const [shelves, setShelves] = useState<Shelf[]>([]);
  const [bookId, setBookId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [filter, setFilter] = useState("");
  const [sort, setSort] = useState("title");
  const [shelfId, setShelfId] = useState("");
  const [filters, setFilters] = useState<LibraryFilters>(emptyFilters);
  const [error, setError] = useState<string | null>(null);
  const [needsLibrary, setNeedsLibrary] = useState(false);
  const [progress, setProgress] = useState<JobProgress | null>(null);
  const [readerBooks, setReaderBooks] = useState<string[]>([]);
  const [overdue, setOverdue] = useState<BookCard[]>([]);
  const [overdueHidden, setOverdueHidden] = useState(false);
  const dirty = useRef(false);
  const [theme, setTheme] = useState<"light" | "dark">(() =>
    document.documentElement.dataset.theme === "dark" ? "dark" : "light",
  );
  const settingsApplied = useRef(false);

  // Wyszukiwanie odpytuje bazę dopiero po krótkiej przerwie w pisaniu.
  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedQuery(query), 180);
    return () => window.clearTimeout(timer);
  }, [query]);

  const refresh = useCallback(async () => {
    if (!inDesktop()) {
      setError(t("previewOnly"));
      return;
    }
    const [nextBooks, nextShelves, late] = await Promise.all([
      api.listBooks({
        q: debouncedQuery,
        view: filter,
        shelfId: shelfId || undefined,
        sort,
        author: filters.author || undefined,
        series: filters.series || undefined,
        tag: filters.tag || undefined,
        year: filters.year ? Number(filters.year) : null,
      }),
      api.shelves(),
      api.listBooks({ view: "overdue", sort: "title" }),
    ]);
    setBooks(nextBooks);
    setShelves(nextShelves);
    setOverdue(late);
    setError(null);
  }, [debouncedQuery, filter, shelfId, sort, filters, t]);

  const safeRefresh = useCallback(() => {
    refresh().catch((cause: unknown) => setError(errorText(cause)));
  }, [refresh]);

  useEffect(() => {
    safeRefresh();
  }, [safeRefresh]);

  // Ustawienia (język, motyw, folder) wczytujemy raz, a nie przy każdym wpisanym znaku.
  useEffect(() => {
    if (!inDesktop() || settingsApplied.current) return;
    settingsApplied.current = true;
    api
      .settings()
      .then((settings) => {
        setNeedsLibrary(!settings.libraryDir);
        if (settings.locale === "en" || settings.locale === "pl") setLocale(settings.locale);
        if (settings.theme === "dark" || settings.theme === "light") changeTheme(settings.theme);
      })
      .catch((cause: unknown) => setError(errorText(cause)));
  }, [setLocale]);

  useEffect(() => {
    if (!inDesktop()) return;
    const unlisten = listen<JobProgress>("job-progress", (event) => setProgress(event.payload));
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, []);

  const setDirty = useCallback((value: boolean) => {
    dirty.current = value;
  }, []);

  const goTo = useCallback(
    async (next: ViewId) => {
      // Niezapisane zmiany w formularzu nie znikają po cichu.
      if (dirty.current && !(await confirm(t("leaveUnsaved"), { confirmLabel: t("leaveAnyway"), danger: true }))) return;
      dirty.current = false;
      setView(next);
    },
    [confirm, t],
  );

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (modalOpen()) return;
      if (event.key === "Escape" && !typingInField(event) && view !== "library") void goTo("library");
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        void goTo("library").then(() => window.setTimeout(() => document.getElementById("library-search")?.focus(), 0));
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "n") {
        event.preventDefault();
        void goTo("add");
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [goTo, modalOpen, view]);

  function changeTheme(next: "light" | "dark") {
    setTheme(next);
    document.documentElement.dataset.theme = next;
    localStorage.setItem("biblioteka-theme", next);
  }

  async function chooseLibrary() {
    const folder = await api.pickFolder();
    if (!folder) return;
    await api.saveSettings({ libraryDir: folder });
    setNeedsLibrary(false);
    safeRefresh();
  }

  function openBook(id: string) {
    setBookId(id);
    dirty.current = false;
    setView("book");
  }

  return (
    <Shell view={view === "book" || view === "add" ? "library" : view} onView={(next) => void goTo(next)} onAdd={() => void goTo("add")}>
      {error ? <p className="mb-6 border-l-2 border-stamp pl-4 text-stamp" role="alert">{error}</p> : null}
      {needsLibrary ? (
        <div className="mb-6 border border-rule p-4 dark:border-night-rule">
          <p>{t("chooseLibrary")}</p>
          <button type="button" className="mt-3 bg-stamp px-4 py-2 text-sheet" onClick={() => void chooseLibrary().catch((cause) => setError(errorText(cause)))}>{t("chooseFolder")}</button>
        </div>
      ) : null}
      {overdue.length > 0 && !overdueHidden && view === "library" ? (
        <div className="mb-6 flex flex-wrap items-center gap-3 border-l-2 border-stamp bg-sheet px-4 py-3 dark:bg-night-sheet" role="status">
          <span className="flex-1">
            {t("overdueBanner")}: {overdue.slice(0, 3).map((book) => `${book.title} (${book.loanedTo ?? "?"}, ${book.loanDue ?? ""})`).join("; ")}
            {overdue.length > 3 ? ` +${overdue.length - 3}` : ""}
          </span>
          <button type="button" className="underline" onClick={() => setFilter("overdue")}>{t("overdueShow")}</button>
          <button type="button" className="text-muted" onClick={() => setOverdueHidden(true)}>{t("hide")}</button>
        </div>
      ) : null}
      {view === "library" ? (
        <LibraryView
          books={books}
          shelves={shelves}
          query={query}
          view={filter}
          sort={sort}
          shelfId={shelfId}
          filters={filters}
          onQuery={setQuery}
          onView={setFilter}
          onSort={setSort}
          onShelf={setShelfId}
          onFilters={setFilters}
          onOpen={openBook}
          onAdd={() => void goTo("add")}
          onChanged={safeRefresh}
          onSend={(ids) => { setReaderBooks(ids); setView("reader"); }}
        />
      ) : null}
      {view === "add" ? (
        <AddView
          onCancel={() => void goTo("library")}
          onDirty={setDirty}
          onSaved={(id, created) => {
            if (!created) notify(t("alreadyInCatalog"));
            openBook(id);
            safeRefresh();
          }}
          onBatchSaved={safeRefresh}
        />
      ) : null}
      {view === "book" && bookId ? (
        <BookView
          id={bookId}
          shelves={shelves}
          onBack={() => void goTo("library")}
          onChanged={safeRefresh}
          onDirty={setDirty}
          onDeleted={() => { dirty.current = false; setView("library"); safeRefresh(); }}
          onSend={(id) => { setReaderBooks([id]); setView("reader"); }}
        />
      ) : null}
      {view === "shelves" ? <ShelvesView shelves={shelves} onChanged={safeRefresh} /> : null}
      {view === "files" ? <FilesView progress={progress} /> : null}
      {view === "reader" ? <ReaderView books={books} bookIds={readerBooks} onChanged={safeRefresh} /> : null}
      {view === "backup" ? <BackupView progress={progress} onChanged={safeRefresh} /> : null}
      {view === "stats" ? <StatsView /> : null}
      {view === "tools" ? <ToolsView progress={progress} onChanged={safeRefresh} onOpen={openBook} /> : null}
      {view === "settings" ? <SettingsView theme={theme} onTheme={changeTheme} onLocale={setLocale} onChanged={(settings) => { safeRefresh(); setNeedsLibrary(!settings.libraryDir); }} /> : null}
    </Shell>
  );
}
