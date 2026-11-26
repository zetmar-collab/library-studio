import { useEffect, useState } from "react";
import { api, coverSrc } from "../api";
import { useI18n } from "../i18n";
import { useUi } from "../ui";
import { highlight } from "../text";
import { errorText, type BookCard, type Facets, type Shelf } from "../types";

export type LibraryFilters = { author: string; series: string; tag: string; year: string };

type LibraryViewProps = {
  books: BookCard[];
  shelves: Shelf[];
  query: string;
  view: string;
  sort: string;
  shelfId: string;
  filters: LibraryFilters;
  onQuery: (value: string) => void;
  onView: (value: string) => void;
  onSort: (value: string) => void;
  onShelf: (value: string) => void;
  onFilters: (value: LibraryFilters) => void;
  onOpen: (id: string) => void;
  onAdd: () => void;
  onChanged: () => void;
  onSend: (ids: string[]) => void;
};

const field = "border border-rule bg-sheet px-2 py-1.5 dark:border-night-rule dark:bg-night";

export function LibraryView(props: LibraryViewProps) {
  const { t, shelfName } = useI18n();
  const { confirm, notify } = useUi();
  const [fillNote, setFillNote] = useState<string | null>(null);
  const [filling, setFilling] = useState(false);
  const [picked, setPicked] = useState<string[]>([]);
  const [facets, setFacets] = useState<Facets | null>(null);
  const [layout, setLayout] = useState<"list" | "grid">(() => (localStorage.getItem("library-layout") === "grid" ? "grid" : "list"));
  const [showFilters, setShowFilters] = useState(() => Object.values(props.filters).some(Boolean));

  useEffect(() => {
    api.facets().then(setFacets).catch(() => setFacets(null));
  }, [props.books.length]);

  useEffect(() => {
    localStorage.setItem("library-layout", layout);
  }, [layout]);

  // Po zmianie filtra nie zostają zaznaczone książki, których już nie widać.
  useEffect(() => {
    setPicked((current) => current.filter((id) => props.books.some((book) => book.id === id)));
  }, [props.books]);

  const views = [
    ["", t("filterAll")],
    ["paper", t("filterPaper")],
    ["ebook", t("filterEbook")],
    ["loaned", t("filterLoaned")],
    ["overdue", t("filterOverdue")],
    ["finished", t("filterFinished")],
    ["unread", t("unread")],
  ];
  const activeFilters = Object.values(props.filters).filter(Boolean).length;

  /// Usunięte książki trafiają do kosza; „Cofnij” przywraca je wszystkie.
  async function removeBooks(ids: string[]) {
    if (ids.length === 0) return;
    const first = props.books.find((book) => book.id === ids[0]);
    const question = ids.length === 1 && first
      ? t("deleteBookConfirm").replace("{title}", first.title)
      : t("deleteBooksConfirm").replace("{count}", String(ids.length));
    const ok = await confirm(question, { danger: true, confirmLabel: t("moveToTrash") });
    if (!ok) return;
    const removed: string[] = [];
    try {
      for (const id of ids) {
        await api.deleteBook(id);
        removed.push(id);
      }
    } catch (cause) {
      notify(errorText(cause));
    }
    setPicked((current) => current.filter((id) => !removed.includes(id)));
    props.onChanged();
    if (removed.length) {
      notify(removed.length === 1 ? t("movedToTrash") : t("movedManyToTrash").replace("{count}", String(removed.length)), {
        label: t("undo"),
        run: () => {
          void Promise.all(removed.map((id) => api.restoreBook(id))).then(props.onChanged).catch((cause) => notify(errorText(cause)));
        },
      });
    }
  }

  const allPicked = props.books.length > 0 && props.books.every((book) => picked.includes(book.id));

  async function fillLibrary() {
    setFilling(true);
    setFillNote(t("fillWorking"));
    try {
      const report = await api.enrichLibrary();
      setFillNote(`${t("fillDone")}: ${report.filled} / ${report.checked}${report.failed ? ` · ${t("fillFailed")}: ${report.failed}` : ""}`);
      props.onChanged();
    } catch (cause) {
      setFillNote(errorText(cause));
    } finally {
      setFilling(false);
    }
  }

  return (
    <section>
      <header className="border-b border-rule pb-6 dark:border-night-rule">
        <div className="flex flex-wrap items-end justify-between gap-4">
          <h1 className="font-display text-5xl leading-none">{t("catalog")}</h1>
          <div className="flex flex-wrap gap-2">
            <button type="button" disabled={picked.length === 0} onClick={() => props.onSend(picked)} className="border border-rule px-4 py-2 disabled:opacity-50">{t("sendToReader")}{picked.length ? ` (${picked.length})` : ""}</button>
            <button type="button" disabled={picked.length === 0} onClick={() => void removeBooks(picked)} className="border border-stamp px-4 py-2 text-stamp disabled:border-rule disabled:text-inherit disabled:opacity-50">{t("deleteSelected")}{picked.length ? ` (${picked.length})` : ""}</button>
            <button type="button" disabled={filling} onClick={() => void fillLibrary()} className="border border-rule px-4 py-2 disabled:opacity-50">{t("fillLibrary")}</button>
            {filling ? <button type="button" onClick={() => void api.cancel()} className="px-3 py-2 text-stamp">{t("cancelJob")}</button> : null}
            <button type="button" onClick={props.onAdd} className="bg-stamp px-4 py-2 text-sheet">{t("addBook")}</button>
          </div>
        </div>
        <p className="mt-3 max-w-[52ch] text-sm text-muted">{t("fillHint")}</p>
        {fillNote ? <p className="mt-2 text-sm" role="status">{fillNote}</p> : null}
        <input id="library-search" type="search" value={props.query} onChange={(event) => props.onQuery(event.target.value)} placeholder={t("searchPlaceholder")} className="mt-6 w-full border border-rule bg-sheet px-3 py-2 dark:border-night-rule dark:bg-night" />
        <div className="mt-4 flex flex-wrap gap-2">
          {views.map(([id, label]) => (
            <button key={id || "all"} type="button" onClick={() => props.onView(id)} className={props.view === id ? "bg-binding px-3 py-1.5 text-sheet" : "border border-rule px-3 py-1.5 dark:border-night-rule"}>{label}</button>
          ))}
        </div>
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <select aria-label={t("sortLabel")} value={props.sort} onChange={(event) => props.onSort(event.target.value)} className={field}>
            <option value="title">{t("sortTitle")}</option>
            <option value="relevance">{t("sortRelevance")}</option>
            <option value="author">{t("sortAuthor")}</option>
            <option value="created">{t("sortCreated")}</option>
            <option value="rating">{t("sortRating")}</option>
            <option value="published">{t("sortPublished")}</option>
          </select>
          <select aria-label={t("anyShelf")} value={props.shelfId} onChange={(event) => props.onShelf(event.target.value)} className={field}>
            <option value="">{t("anyShelf")}</option>
            {props.shelves.map((shelf) => <option key={shelf.id} value={shelf.id}>{shelfName(shelf.name)}</option>)}
          </select>
          <button type="button" onClick={() => setShowFilters(!showFilters)} className="border border-rule px-3 py-1.5 dark:border-night-rule" aria-expanded={showFilters}>
            {t("moreFilters")}{activeFilters ? ` (${activeFilters})` : ""}
          </button>
          <span className="ml-auto flex gap-1" role="group" aria-label={t("layout")}>
            <button type="button" aria-pressed={layout === "list"} onClick={() => setLayout("list")} className={layout === "list" ? "bg-binding px-3 py-1.5 text-sheet" : "border border-rule px-3 py-1.5 dark:border-night-rule"}>{t("layoutList")}</button>
            <button type="button" aria-pressed={layout === "grid"} onClick={() => setLayout("grid")} className={layout === "grid" ? "bg-binding px-3 py-1.5 text-sheet" : "border border-rule px-3 py-1.5 dark:border-night-rule"}>{t("layoutGrid")}</button>
          </span>
        </div>
        {showFilters ? (
          <div className="mt-3 grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
            <FacetSelect label={t("author")} value={props.filters.author} options={facets?.authors ?? []} onChange={(author) => props.onFilters({ ...props.filters, author })} />
            <FacetSelect label={t("series")} value={props.filters.series} options={facets?.series ?? []} onChange={(series) => props.onFilters({ ...props.filters, series })} />
            <FacetSelect label={t("tagOrGenre")} value={props.filters.tag} options={facets?.tags ?? []} onChange={(tag) => props.onFilters({ ...props.filters, tag })} />
            <FacetSelect label={t("year")} value={props.filters.year} options={facets?.years ?? []} onChange={(year) => props.onFilters({ ...props.filters, year })} />
            {activeFilters ? <button type="button" className="w-fit text-stamp" onClick={() => props.onFilters({ author: "", series: "", tag: "", year: "" })}>{t("clearFilters")}</button> : null}
          </div>
        ) : null}
        <p className="mt-3 flex flex-wrap items-center gap-4 text-sm text-muted">
          <span>{t("shownCount")}: {props.books.length}</span>
          {props.books.length ? (
            <label className="flex items-center gap-1">
              <input type="checkbox" checked={allPicked} onChange={() => setPicked(allPicked ? [] : props.books.map((book) => book.id))} />
              {t("selectAll")}
            </label>
          ) : null}
        </p>
      </header>
      {props.books.length === 0 ? (
        <div className="py-16">
          <p className="font-display text-2xl">{props.query || activeFilters || props.view ? t("noMatches") : t("emptyCatalog")}</p>
          <p className="mt-3 max-w-[42ch] text-muted">{props.query || activeFilters || props.view ? t("noMatchesHint") : t("emptyCatalogHint")}</p>
        </div>
      ) : layout === "grid" ? (
        <ul className="mt-6 grid grid-cols-[repeat(auto-fill,minmax(130px,1fr))] gap-5">
          {props.books.map((book) => (
            <li key={book.id} className="[content-visibility:auto] [contain-intrinsic-size:auto_280px]">
              <button type="button" onClick={() => props.onOpen(book.id)} className="block w-full text-left">
                <Cover book={book} large />
                <span className="mt-2 block font-display text-lg leading-tight">{book.title}</span>
                <span className="block text-sm text-muted dark:text-night-muted">{book.authors.join(", ")}</span>
              </button>
              <span className="mt-1 flex items-center justify-between gap-1 text-xs text-muted">
                <label className="flex items-center gap-1">
                  <input type="checkbox" checked={picked.includes(book.id)} onChange={() => setPicked((current) => current.includes(book.id) ? current.filter((id) => id !== book.id) : [...current, book.id])} />
                  {t("select")}
                </label>
                <button type="button" className="text-stamp" onClick={() => void removeBooks([book.id])} aria-label={`${t("remove")}: ${book.title}`}>{t("remove")}</button>
              </span>
            </li>
          ))}
        </ul>
      ) : (
        <ul>
          {props.books.map((book) => (
            <li key={book.id} className="flex items-center gap-3 border-b border-rule [content-visibility:auto] [contain-intrinsic-size:auto_120px] dark:border-night-rule">
              <input type="checkbox" className="ml-1" checked={picked.includes(book.id)} onChange={() => setPicked((current) => current.includes(book.id) ? current.filter((id) => id !== book.id) : [...current, book.id])} aria-label={book.title} />
              <button type="button" onClick={() => props.onOpen(book.id)} className="flex min-w-0 flex-1 gap-4 py-5 text-left">
                <Cover book={book} />
                <span className="min-w-0">
                  <span className="block font-display text-2xl leading-tight">
                    {highlight(book.title, props.query).map((part, index) => part.hit ? <mark key={index} className="bg-transparent text-inherit underline decoration-stamp">{part.text}</mark> : <span key={index}>{part.text}</span>)}
                  </span>
                  <span className="mt-1 block text-muted dark:text-night-muted">
                    {book.authors.join(", ")}
                    {book.publicationYear ? `, ${book.publicationYear}` : ""}
                    {book.seriesName ? ` · ${book.seriesName}${book.seriesNumber ? ` ${book.seriesNumber}` : ""}` : ""}
                  </span>
                  <span className="mt-1 flex flex-wrap gap-2 text-sm">
                    {book.hasPaper ? <span className="text-muted">{t("filterPaper")}</span> : null}
                    {book.hasDigital ? <span className="text-muted">{t("filterEbook")}</span> : null}
                    {book.finished ? <span className="text-moss">{t("filterFinished")}</span> : null}
                    {book.loanedTo ? (
                      <span className={isOverdue(book.loanDue) ? "text-stamp" : "text-muted"}>
                        {t("loanedTo")}: {book.loanedTo}{book.loanDue ? `, ${t("returnBy")} ${book.loanDue}` : ""}
                      </span>
                    ) : null}
                  </span>
                </span>
              </button>
              <button type="button" className="shrink-0 px-3 py-2 text-sm text-stamp" onClick={() => void removeBooks([book.id])} aria-label={`${t("remove")}: ${book.title}`}>{t("remove")}</button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function isOverdue(due: string | null): boolean {
  if (!due) return false;
  const today = new Date();
  const local = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;
  return due < local;
}

function FacetSelect({ label, value, options, onChange }: { label: string; value: string; options: { name: string; count: number }[]; onChange: (value: string) => void }) {
  const { t } = useI18n();
  return (
    <label className="text-sm">
      {label}
      <select value={value} onChange={(event) => onChange(event.target.value)} className={`mt-1 block w-full ${field}`}>
        <option value="">{t("any")}</option>
        {options.map((option) => (
          <option key={option.name} value={option.name}>{option.name} ({option.count})</option>
        ))}
      </select>
    </label>
  );
}

function Cover({ book, large }: { book: BookCard; large?: boolean }) {
  const [broken, setBroken] = useState(false);
  const src = broken ? null : coverSrc(book.coverFile, book.updatedAt);
  const size = large ? "aspect-[2/3] w-full" : "h-24 w-16 shrink-0";
  return src ? (
    <img src={src} alt="" loading="lazy" decoding="async" onError={() => setBroken(true)} className={`${size} object-cover`} />
  ) : (
    <span className={`${size} flex items-end bg-binding p-1 font-display text-sm text-sheet`} aria-hidden>
      {large ? book.title : book.title.slice(0, 1)}
    </span>
  );
}
