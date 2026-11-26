import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import { toIsbn13 } from "../barcode";
import { CoverPicker, useSourceLabel } from "../CoverPicker";
import { useI18n } from "../i18n";
import { lines } from "../text";
import { errorText, type BookDraft, type MetadataHit } from "../types";

type AddViewProps = {
  onSaved: (bookId: string, created: boolean) => void;
  onBatchSaved: () => void;
  onDirty: (dirty: boolean) => void;
  onCancel: () => void;
};

type Mode = "isbn" | "search" | "scan" | "batch" | "manual";

type QueueItem = {
  isbn: string;
  status: "looking" | "found" | "missing" | "saving" | "saved" | "duplicate" | "error";
  hit?: MetadataHit;
  message?: string;
};

const emptyDraft = (): BookDraft => ({
  title: "",
  authors: [],
  translators: [],
  genres: [],
  tags: [],
});

export function draftFromHit(hit: MetadataHit): BookDraft {
  return {
    title: hit.title,
    subtitle: hit.subtitle ?? undefined,
    authors: hit.authors,
    translators: [],
    publisher: hit.publisher ?? undefined,
    publicationYear: hit.publicationYear,
    language: hit.language ?? undefined,
    description: hit.description ?? undefined,
    seriesName: hit.seriesName ?? undefined,
    genres: hit.genres,
    tags: [],
    isbn10: hit.isbn10 ?? undefined,
    isbn13: hit.isbn13 ?? undefined,
    pageCount: hit.pageCount,
    coverUrl: hit.coverUrl ?? undefined,
    metadataSource: hit.source,
    externalId: hit.externalId,
    averageRating: hit.rating,
    ratingSource: hit.rating ? hit.source : undefined,
  };
}

/** Wynik wyszukiwania pasujący do ISBN, a gdy żaden nie pasuje dokładnie, pierwszy z listy. */
function bestForIsbn(hits: MetadataHit[], isbn: string): MetadataHit | undefined {
  const wanted = toIsbn13(isbn);
  return hits.find((hit) => (hit.isbn13 && hit.isbn13 === wanted) || (hit.isbn10 && toIsbn13(hit.isbn10) === wanted)) ?? hits[0];
}

const input = "w-full border border-rule bg-sheet px-3 py-2 dark:border-night-rule dark:bg-night";

export function AddView({ onSaved, onBatchSaved, onDirty, onCancel }: AddViewProps) {
  const { t, text } = useI18n();
  const sourceLabel = useSourceLabel();
  const [mode, setMode] = useState<Mode>("isbn");
  const [isbn, setIsbn] = useState("");
  const [title, setTitle] = useState("");
  const [author, setAuthor] = useState("");
  const [hits, setHits] = useState<MetadataHit[]>([]);
  const [errors, setErrors] = useState<string[]>([]);
  const [draft, setDraft] = useState<BookDraft>(emptyDraft());
  const [authorText, setAuthorText] = useState("");
  const [coverPreview, setCoverPreview] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [batchInput, setBatchInput] = useState("");
  const [batchCamera, setBatchCamera] = useState(false);
  const videoRef = useRef<HTMLVideoElement>(null);
  const queueRef = useRef<QueueItem[]>([]);
  queueRef.current = queue;

  useEffect(() => {
    onDirty(mode === "manual" && (draft.title.trim() !== "" || authorText.trim() !== "") || queue.some((item) => item.status === "found"));
  }, [mode, draft.title, authorText, queue, onDirty]);

  useEffect(() => () => onDirty(false), [onDirty]);

  // Kamera dla pojedynczego skanu (tryb „scan”) i dla skanowania seryjnego.
  const cameraOn = mode === "scan" || (mode === "batch" && batchCamera);
  useEffect(() => {
    if (!cameraOn) return;
    const video = videoRef.current;
    if (!video) return;
    let stopped = false;
    let stop: (() => void) | null = null;
    const recent = new Map<string, number>();
    // Biblioteka skanera jest duża, więc wczytujemy ją dopiero przy włączeniu kamery.
    import("@zxing/browser")
      .then(({ BrowserMultiFormatReader }) => new BrowserMultiFormatReader().decodeFromVideoDevice(undefined, video, (result) => {
        if (!result || stopped) return;
        const code = result.getText().replace(/[^0-9Xx]/g, "");
        if (code.length !== 10 && code.length !== 13) return;
        if (mode === "scan") {
          stopped = true;
          stop?.();
          setIsbn(code);
          setMode("isbn");
          void lookup(code);
          return;
        }
        // Ten sam kod przed kamerą przez kilka klatek dodajemy tylko raz.
        const now = Date.now();
        if ((recent.get(code) ?? 0) > now - 3000) return;
        recent.set(code, now);
        addToQueue(code);
      }))
      .then((controls) => {
        stop = () => controls.stop();
        if (stopped) controls.stop();
      })
      .catch((cause: unknown) => setMessage(errorText(cause)));
    return () => {
      stopped = true;
      stop?.();
    };
  }, [cameraOn, mode]);

  function take(hit: MetadataHit) {
    setDraft(draftFromHit(hit));
    setAuthorText(hit.authors.join("\n"));
    setCoverPreview(null);
    if (hit.coverUrl) {
      api.previewCover(hit.coverUrl).then(setCoverPreview).catch(() => setCoverPreview(null));
    }
    setMode("manual");
  }

  async function lookup(nextIsbn?: string) {
    setBusy(true);
    setMessage(null);
    try {
      const result = await api.searchMetadata({
        isbn: nextIsbn ?? (isbn || undefined),
        title: mode === "search" ? title || undefined : undefined,
        author: mode === "search" ? author || undefined : undefined,
      });
      setHits(result.hits);
      setErrors(result.errors);
      if (result.hits.length === 0 && result.errors.length === 0) {
        setMessage(t("noHits"));
      }
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(false);
    }
  }

  function addToQueue(raw: string) {
    const code = raw.replace(/[^0-9Xx]/g, "").toUpperCase();
    const normalized = toIsbn13(code);
    if (!normalized) {
      setMessage(`${t("badIsbn")}: ${raw}`);
      return;
    }
    if (queueRef.current.some((item) => toIsbn13(item.isbn) === normalized)) return;
    const item: QueueItem = { isbn: normalized, status: "looking" };
    setQueue((current) => [item, ...current]);
    void api
      .searchMetadata({ isbn: normalized })
      .then((result) => {
        const hit = bestForIsbn(result.hits, normalized);
        update(normalized, hit ? { status: "found", hit } : { status: "missing", message: result.errors.join(" ") || t("noHits") });
      })
      .catch((cause) => update(normalized, { status: "error", message: errorText(cause) }));
  }

  function update(code: string, patch: Partial<QueueItem>) {
    setQueue((current) => current.map((item) => (item.isbn === code ? { ...item, ...patch } : item)));
  }

  async function saveQueue() {
    setBusy(true);
    let saved = 0;
    for (const item of queueRef.current.filter((entry) => entry.status === "found" && entry.hit)) {
      update(item.isbn, { status: "saving" });
      try {
        const result = await api.saveBook(draftFromHit(item.hit as MetadataHit));
        update(item.isbn, { status: result.created ? "saved" : "duplicate" });
        if (result.created) saved += 1;
      } catch (cause) {
        update(item.isbn, { status: "error", message: errorText(cause) });
      }
    }
    setBusy(false);
    setMessage(`${t("batchSaved")}: ${saved}`);
    onBatchSaved();
  }

  async function save() {
    setBusy(true);
    setMessage(null);
    try {
      const result = await api.saveBook({ ...draft, authors: lines(authorText), title: draft.title.trim() });
      onDirty(false);
      onSaved(result.bookId, result.created);
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(false);
    }
  }

  const modes: [Mode, string][] = [
    ["isbn", "ISBN"],
    ["search", t("modeTitle")],
    ["scan", t("modeScan")],
    ["batch", t("modeBatch")],
    ["manual", t("modeManual")],
  ];
  const found = queue.filter((item) => item.status === "found").length;

  return (
    <section>
      <header className="flex items-start justify-between gap-4 border-b border-rule pb-6 dark:border-night-rule">
        <div>
          <h1 className="font-display text-5xl leading-none">{t("addTitle")}</h1>
          <p className="mt-3 max-w-[48ch] text-muted dark:text-night-muted">{t("addLead")}</p>
        </div>
        <button type="button" onClick={onCancel} className="px-3 py-2 text-muted">{t("cancel")}</button>
      </header>
      <div className="mt-6 flex flex-wrap gap-2">
        {modes.map(([id, label]) => (
          <button key={id} type="button" onClick={() => { setMessage(null); setMode(id); }} className={mode === id ? "bg-binding px-3 py-1.5 text-sheet" : "border border-rule px-3 py-1.5 dark:border-night-rule"}>
            {label}
          </button>
        ))}
      </div>
      {mode === "isbn" ? (
        <form className="mt-6 flex gap-2" onSubmit={(event) => { event.preventDefault(); void lookup(); }}>
          <input value={isbn} onChange={(event) => setIsbn(event.target.value)} placeholder={t("isbnPlaceholder")} className={input} autoFocus />
          <button className="bg-binding px-4 py-2 text-sheet" disabled={busy}>{t("search")}</button>
        </form>
      ) : null}
      {mode === "search" ? (
        <form className="mt-6 grid gap-3" onSubmit={(event) => { event.preventDefault(); void lookup(""); }}>
          <input value={title} onChange={(event) => setTitle(event.target.value)} placeholder={t("modeTitle")} className={input} autoFocus />
          <input value={author} onChange={(event) => setAuthor(event.target.value)} placeholder={t("author")} className={input} />
          <button className="w-fit bg-binding px-4 py-2 text-sheet" disabled={busy}>{t("search")}</button>
        </form>
      ) : null}
      {mode === "batch" ? (
        <div className="mt-6">
          <p className="max-w-[60ch] text-muted">{t("batchLead")}</p>
          <form className="mt-4 flex gap-2" onSubmit={(event) => { event.preventDefault(); batchInput.split(/[\s,;]+/).filter(Boolean).forEach(addToQueue); setBatchInput(""); }}>
            <input value={batchInput} onChange={(event) => setBatchInput(event.target.value)} placeholder={t("batchPlaceholder")} className={input} autoFocus />
            <button className="bg-binding px-4 py-2 text-sheet">{t("add")}</button>
          </form>
          <label className="mt-3 flex items-center gap-2">
            <input type="checkbox" checked={batchCamera} onChange={(event) => setBatchCamera(event.target.checked)} /> {t("batchCamera")}
          </label>
        </div>
      ) : null}
      {cameraOn ? <video ref={videoRef} className="mt-6 max-h-72 w-full bg-binding" muted playsInline /> : null}
      {message ? <p className="mt-4 text-stamp" role="alert">{text(message)}</p> : null}
      {mode === "batch" && queue.length > 0 ? (
        <div className="mt-6">
          <div className="flex flex-wrap items-center gap-3">
            <button type="button" disabled={busy || found === 0} onClick={() => void saveQueue()} className="bg-stamp px-4 py-2 text-sheet disabled:opacity-50">
              {t("batchSaveAll")} ({found})
            </button>
            <button type="button" className="text-muted" onClick={() => setQueue((current) => current.filter((item) => item.status === "found" || item.status === "looking"))}>{t("batchClearDone")}</button>
          </div>
          <ul className="mt-4">
            {queue.map((item) => (
              <li key={item.isbn} className="flex items-center gap-3 border-b border-rule py-3 dark:border-night-rule">
                <span className="w-36 shrink-0 font-mono text-sm">{item.isbn}</span>
                <span className="min-w-0 flex-1">
                  {item.hit ? <span className="font-display text-lg">{item.hit.title}</span> : null}
                  {item.hit ? <span className="block text-sm text-muted">{item.hit.authors.join(", ")} · {sourceLabel(item.hit.source)}</span> : null}
                  {item.message ? <span className="block text-sm text-muted">{text(item.message)}</span> : null}
                </span>
                <span className={item.status === "error" || item.status === "missing" ? "text-stamp" : item.status === "saved" ? "text-moss" : "text-muted"}>{t(`queue_${item.status}` as const)}</span>
                {item.status !== "saving" ? (
                  <button type="button" aria-label={t("remove")} className="text-muted" onClick={() => setQueue((current) => current.filter((entry) => entry.isbn !== item.isbn))}>×</button>
                ) : null}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {mode !== "batch" ? errors.map((error) => <p key={error} className="mt-2 text-sm text-muted">{text(error)}</p>) : null}
      {hits.length > 0 && mode !== "manual" && mode !== "batch" ? (
        <ul className="mt-6">
          {hits.map((hit) => (
            <li key={`${hit.source}-${hit.externalId}-${hit.isbn13}`} className="border-b border-rule py-4 dark:border-night-rule">
              <button type="button" onClick={() => take(hit)} className="text-left">
                <span className="font-display text-2xl">{hit.title}</span>
                <span className="mt-1 block text-muted">{hit.authors.join(", ")}{hit.publicationYear ? `, ${hit.publicationYear}` : ""} · {sourceLabel(hit.source)}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      {mode === "manual" ? (
        <form className="mt-6 grid max-w-xl gap-3" onSubmit={(event) => { event.preventDefault(); void save(); }}>
          <label>{t("modeTitle")}<input value={draft.title} onChange={(event) => setDraft({ ...draft, title: event.target.value })} className={`mt-1 ${input}`} /></label>
          <label>{t("authorsLines")}<textarea value={authorText} onChange={(event) => setAuthorText(event.target.value)} rows={4} className={`mt-1 ${input}`} /></label>
          <label>ISBN-13<input value={draft.isbn13 ?? ""} onChange={(event) => setDraft({ ...draft, isbn13: event.target.value })} className={`mt-1 ${input}`} /></label>
          <label>{t("publisher")}<input value={draft.publisher ?? ""} onChange={(event) => setDraft({ ...draft, publisher: event.target.value })} className={`mt-1 ${input}`} /></label>
          <label>{t("year")}<input inputMode="numeric" value={draft.publicationYear ?? ""} onChange={(event) => { const value = Number(event.target.value); setDraft({ ...draft, publicationYear: event.target.value && Number.isInteger(value) ? value : null }); }} className={`mt-1 ${input}`} /></label>
          <label>{t("series")}<input value={draft.seriesName ?? ""} onChange={(event) => setDraft({ ...draft, seriesName: event.target.value })} className={`mt-1 ${input}`} /></label>
          <label>{t("description")}<textarea value={draft.description ?? ""} onChange={(event) => setDraft({ ...draft, description: event.target.value })} rows={5} className={`mt-1 ${input}`} /></label>
          {coverPreview ? <img src={coverPreview} alt="" className="h-40 w-28 object-cover" /> : null}
          <CoverPicker title={draft.title} author={authorText.split("\n")[0] ?? ""} isbn={draft.isbn13} onPick={(url) => { setDraft({ ...draft, coverUrl: url }); void api.previewCover(url).then(setCoverPreview).catch((cause) => setMessage(errorText(cause))); }} />
          <p className="text-sm text-muted">{t("fillHint")}</p>
          <button className="w-fit bg-stamp px-4 py-2 text-sheet" disabled={busy}>{busy ? t("saving") : t("saveBook")}</button>
        </form>
      ) : null}
    </section>
  );
}
