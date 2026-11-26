import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { api } from "../api";
import { CoverPicker } from "../CoverPicker";
import { useI18n } from "../i18n";
import { lines } from "../text";
import { useUi } from "../ui";
import { errorText, type BookDetails, type Shelf } from "../types";
import { isOverdue } from "./LibraryView";

type Form = {
  title: string;
  subtitle: string;
  originalTitle: string;
  authors: string;
  translators: string;
  publisher: string;
  year: string;
  language: string;
  seriesName: string;
  seriesNumber: string;
  genres: string;
  tags: string;
  isbn13: string;
  isbn10: string;
  pageCount: string;
  description: string;
};

type Copy = { shelfId: string; location: string; loanedTo: string; loanedDate: string; returnDueDate: string };

function toForm(details: BookDetails): Form {
  const book = details.book;
  const edition = details.editions[0];
  return {
    title: book.title,
    subtitle: book.subtitle ?? "",
    originalTitle: book.originalTitle ?? "",
    authors: book.authors.join("\n"),
    translators: book.translators.join("\n"),
    publisher: book.publisher ?? "",
    year: book.publicationYear?.toString() ?? "",
    language: book.language ?? "",
    seriesName: book.seriesName ?? "",
    seriesNumber: book.seriesNumber ?? "",
    genres: book.genres.join("\n"),
    tags: book.tags.join("\n"),
    isbn13: edition?.isbn13 ?? "",
    isbn10: edition?.isbn10 ?? "",
    pageCount: edition?.pageCount?.toString() ?? "",
    description: book.description ?? "",
  };
}

function toCopy(details: BookDetails): Copy {
  const copy = details.physical[0];
  return {
    shelfId: copy?.shelfId ?? "",
    location: copy?.locationNote ?? "",
    loanedTo: copy?.loanedTo ?? "",
    loanedDate: copy?.loanedDate ?? "",
    returnDueDate: copy?.returnDueDate ?? "",
  };
}

function today(offsetDays = 0): string {
  const date = new Date();
  date.setDate(date.getDate() + offsetDays);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

const input = "mt-1 w-full border border-rule bg-sheet px-3 py-2 dark:border-night-rule dark:bg-night";

type BookViewProps = {
  id: string;
  shelves: Shelf[];
  onBack: () => void;
  onChanged: () => void;
  onDirty: (dirty: boolean) => void;
  onDeleted: () => void;
  onSend: (id: string) => void;
};

export function BookView({ id, shelves, onBack, onChanged, onDirty, onDeleted, onSend }: BookViewProps) {
  const { t, shelfName } = useI18n();
  const { confirm, notify } = useUi();
  const [details, setDetails] = useState<BookDetails | null>(null);
  const [cover, setCover] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [form, setForm] = useState<Form | null>(null);
  const [copy, setCopy] = useState<Copy | null>(null);
  const [rating, setRating] = useState("");
  const [notes, setNotes] = useState("");
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    const next = await api.getBook(id);
    setDetails(next);
    setForm(toForm(next));
    setCopy(toCopy(next));
    setRating(next.progress[0]?.rating?.toString() ?? "");
    setNotes(next.progress[0]?.notes ?? "");
    setCover(await api.cover(id).catch(() => null));
  }, [id]);

  useEffect(() => {
    let alive = true;
    setDetails(null);
    load().catch((cause) => alive && setMessage(errorText(cause)));
    return () => {
      alive = false;
    };
  }, [load]);

  const dirty = useMemo(() => {
    if (!details || !form || !copy) return false;
    const progress = details.progress[0];
    return (
      JSON.stringify(form) !== JSON.stringify(toForm(details)) ||
      JSON.stringify(copy) !== JSON.stringify(toCopy(details)) ||
      rating !== (progress?.rating?.toString() ?? "") ||
      notes !== (progress?.notes ?? "")
    );
  }, [details, form, copy, rating, notes]);

  useEffect(() => {
    onDirty(dirty);
    return () => onDirty(false);
  }, [dirty, onDirty]);

  if (!details || !form || !copy) return <p>{message ?? t("loadingBook")}</p>;
  const current = details;
  const edition = current.editions[0];
  const set = (patch: Partial<Form>) => setForm({ ...form, ...patch });
  const manualShelves = shelves.filter((shelf) => !shelf.isSmartShelf);
  const memberOf = new Set(current.shelves.map((shelf) => shelf.id));

  async function act(work: () => Promise<unknown>, done?: string) {
    setMessage(null);
    setBusy(true);
    try {
      await work();
      await load();
      onChanged();
      if (done) setMessage(done);
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(false);
    }
  }

  function saveMeta() {
    if (!form) return;
    const year = form.year.trim() ? Number(form.year) : null;
    const pages = form.pageCount.trim() ? Number(form.pageCount) : null;
    if ((year !== null && !Number.isInteger(year)) || (pages !== null && !Number.isInteger(pages))) {
      setMessage(t("numberExpected"));
      return;
    }
    void act(
      () =>
        api.saveBook({
          id: current.book.id,
          editionId: edition?.id,
          title: form.title.trim(),
          subtitle: form.subtitle,
          originalTitle: form.originalTitle,
          authors: lines(form.authors),
          translators: lines(form.translators),
          publisher: form.publisher,
          publicationYear: year,
          language: form.language,
          description: form.description,
          seriesName: form.seriesName,
          seriesNumber: form.seriesNumber,
          genres: lines(form.genres),
          tags: lines(form.tags),
          isbn10: form.isbn10,
          isbn13: form.isbn13,
          pageCount: pages,
          metadataSource: edition?.metadataSource ?? "manual",
        }),
      t("saved"),
    );
  }

  function saveCopy(next: Copy) {
    void act(
      () =>
        api.savePhysical({
          id: current.physical[0]?.id,
          bookId: current.book.id,
          shelfId: next.shelfId || null,
          locationNote: next.location || null,
          loanedTo: next.loanedTo || null,
          loanedDate: next.loanedTo ? next.loanedDate || today() : null,
          returnDueDate: next.loanedTo ? next.returnDueDate || null : null,
        }),
      t("saved"),
    );
  }

  async function remove() {
    const ok = await confirm(t("deleteBookConfirm").replace("{title}", current.book.title), { danger: true, confirmLabel: t("moveToTrash") });
    if (!ok) return;
    try {
      await api.deleteBook(current.book.id);
      notify(t("movedToTrash"), {
        label: t("undo"),
        run: () => {
          void api.restoreBook(current.book.id).then(onChanged).catch((cause) => notify(errorText(cause)));
        },
      });
      onDeleted();
    } catch (cause) {
      setMessage(errorText(cause));
    }
  }

  const progress = current.progress[0];

  return (
    <section>
      <button type="button" onClick={onBack} className="text-muted">{t("backToCatalog")}</button>
      <div className="mt-4 flex flex-col gap-6 sm:flex-row">
        {cover ? <img src={cover} alt="" className="h-56 w-36 shrink-0 object-cover" /> : <div className="h-56 w-36 shrink-0 bg-binding" />}
        <div className="min-w-0 flex-1">
          <label className="block text-sm text-muted">
            {t("modeTitle")}
            <input value={form.title} onChange={(event) => set({ title: event.target.value })} className="block w-full bg-transparent font-display text-4xl text-ink dark:text-night-ink" />
          </label>
          <p className="mt-2 text-sm text-muted">
            ISBN {edition?.isbn13 ?? edition?.isbn10 ?? t("isbnMissing")} · {t("sourcePrefix")} {edition?.metadataSource ?? t("sourceManual")}
          </p>
          <button
            type="button"
            disabled={busy}
            className="mt-3 border border-rule px-3 py-1.5 disabled:opacity-50"
            onClick={() =>
              void act(async () => {
                setMessage(t("fillWorking"));
                const report = await api.enrichBook(current.book.id);
                notify(report.filled ? t("fillDone") : t("fillNone"));
              })
            }
          >
            {t("fillBook")}
          </button>
          <p className="mt-2 text-sm text-muted">{t("fillHint")}</p>
          <CoverPicker
            title={form.title}
            author={lines(form.authors)[0] ?? ""}
            isbn={form.isbn13 || form.isbn10 || undefined}
            onPick={(url) => void act(() => api.attachCover(current.book.id, url), t("coverSaved"))}
          />
        </div>
      </div>

      <form className="mt-8 grid gap-4 sm:grid-cols-2" onSubmit={(event) => { event.preventDefault(); saveMeta(); }}>
        <Field label={t("authorsLines")}><textarea value={form.authors} onChange={(event) => set({ authors: event.target.value })} rows={3} className={input} /></Field>
        <Field label={t("translatorsLines")}><textarea value={form.translators} onChange={(event) => set({ translators: event.target.value })} rows={3} className={input} /></Field>
        <Field label={t("subtitle")}><input value={form.subtitle} onChange={(event) => set({ subtitle: event.target.value })} className={input} /></Field>
        <Field label={t("originalTitle")}><input value={form.originalTitle} onChange={(event) => set({ originalTitle: event.target.value })} className={input} /></Field>
        <Field label={t("publisher")}><input value={form.publisher} onChange={(event) => set({ publisher: event.target.value })} className={input} /></Field>
        <div className="grid grid-cols-2 gap-4">
          <Field label={t("year")}><input inputMode="numeric" value={form.year} onChange={(event) => set({ year: event.target.value })} className={input} /></Field>
          <Field label={t("pages")}><input inputMode="numeric" value={form.pageCount} onChange={(event) => set({ pageCount: event.target.value })} className={input} /></Field>
        </div>
        <Field label={t("series")}><input value={form.seriesName} onChange={(event) => set({ seriesName: event.target.value })} className={input} /></Field>
        <div className="grid grid-cols-2 gap-4">
          <Field label={t("seriesNumber")}><input value={form.seriesNumber} onChange={(event) => set({ seriesNumber: event.target.value })} className={input} /></Field>
          <Field label={t("languageField")}><input value={form.language} onChange={(event) => set({ language: event.target.value })} className={input} /></Field>
        </div>
        <Field label="ISBN-13"><input value={form.isbn13} onChange={(event) => set({ isbn13: event.target.value })} className={input} /></Field>
        <Field label="ISBN-10"><input value={form.isbn10} onChange={(event) => set({ isbn10: event.target.value })} className={input} /></Field>
        <Field label={t("genresLines")}><textarea value={form.genres} onChange={(event) => set({ genres: event.target.value })} rows={3} className={input} /></Field>
        <Field label={t("tagsLines")}><textarea value={form.tags} onChange={(event) => set({ tags: event.target.value })} rows={3} className={input} /></Field>
        <div className="sm:col-span-2">
          <Field label={t("description")}><textarea value={form.description} onChange={(event) => set({ description: event.target.value })} rows={6} className={input} /></Field>
        </div>
        <div className="flex items-center gap-3 sm:col-span-2">
          <button type="submit" disabled={busy} className="bg-binding px-4 py-2 text-sheet disabled:opacity-50">{t("saveDetails")}</button>
          {dirty ? <span className="text-sm text-stamp">{t("unsavedChanges")}</span> : null}
        </div>
      </form>

      <h2 className="mt-10 font-display text-3xl">{t("physicalCopy")}</h2>
      <div className="mt-3 grid max-w-2xl gap-3 sm:grid-cols-2">
        <Field label={t("shelfForCopy")}>
          <select value={copy.shelfId} onChange={(event) => setCopy({ ...copy, shelfId: event.target.value })} className={input}>
            <option value="">{t("noShelf")}</option>
            {manualShelves.map((shelf) => <option key={shelf.id} value={shelf.id}>{shelfName(shelf.name)}</option>)}
          </select>
        </Field>
        <Field label={t("location")}><input value={copy.location} onChange={(event) => setCopy({ ...copy, location: event.target.value })} placeholder={t("locationPlaceholder")} className={input} /></Field>
        <Field label={t("loanedPerson")}><input value={copy.loanedTo} onChange={(event) => setCopy({ ...copy, loanedTo: event.target.value, loanedDate: copy.loanedDate || (event.target.value ? today() : ""), returnDueDate: copy.returnDueDate || (event.target.value ? today(30) : "") })} className={input} /></Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("loanedDate")}><input type="date" value={copy.loanedDate} disabled={!copy.loanedTo} onChange={(event) => setCopy({ ...copy, loanedDate: event.target.value })} className={input} /></Field>
          <Field label={t("returnDue")}><input type="date" value={copy.returnDueDate} disabled={!copy.loanedTo} onChange={(event) => setCopy({ ...copy, returnDueDate: event.target.value })} className={input} /></Field>
        </div>
        {copy.loanedTo && isOverdue(copy.returnDueDate || null) ? <p className="text-stamp sm:col-span-2" role="status">{t("overdueOne")}</p> : null}
        <div className="flex flex-wrap gap-2 sm:col-span-2">
          <button type="button" disabled={busy} onClick={() => saveCopy(copy)} className="bg-binding px-4 py-2 text-sheet disabled:opacity-50">{t("saveCopy")}</button>
          {current.physical[0]?.loanedTo ? (
            <button type="button" disabled={busy} onClick={() => saveCopy({ ...copy, loanedTo: "", loanedDate: "", returnDueDate: "" })} className="border border-rule px-4 py-2">{t("markReturned")}</button>
          ) : null}
          {current.physical[0] ? (
            <button
              type="button"
              className="px-4 py-2 text-stamp"
              onClick={() =>
                void confirm(t("deleteCopyConfirm"), { danger: true }).then((ok) => {
                  if (ok) void act(() => api.deletePhysical(current.physical[0].id));
                })
              }
            >
              {t("deleteCopy")}
            </button>
          ) : null}
        </div>
      </div>

      {manualShelves.length > 0 ? (
        <>
          <h2 className="mt-10 font-display text-3xl">{t("shelves")}</h2>
          <ul className="mt-3 flex flex-wrap gap-3">
            {manualShelves.map((shelf) => (
              <li key={shelf.id}>
                <label className="flex items-center gap-2 border border-rule px-3 py-1.5 dark:border-night-rule">
                  <input
                    type="checkbox"
                    checked={memberOf.has(shelf.id)}
                    onChange={(event) =>
                      void act(() => (event.target.checked ? api.addToShelf(shelf.id, current.book.id) : api.removeFromShelf(shelf.id, current.book.id)))
                    }
                  />
                  {shelfName(shelf.name)}
                </label>
              </li>
            ))}
          </ul>
        </>
      ) : null}

      <h2 className="mt-10 font-display text-3xl">{t("files")}</h2>
      <ul className="mt-3">
        {current.digital.map((file) => (
          <li key={file.id} className="flex items-center justify-between border-b border-rule py-3 dark:border-night-rule">
            <span>{file.fileName}</span>
            <button type="button" onClick={() => void api.openFile(file.id).catch((cause) => setMessage(errorText(cause)))} className="text-moss">{t("open")}</button>
          </li>
        ))}
      </ul>
      <button
        type="button"
        onClick={() =>
          void (async () => {
            const path = await api.pickFile();
            if (path) await act(() => api.addFile(current.book.id, path), t("fileAdded"));
          })()
        }
        className="mt-3 bg-stamp px-4 py-2 text-sheet"
      >
        {t("addFile")}
      </button>
      <button type="button" onClick={() => onSend(current.book.id)} className="ml-2 border border-rule px-4 py-2">{t("sendThisBook")}</button>

      <h2 className="mt-10 font-display text-3xl">{t("reading")}</h2>
      <div className="mt-3 grid max-w-xl gap-3">
        {progress?.finishedAt ? <p className="text-moss">{t("finishedOn")} {progress.finishedAt}</p> : null}
        <Field label={t("ratingLabel")}>
          <select value={rating} onChange={(event) => setRating(event.target.value)} className={input}>
            <option value="">{t("noRating")}</option>
            {[1, 2, 3, 4, 5].map((value) => <option key={value} value={value}>{"★".repeat(value)}</option>)}
          </select>
        </Field>
        <textarea value={notes} onChange={(event) => setNotes(event.target.value)} rows={4} placeholder={t("notes")} aria-label={t("notes")} className={input} />
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={() => void act(() => api.saveProgress({ id: progress?.id, bookId: current.book.id, rating: rating ? Number(rating) : null, notes: notes || null, finishedAt: progress?.finishedAt ?? null }), t("saved"))}
            className="w-fit bg-binding px-4 py-2 text-sheet disabled:opacity-50"
          >
            {t("saveNote")}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => void act(() => api.saveProgress({ id: progress?.id, bookId: current.book.id, finishedAt: today(), progressPercent: 100, rating: rating ? Number(rating) : null, notes: notes || null }), t("saved"))}
            className="w-fit border border-rule px-4 py-2"
          >
            {t("markFinished")}
          </button>
          {progress?.finishedAt ? (
            <button type="button" disabled={busy} onClick={() => void act(() => api.saveProgress({ id: progress.id, bookId: current.book.id, finishedAt: null, rating: rating ? Number(rating) : null, notes: notes || null }))} className="w-fit px-4 py-2 text-muted">
              {t("markUnread")}
            </button>
          ) : null}
        </div>
      </div>
      {message ? <p className="mt-4 text-stamp" role="status">{message}</p> : null}
      <button type="button" className="mt-8 text-stamp" onClick={() => void remove()}>{t("deleteFromCatalog")}</button>
    </section>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="block text-sm">
      <span className="text-muted dark:text-night-muted">{label}</span>
      {children}
    </label>
  );
}
