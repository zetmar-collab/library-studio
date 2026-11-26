import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { ean13Modules } from "../barcode";
import { useI18n } from "../i18n";
import { useUi } from "../ui";
import { errorText, type DuplicateGroup, type ExportRow, type FileRepair, type ImportRow, type JobProgress, type TrashRow } from "../types";

type Tab = "trash" | "repair" | "duplicates" | "import" | "print";

export function ToolsView({ progress, onChanged, onOpen }: { progress: JobProgress | null; onChanged: () => void; onOpen: (id: string) => void }) {
  const { t } = useI18n();
  const [tab, setTab] = useState<Tab>("trash");
  const tabs: [Tab, string][] = [
    ["trash", t("trashTitle")],
    ["repair", t("repairTitle")],
    ["duplicates", t("duplicatesTitle")],
    ["import", t("importTitle")],
    ["print", t("printTitle")],
  ];
  return (
    <section>
      <h1 className="font-display text-5xl leading-none print:hidden">{t("toolsTitle")}</h1>
      <div className="mt-6 flex flex-wrap gap-2 print:hidden" role="tablist">
        {tabs.map(([id, label]) => (
          <button key={id} type="button" role="tab" aria-selected={tab === id} onClick={() => setTab(id)} className={tab === id ? "bg-binding px-3 py-1.5 text-sheet" : "border border-rule px-3 py-1.5 dark:border-night-rule"}>
            {label}
          </button>
        ))}
      </div>
      <div className="mt-8 print:mt-0">
        {tab === "trash" ? <TrashTab onChanged={onChanged} /> : null}
        {tab === "repair" ? <RepairTab onChanged={onChanged} /> : null}
        {tab === "duplicates" ? <DuplicatesTab onChanged={onChanged} onOpen={onOpen} /> : null}
        {tab === "import" ? <ImportTab progress={progress} onChanged={onChanged} /> : null}
        {tab === "print" ? <PrintTab /> : null}
      </div>
    </section>
  );
}

function TrashTab({ onChanged }: { onChanged: () => void }) {
  const { t } = useI18n();
  const { confirm, notify } = useUi();
  const [rows, setRows] = useState<TrashRow[]>([]);
  const [message, setMessage] = useState<string | null>(null);

  const load = useCallback(() => {
    api.trash().then(setRows).catch((cause) => setMessage(errorText(cause)));
  }, []);
  useEffect(load, [load]);

  async function run(work: () => Promise<unknown>, done?: string) {
    setMessage(null);
    try {
      await work();
      if (done) notify(done);
      load();
      onChanged();
    } catch (cause) {
      setMessage(errorText(cause));
    }
  }

  return (
    <div>
      <p className="max-w-[60ch] text-muted">{t("trashLead")}</p>
      {rows.length > 0 ? (
        <button
          type="button"
          className="mt-4 text-stamp"
          onClick={() => void confirm(t("emptyTrashConfirm").replace("{count}", String(rows.length)), { danger: true, confirmLabel: t("emptyTrash") }).then((ok) => { if (ok) void run(() => api.emptyTrash(), t("trashEmptied")); })}
        >
          {t("emptyTrash")}
        </button>
      ) : null}
      <ul className="mt-4">
        {rows.map((row) => (
          <li key={row.id} className="flex flex-wrap items-center gap-3 border-b border-rule py-3 dark:border-night-rule">
            <span className="min-w-0 flex-1">
              <span className="font-display text-xl">{row.title}</span>
              <span className="block text-sm text-muted">{row.authors.join(", ")} · {t("deletedOn")} {row.deletedAt.slice(0, 10)}{row.files ? ` · ${t("files")}: ${row.files}` : ""}</span>
            </span>
            <button type="button" className="border border-rule px-3 py-1 dark:border-night-rule" onClick={() => void run(() => api.restoreBook(row.id), t("bookRestored"))}>{t("restoreBook")}</button>
            <button
              type="button"
              className="px-3 py-1 text-stamp"
              onClick={() => void confirm(t("purgeConfirm").replace("{title}", row.title), { danger: true, confirmLabel: t("purge") }).then((ok) => { if (ok) void run(() => api.purgeBook(row.id)); })}
            >
              {t("purge")}
            </button>
          </li>
        ))}
      </ul>
      {rows.length === 0 ? <p className="mt-4 text-muted">{t("trashEmpty")}</p> : null}
      {message ? <p className="mt-4 text-stamp" role="alert">{message}</p> : null}
    </div>
  );
}

function RepairTab({ onChanged }: { onChanged: () => void }) {
  const { t } = useI18n();
  const { notify } = useUi();
  const [rows, setRows] = useState<FileRepair[] | null>(null);
  const [picked, setPicked] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const load = useCallback(() => {
    setRows(null);
    api.fileRepairs().then((next) => { setRows(next); setPicked(next.map((row) => row.bookId)); }).catch((cause) => setMessage(errorText(cause)));
  }, []);
  useEffect(load, [load]);

  async function apply() {
    setBusy(true);
    try {
      const fixed = await api.applyFileRepairs(picked);
      notify(t("repairDone").replace("{count}", String(fixed)));
      load();
      onChanged();
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(false);
    }
  }

  if (!rows) return <p>{message ?? t("loading")}</p>;
  return (
    <div>
      <p className="max-w-[60ch] text-muted">{t("repairLead")}</p>
      {message ? <p className="mt-3 text-stamp" role="status">{message}</p> : null}
      {rows.length === 0 ? <p className="mt-4">{t("repairNone")}</p> : (
        <>
          <button type="button" disabled={busy || picked.length === 0} onClick={() => void apply()} className="mt-4 bg-binding px-4 py-2 text-sheet disabled:opacity-50">
            {t("repairApply")} ({picked.length})
          </button>
          <ul className="mt-4">
            {rows.map((row) => (
              <li key={row.bookId} className="border-b border-rule py-3 dark:border-night-rule">
                <label className="flex items-start gap-3">
                  <input type="checkbox" className="mt-1" checked={picked.includes(row.bookId)} onChange={() => setPicked((current) => current.includes(row.bookId) ? current.filter((id) => id !== row.bookId) : [...current, row.bookId])} />
                  <span>
                    <span className="block text-muted line-through">{row.oldTitle} · {row.oldAuthors.join(", ")}</span>
                    <span className="block font-display text-xl">{row.newTitle} · {row.newAuthors.join(", ")}</span>
                  </span>
                </label>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

function DuplicatesTab({ onChanged, onOpen }: { onChanged: () => void; onOpen: (id: string) => void }) {
  const { t } = useI18n();
  const { confirm, notify } = useUi();
  const [groups, setGroups] = useState<DuplicateGroup[] | null>(null);
  const [keep, setKeep] = useState<Record<number, string>>({});
  const [message, setMessage] = useState<string | null>(null);

  const load = useCallback(() => {
    api.duplicates().then((next) => { setGroups(next); setKeep({}); }).catch((cause) => setMessage(errorText(cause)));
  }, []);
  useEffect(load, [load]);

  async function merge(index: number, group: DuplicateGroup) {
    const keepId = keep[index] ?? group.books[0].id;
    const kept = group.books.find((book) => book.id === keepId);
    const others = group.books.filter((book) => book.id !== keepId);
    const ok = await confirm(t("mergeConfirm").replace("{title}", kept?.title ?? "").replace("{count}", String(others.length)), { confirmLabel: t("merge") });
    if (!ok) return;
    try {
      for (const other of others) await api.mergeBooks(keepId, other.id);
      notify(t("merged"));
      load();
      onChanged();
    } catch (cause) {
      setMessage(errorText(cause));
    }
  }

  if (!groups) return <p>{message ?? t("loading")}</p>;
  return (
    <div>
      <p className="max-w-[60ch] text-muted">{t("duplicatesLead")}</p>
      {groups.length === 0 ? <p className="mt-4">{t("duplicatesNone")}</p> : null}
      <ul className="mt-4 grid gap-6">
        {groups.map((group, index) => (
          <li key={group.books.map((b) => b.id).join("|")} className="border border-rule p-4 dark:border-night-rule">
            <p className="text-sm text-muted">{group.reason === "isbn" ? t("duplicateByIsbn") : t("duplicateByTitle")}</p>
            <fieldset className="mt-2">
              <legend className="sr-only">{t("keepWhich")}</legend>
              {group.books.map((book) => (
                <label key={book.id} className="flex items-start gap-3 py-2">
                  <input type="radio" name={`keep-${index}`} checked={(keep[index] ?? group.books[0].id) === book.id} onChange={() => setKeep({ ...keep, [index]: book.id })} className="mt-1.5" />
                  <span className="flex-1">
                    <span className="font-display text-xl">{book.title}</span>
                    <span className="block text-sm text-muted">
                      {book.authors.join(", ")}{book.publicationYear ? `, ${book.publicationYear}` : ""}{book.isbn ? ` · ISBN ${book.isbn}` : ""} · {t("files")}: {book.files} · {t("physicalCopy")}: {book.copies}
                    </span>
                  </span>
                  <button type="button" className="text-sm text-moss" onClick={() => onOpen(book.id)}>{t("open")}</button>
                </label>
              ))}
            </fieldset>
            <div className="mt-3 flex flex-wrap gap-2">
              <button type="button" className="bg-binding px-4 py-2 text-sheet" onClick={() => void merge(index, group)}>{t("mergeIntoChosen")}</button>
              <button type="button" className="border border-rule px-4 py-2 dark:border-night-rule" onClick={() => void api.ignoreDuplicates(group.books.map((b) => b.id)).then(load).catch((cause) => setMessage(errorText(cause)))}>{t("notDuplicates")}</button>
            </div>
          </li>
        ))}
      </ul>
      {message ? <p className="mt-4 text-stamp" role="alert">{message}</p> : null}
    </div>
  );
}

function ImportTab({ progress, onChanged }: { progress: JobProgress | null; onChanged: () => void }) {
  const { t, text } = useI18n();
  const [rows, setRows] = useState<ImportRow[]>([]);
  const [chosen, setChosen] = useState<Set<number>>(new Set());
  const [message, setMessage] = useState<string | null>(null);
  const [errors, setErrors] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  async function preview(kind: "calibre" | "goodreads" | "csv") {
    setMessage(null);
    setErrors([]);
    const path = kind === "calibre" ? await api.pickFolder() : await api.pickCsvFile();
    if (!path) return;
    setBusy(true);
    try {
      const next = await api.previewImport(kind, path);
      setRows(next);
      setChosen(new Set(next.map((row, index) => (row.duplicateOf ? -1 : index)).filter((index) => index >= 0)));
      setMessage(`${t("importFound")}: ${next.length}`);
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(false);
    }
  }

  async function commit() {
    setBusy(true);
    setMessage(null);
    try {
      const summary = await api.commitImport(rows.filter((_, index) => chosen.has(index)));
      setMessage(`${t("importAdded")}: ${summary.added} · ${t("importSkipped")}: ${summary.skipped} · ${t("files")}: ${summary.files}`);
      setErrors(summary.errors);
      setRows([]);
      setChosen(new Set());
      onChanged();
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(false);
    }
  }

  const toggle = (index: number) => {
    const next = new Set(chosen);
    if (next.has(index)) next.delete(index);
    else next.add(index);
    setChosen(next);
  };

  return (
    <div>
      <p className="max-w-[60ch] text-muted">{t("importLead")}</p>
      <div className="mt-4 flex flex-wrap gap-2">
        <button type="button" disabled={busy} className="bg-binding px-4 py-2 text-sheet disabled:opacity-50" onClick={() => void preview("calibre")}>{t("importCalibre")}</button>
        <button type="button" disabled={busy} className="border border-rule px-4 py-2 disabled:opacity-50" onClick={() => void preview("goodreads")}>{t("importGoodreads")}</button>
        <button type="button" disabled={busy} className="border border-rule px-4 py-2 disabled:opacity-50" onClick={() => void preview("csv")}>{t("importCsv")}</button>
      </div>
      <p className="mt-2 text-sm text-muted">{t("importCsvHint")}</p>
      {busy && progress?.job === "import" && !progress.done ? <p className="mt-3" role="status">{progress.current} / {progress.total} · {progress.label} <button type="button" className="ml-2 text-stamp" onClick={() => void api.cancel()}>{t("cancelJob")}</button></p> : null}
      {message ? <p className="mt-4" role="status">{text(message)}</p> : null}
      {errors.length > 0 ? (
        <details className="mt-2 text-sm text-stamp">
          <summary>{t("importErrors")}: {errors.length}</summary>
          <ul className="mt-2">{errors.slice(0, 100).map((error, index) => <li key={index}>{text(error)}</li>)}</ul>
        </details>
      ) : null}
      {rows.length > 0 ? (
        <>
          <div className="mt-6 flex flex-wrap items-center gap-3">
            <button type="button" disabled={busy || chosen.size === 0} className="bg-stamp px-4 py-2 text-sheet disabled:opacity-50" onClick={() => void commit()}>{t("importChosen")} ({chosen.size})</button>
            <button type="button" className="text-muted" onClick={() => setChosen(new Set(rows.map((_, index) => index)))}>{t("selectAll")}</button>
            <button type="button" className="text-muted" onClick={() => setChosen(new Set())}>{t("selectNone")}</button>
          </div>
          <ul className="mt-4">
            {rows.slice(0, 1000).map((row, index) => (
              <li key={index} className="border-b border-rule py-2 dark:border-night-rule">
                <label className="flex items-start gap-3">
                  <input type="checkbox" className="mt-1.5" checked={chosen.has(index)} onChange={() => toggle(index)} />
                  <span className="flex-1">
                    <span className="font-display text-lg">{row.title}</span>
                    <span className="block text-sm text-muted">
                      {row.authors.join(", ") || "—"}{row.year ? `, ${row.year}` : ""}{row.isbn13 || row.isbn10 ? ` · ISBN ${row.isbn13 ?? row.isbn10}` : ""}
                      {row.files.length ? ` · ${t("files")}: ${row.files.length}` : ""}{row.rating ? ` · ${"★".repeat(row.rating)}` : ""}
                    </span>
                    {row.duplicateOf ? <span className="block text-sm text-stamp">{t("importDuplicate")}</span> : null}
                  </span>
                </label>
              </li>
            ))}
          </ul>
          {rows.length > 1000 ? <p className="mt-2 text-sm text-muted">{t("importMore").replace("{count}", String(rows.length - 1000))}</p> : null}
        </>
      ) : null}
    </div>
  );
}

function PrintTab() {
  const { t } = useI18n();
  const [rows, setRows] = useState<ExportRow[]>([]);
  const [mode, setMode] = useState<"catalog" | "labels">("catalog");
  const [onlyPaper, setOnlyPaper] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    api.printRows().then(setRows).catch((cause) => setMessage(errorText(cause)));
  }, []);

  const visible = onlyPaper ? rows.filter((row) => row.paper) : rows;

  async function exportCsv() {
    try {
      const folder = await api.pickFolder();
      if (!folder) return;
      const file = await api.exportCsv(folder);
      setMessage(`${t("csvSaved")}: ${file}`);
    } catch (cause) {
      setMessage(errorText(cause));
    }
  }

  return (
    <div>
      <div className="flex flex-wrap items-center gap-2 print:hidden">
        <button type="button" className="bg-binding px-4 py-2 text-sheet" onClick={() => void exportCsv()}>{t("exportCsv")}</button>
        <select value={mode} onChange={(event) => setMode(event.target.value as "catalog" | "labels")} aria-label={t("printTitle")} className="border border-rule bg-sheet px-2 py-2 dark:border-night-rule dark:bg-night">
          <option value="catalog">{t("printCatalog")}</option>
          <option value="labels">{t("printLabels")}</option>
        </select>
        <label className="flex items-center gap-2"><input type="checkbox" checked={onlyPaper} onChange={(event) => setOnlyPaper(event.target.checked)} /> {t("onlyPaper")}</label>
        <button type="button" className="border border-rule px-4 py-2" onClick={() => window.print()}>{t("printNow")}</button>
      </div>
      <p className="mt-2 text-sm text-muted print:hidden">{t("printHint")}</p>
      {message ? <p className="mt-3 break-all print:hidden" role="status">{message}</p> : null}
      {mode === "catalog" ? (
        <table className="mt-6 w-full border-collapse text-sm">
          <thead>
            <tr className="border-b-2 border-ink text-left">
              <th className="py-1 pr-2">{t("modeTitle")}</th>
              <th className="py-1 pr-2">{t("author")}</th>
              <th className="py-1 pr-2">{t("year")}</th>
              <th className="py-1 pr-2">ISBN</th>
              <th className="py-1 pr-2">{t("location")}</th>
              <th className="py-1">{t("filterLoaned")}</th>
            </tr>
          </thead>
          <tbody>
            {visible.map((row) => (
              <tr key={row.id} className="break-inside-avoid border-b border-rule">
                <td className="py-1 pr-2">{row.title}{row.series ? ` (${row.series}${row.seriesNumber ? ` ${row.seriesNumber}` : ""})` : ""}</td>
                <td className="py-1 pr-2">{row.authors.join(", ")}</td>
                <td className="py-1 pr-2 tabular-nums">{row.year ?? ""}</td>
                <td className="py-1 pr-2 tabular-nums">{row.isbn13 ?? row.isbn10 ?? ""}</td>
                <td className="py-1 pr-2">{[row.shelves.join(", "), row.location].filter(Boolean).join(" · ")}</td>
                <td className="py-1">{row.loanedTo ?? ""}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <ul className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-3 print:grid-cols-3">
          {visible.map((row) => (
            <li key={row.id} className="break-inside-avoid border border-rule p-2 text-xs print:border-ink">
              <p className="font-display text-sm leading-tight">{row.title}</p>
              <p className="text-muted print:text-ink">{row.authors.join(", ")}</p>
              <Barcode value={row.barcode ?? row.isbn13 ?? row.isbn10 ?? ""} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Barcode({ value }: { value: string }) {
  const bits = ean13Modules(value);
  if (!bits) return value ? <p className="mt-2 font-mono">{value}</p> : null;
  const width = bits.length + 18;
  return (
    <svg viewBox={`0 0 ${width} 40`} className="mt-2 h-12 w-full" role="img" aria-label={value}>
      <rect width={width} height={40} fill="#fff" />
      {bits.split("").map((bit, index) => (bit === "1" ? <rect key={index} x={index + 9} y={2} width={1} height={30} fill="#000" /> : null))}
      <text x={width / 2} y={39} fontSize={7} textAnchor="middle" fontFamily="monospace" fill="#000">{value.replace(/[^0-9X]/gi, "")}</text>
    </svg>
  );
}
