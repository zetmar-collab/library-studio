import { useEffect, useState } from "react";
import { api } from "../api";
import { useI18n } from "../i18n";
import { errorText, type BookCard, type ReaderDevice, type ReaderFile, type TransferReport } from "../types";

function noteText(note: TransferReport["notes"][number], t: (key: "readerNoFile" | "readerBadFormat") => string): string {
  if (note.code === "no-file") return `${note.name}: ${t("readerNoFile")}`;
  if (note.code === "bad-format") return `${note.name}: ${t("readerBadFormat")} ${note.detail}`;
  return note.detail ? `${note.name}: ${note.detail}` : note.name;
}

export function ReaderView({ books, bookIds, onChanged }: { books: BookCard[]; bookIds: string[]; onChanged: () => void }) {
  const { t } = useI18n();
  const [devices, setDevices] = useState<ReaderDevice[]>([]);
  const [device, setDevice] = useState<ReaderDevice | null>(null);
  const [files, setFiles] = useState<ReaderFile[]>([]);
  const [pickedBooks, setPickedBooks] = useState<string[]>(bookIds);
  const [pickedFiles, setPickedFiles] = useState<string[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const [notes, setNotes] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  async function loadDevices() {
    const found = await api.readers();
    setDevices(found);
    setDevice((current) => current ?? found[0] ?? null);
  }

  async function loadFiles(next: ReaderDevice) {
    setFiles(await api.readerBooks(next.root));
    setPickedFiles([]);
  }

  useEffect(() => {
    loadDevices().catch((cause) => setMessage(errorText(cause)));
  }, []);

  useEffect(() => {
    if (!device) return;
    loadFiles(device).catch((cause) => setMessage(errorText(cause)));
  }, [device?.root]);

  function reportText(report: TransferReport): string {
    return `${t("readerCopied")}: ${report.copied}. ${t("readerAdded")}: ${report.added}. ${t("readerSkipped")}: ${report.skipped}.`;
  }

  async function send() {
    if (!device || pickedBooks.length === 0) return;
    setBusy(true);
    setMessage(t("readerWorking"));
    try {
      const report = await api.sendToReader(device.root, pickedBooks);
      setMessage(reportText(report));
      setNotes(report.notes.map((note) => noteText(note, t)));
      await loadFiles(device);
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(false);
    }
  }

  async function pull() {
    if (!device || pickedFiles.length === 0) return;
    setBusy(true);
    setMessage(t("readerWorking"));
    try {
      const report = await api.importFromReader(device.root, pickedFiles);
      setMessage(reportText(report));
      setNotes(report.notes.map((note) => noteText(note, t)));
      await loadFiles(device);
      onChanged();
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(false);
    }
  }

  async function pickFolder() {
    const folder = await api.pickFolder();
    if (!folder) return;
    const next = await api.openReaderFolder(folder);
    setDevices((current) => [next, ...current.filter((item) => item.root !== next.root)]);
    setDevice(next);
  }

  const digital = books.filter((book) => book.hasDigital);

  return (
    <section>
      <h1 className="font-display text-5xl leading-none">{t("readerTitle")}</h1>
      <p className="mt-4 max-w-[58ch] text-muted">{t("readerLead")}</p>
      <div className="mt-6 flex flex-wrap gap-2">
        <button type="button" className="border border-rule px-4 py-2" onClick={() => void loadDevices().catch((cause) => setMessage(errorText(cause)))}>{t("readerRefresh")}</button>
        <button type="button" className="border border-rule px-4 py-2" onClick={() => void pickFolder().catch((cause) => setMessage(errorText(cause)))}>{t("readerPick")}</button>
      </div>
      {devices.length === 0 ? <p className="mt-6 max-w-[52ch]">{t("readerNone")}</p> : null}
      <div className="mt-4 flex flex-wrap gap-2">
        {devices.map((item) => (
          <button key={item.root} type="button" onClick={() => setDevice(item)} className={device?.root === item.root ? "bg-binding px-3 py-2 text-sheet" : "border border-rule px-3 py-2"}>
            {item.name}
          </button>
        ))}
      </div>
      {device ? <p className="mt-3 text-sm text-muted">{t("readerFormats")}: {device.formats.join(", ")}</p> : null}

      <h2 className="mt-10 font-display text-3xl">{t("sendToReader")}</h2>
      <p className="mt-2 text-sm text-muted">{t("readerSelectBooks")}</p>
      <ul className="mt-3">
        {digital.map((book) => (
          <li key={book.id} className="border-b border-rule py-2 dark:border-night-rule">
            <label className="flex items-center gap-3">
              <input type="checkbox" checked={pickedBooks.includes(book.id)} onChange={() => setPickedBooks((current) => current.includes(book.id) ? current.filter((id) => id !== book.id) : [...current, book.id])} />
              <span>{book.title}<span className="text-muted"> · {book.authors.join(", ")}</span></span>
            </label>
          </li>
        ))}
      </ul>
      <button type="button" disabled={busy || !device || pickedBooks.length === 0} onClick={() => void send()} className="mt-4 bg-stamp px-4 py-2 text-sheet disabled:opacity-50">{t("readerSend")}</button>

      <h2 className="mt-10 font-display text-3xl">{t("readerOnDevice")}</h2>
      {files.length === 0 ? <p className="mt-3 text-muted">{t("readerEmptyDevice")}</p> : (
        <ul className="mt-3">
          {files.map((file) => (
            <li key={file.relativePath} className="border-b border-rule py-2 dark:border-night-rule">
              <label className="flex items-center gap-3">
                <input type="checkbox" checked={pickedFiles.includes(file.relativePath)} onChange={() => setPickedFiles((current) => current.includes(file.relativePath) ? current.filter((path) => path !== file.relativePath) : [...current, file.relativePath])} />
                <span>{file.fileName}<span className="text-muted"> · {file.format}{file.alreadyInCatalog ? ` · ${t("readerAlready")}` : ""}</span></span>
              </label>
            </li>
          ))}
        </ul>
      )}
      <button type="button" disabled={busy || !device || pickedFiles.length === 0} onClick={() => void pull()} className="mt-4 bg-binding px-4 py-2 text-sheet disabled:opacity-50">{t("readerImport")}</button>
      {message ? <p className="mt-4" role="status">{message}</p> : null}
      {notes.length > 0 ? <ul className="mt-2 text-sm text-stamp">{notes.map((note) => <li key={note}>{note}</li>)}</ul> : null}
    </section>
  );
}
