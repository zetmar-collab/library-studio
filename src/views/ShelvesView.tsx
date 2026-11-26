import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { useI18n } from "../i18n";
import { fold } from "../text";
import { useUi } from "../ui";
import { errorText, type BookCard, type Shelf } from "../types";

const DRAG_TYPE = "text/book-id";

export function ShelvesView({ shelves, onChanged }: { shelves: Shelf[]; onChanged: () => void }) {
  const { t, shelfName } = useI18n();
  const { confirm, notify } = useUi();
  const [name, setName] = useState("");
  const [smart, setSmart] = useState(false);
  const [filter, setFilter] = useState("unread");
  const [filterValue, setFilterValue] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [active, setActive] = useState(shelves[0]?.id ?? "");
  const [visible, setVisible] = useState<BookCard[]>([]);
  const [catalog, setCatalog] = useState<BookCard[]>([]);
  const [search, setSearch] = useState("");
  const [dropTarget, setDropTarget] = useState<string | null>(null);

  const shelf = shelves.find((item) => item.id === active) ?? shelves[0];

  const loadShelf = useCallback(async () => {
    if (!shelf) {
      setVisible([]);
      return;
    }
    setVisible(await api.listBooks({ shelfId: shelf.id, sort: shelf.isSmartShelf ? "title" : "shelf" }));
  }, [shelf]);

  useEffect(() => {
    loadShelf().catch((cause) => setMessage(errorText(cause)));
  }, [loadShelf]);

  useEffect(() => {
    api.listBooks({ sort: "title" }).then(setCatalog).catch((cause) => setMessage(errorText(cause)));
  }, [shelves]);

  const onShelf = useMemo(() => new Set(visible.map((book) => book.id)), [visible]);
  const candidates = useMemo(() => {
    const needle = fold(search);
    return catalog
      .filter((book) => !onShelf.has(book.id))
      .filter((book) => !needle || fold(`${book.title} ${book.authors.join(" ")}`).includes(needle))
      .slice(0, 200);
  }, [catalog, onShelf, search]);

  async function create() {
    try {
      let smartFilter: string | undefined;
      if (smart) {
        const value =
          filter === "loaned" ? { loaned: true } :
          filter === "digital" ? { digital: true } :
          filter === "author" ? { author: filterValue.trim() } :
          filter === "tag" ? { tag: filterValue.trim() } :
          { unread: true };
        if ((filter === "author" || filter === "tag") && !filterValue.trim()) {
          setMessage(t("filterValueNeeded"));
          return;
        }
        smartFilter = JSON.stringify(value);
      }
      const id = await api.saveShelf({ name, isSmartShelf: smart, smartFilterJson: smartFilter });
      setName("");
      setFilterValue("");
      setActive(id);
      onChanged();
    } catch (cause) {
      setMessage(errorText(cause));
    }
  }

  async function persist(order: string[]) {
    if (!shelf || shelf.isSmartShelf) return;
    try {
      await api.reorderShelf(shelf.id, order);
      await loadShelf();
      onChanged();
    } catch (cause) {
      setMessage(errorText(cause));
    }
  }

  /** Upuszczenie książki przed wskazaną pozycję (albo na koniec półki). */
  function drop(bookId: string, beforeId: string | null) {
    const order = visible.map((book) => book.id).filter((id) => id !== bookId);
    const index = beforeId ? order.indexOf(beforeId) : -1;
    if (index < 0) order.push(bookId);
    else order.splice(index, 0, bookId);
    void persist(order);
  }

  function move(bookId: string, delta: number) {
    const order = visible.map((book) => book.id);
    const index = order.indexOf(bookId);
    const target = index + delta;
    if (index < 0 || target < 0 || target >= order.length) return;
    [order[index], order[target]] = [order[target], order[index]];
    void persist(order);
  }

  async function removeShelf() {
    if (!shelf) return;
    const ok = await confirm(t("deleteShelfConfirm").replace("{name}", shelfName(shelf.name)), { danger: true, confirmLabel: t("deleteShelf") });
    if (!ok) return;
    try {
      await api.deleteShelf(shelf.id);
      notify(t("shelfDeleted"));
      setActive("");
      onChanged();
    } catch (cause) {
      setMessage(errorText(cause));
    }
  }

  const manual = shelf && !shelf.isSmartShelf;

  return (
    <section>
      <h1 className="font-display text-5xl leading-none">{t("shelves")}</h1>
      <div className="mt-6 flex flex-wrap gap-2">
        {shelves.map((item) => (
          <button key={item.id} type="button" onClick={() => setActive(item.id)} className={item.id === shelf?.id ? "bg-binding px-3 py-1.5 text-sheet" : "border border-rule px-3 py-1.5 dark:border-night-rule"}>
            {shelfName(item.name)}{item.isSmartShelf ? " ⚙" : ` (${item.bookCount})`}
          </button>
        ))}
      </div>
      <form className="mt-6 flex flex-wrap items-center gap-2" onSubmit={(event) => { event.preventDefault(); void create(); }}>
        <input value={name} onChange={(event) => setName(event.target.value)} placeholder={t("newShelf")} aria-label={t("newShelf")} className="border border-rule bg-sheet px-3 py-2 dark:border-night-rule dark:bg-night" />
        <label className="flex items-center gap-2"><input type="checkbox" checked={smart} onChange={(event) => setSmart(event.target.checked)} /> {t("withFilter")}</label>
        {smart ? (
          <>
            <select value={filter} onChange={(event) => setFilter(event.target.value)} aria-label={t("withFilter")} className="border border-rule bg-sheet px-2 py-2 dark:border-night-rule dark:bg-night">
              <option value="unread">{t("unread")}</option>
              <option value="loaned">{t("filterLoaned")}</option>
              <option value="digital">{t("filterEbook")}</option>
              <option value="author">{t("author")}</option>
              <option value="tag">{t("tagOrGenre")}</option>
            </select>
            {filter === "author" || filter === "tag" ? (
              <input value={filterValue} onChange={(event) => setFilterValue(event.target.value)} placeholder={filter === "author" ? t("author") : t("tagOrGenre")} className="border border-rule bg-sheet px-3 py-2 dark:border-night-rule dark:bg-night" />
            ) : null}
          </>
        ) : null}
        <button className="bg-stamp px-4 py-2 text-sheet">{t("create")}</button>
      </form>
      {shelf ? (
        <div className={manual ? "mt-8 grid gap-8 lg:grid-cols-2" : "mt-8"}>
          <div
            onDragOver={(event) => { if (manual) { event.preventDefault(); setDropTarget("end"); } }}
            onDragLeave={() => setDropTarget(null)}
            onDrop={(event) => { event.preventDefault(); setDropTarget(null); const id = event.dataTransfer.getData(DRAG_TYPE); if (id && manual) drop(id, null); }}
            className={dropTarget === "end" ? "outline outline-2 outline-stamp" : undefined}
          >
            <div className="flex items-center justify-between gap-4">
              <h2 className="font-display text-3xl">{shelfName(shelf.name)}</h2>
              <button type="button" className="text-stamp" onClick={() => void removeShelf()}>{t("deleteShelf")}</button>
            </div>
            <p className="mt-1 text-sm text-muted">{manual ? t("shelfDragHint") : t("smartShelfHint")}</p>
            <ol className="mt-4">
              {visible.map((book, index) => (
                <li
                  key={book.id}
                  draggable={Boolean(manual)}
                  onDragStart={(event) => event.dataTransfer.setData(DRAG_TYPE, book.id)}
                  onDragOver={(event) => { if (manual) { event.preventDefault(); event.stopPropagation(); setDropTarget(book.id); } }}
                  onDrop={(event) => { event.preventDefault(); event.stopPropagation(); setDropTarget(null); const id = event.dataTransfer.getData(DRAG_TYPE); if (id && manual && id !== book.id) drop(id, book.id); }}
                  className={`flex items-center gap-2 border-b border-rule py-3 dark:border-night-rule ${dropTarget === book.id ? "border-t-2 border-t-stamp" : ""} ${manual ? "cursor-grab" : ""}`}
                >
                  <span className="flex-1">{book.title}<span className="block text-sm text-muted">{book.authors.join(", ")}</span></span>
                  {manual ? (
                    <>
                      <button type="button" aria-label={t("moveUp")} disabled={index === 0} onClick={() => move(book.id, -1)} className="px-2 disabled:opacity-30">↑</button>
                      <button type="button" aria-label={t("moveDown")} disabled={index === visible.length - 1} onClick={() => move(book.id, 1)} className="px-2 disabled:opacity-30">↓</button>
                      <button type="button" aria-label={t("removeFromShelf")} onClick={() => void api.removeFromShelf(shelf.id, book.id).then(loadShelf).then(onChanged).catch((cause) => setMessage(errorText(cause)))} className="px-2 text-stamp">×</button>
                    </>
                  ) : null}
                </li>
              ))}
            </ol>
            {visible.length === 0 ? <p className="mt-4 text-muted">{t("shelfEmpty")}</p> : null}
          </div>
          {manual ? (
            <div>
              <h2 className="font-display text-3xl">{t("catalog")}</h2>
              <input value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t("searchPlaceholder")} aria-label={t("searchPlaceholder")} className="mt-3 w-full border border-rule bg-sheet px-3 py-2 dark:border-night-rule dark:bg-night" />
              <ul className="mt-3 max-h-[60vh] overflow-auto">
                {candidates.map((book) => (
                  <li key={book.id} draggable onDragStart={(event) => event.dataTransfer.setData(DRAG_TYPE, book.id)} className="flex cursor-grab items-center gap-2 border-b border-rule py-2 dark:border-night-rule">
                    <span className="flex-1">{book.title}<span className="block text-sm text-muted">{book.authors.join(", ")}</span></span>
                    <button type="button" onClick={() => drop(book.id, null)} className="border border-rule px-2 py-1 text-sm dark:border-night-rule">{t("addToShelf")}</button>
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
        </div>
      ) : null}
      {message ? <p className="mt-4 text-stamp" role="alert">{message}</p> : null}
    </section>
  );
}
