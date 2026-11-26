import { useMemo, useState } from "react";
import { api } from "../api";
import { useI18n } from "../i18n";
import { useUi } from "../ui";
import { errorText, type FolderScan, type JobProgress } from "../types";

const input = "w-full border border-rule bg-sheet px-3 py-2 dark:border-night-rule dark:bg-night";

function megabytes(size: number): string {
  return `${(size / 1_048_576).toFixed(size < 10_485_760 ? 1 : 0)} MB`;
}

/// Dodawanie e-booków z folderu na komputerze: wybór folderu, podgląd z tytułem i autorem, import zaznaczonych.
export function FolderImport({ progress, onImported }: { progress: JobProgress | null; onImported: () => void }) {
  const { t, text } = useI18n();
  const { notify } = useUi();
  const [scan, setScan] = useState<FolderScan | null>(null);
  const [picked, setPicked] = useState<string[]>([]);
  const [filter, setFilter] = useState("");
  const [busy, setBusy] = useState<"scan" | "import" | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const shown = useMemo(() => {
    if (!scan) return [];
    const needle = filter.trim().toLowerCase();
    if (!needle) return scan.files;
    return scan.files.filter((file) => `${file.title} ${file.authors.join(" ")} ${file.relativePath}`.toLowerCase().includes(needle));
  }, [scan, filter]);
  const fresh = shown.filter((file) => !file.alreadyInCatalog);
  const allPicked = fresh.length > 0 && fresh.every((file) => picked.includes(file.relativePath));

  async function choose() {
    setMessage(null);
    const path = await api.pickFolder().catch(() => null);
    if (!path) return;
    await load(path);
  }

  async function load(path: string) {
    setBusy("scan");
    setScan(null);
    try {
      const next = await api.scanFolder(path);
      setScan(next);
      setPicked(next.files.filter((file) => !file.alreadyInCatalog).map((file) => file.relativePath));
      if (next.files.length === 0) setMessage(t("folderEmpty"));
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(null);
    }
  }

  async function importPicked() {
    if (!scan || picked.length === 0) return;
    setBusy("import");
    setMessage(null);
    try {
      const report = await api.importFolder(scan.root, picked);
      const summary = t("folderDone").replace("{added}", String(report.added)).replace("{copied}", String(report.copied)).replace("{skipped}", String(report.skipped));
      const problems = report.notes.map((note) => `${note.name}: ${text(note.detail) || note.code}`);
      setMessage([summary, ...problems.slice(0, 5)].join("\n"));
      notify(summary);
      onImported();
      await load(scan.root);
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(null);
    }
  }

  const running = busy !== null && progress && progress.job === "folder" && !progress.done;

  return (
    <div className="mt-6">
      <p className="max-w-[60ch] text-muted">{t("folderLead")}</p>
      <div className="mt-4 flex flex-wrap items-center gap-2">
        <button type="button" disabled={busy !== null} onClick={() => void choose()} className="bg-binding px-4 py-2 text-sheet disabled:opacity-50">
          {scan ? t("folderChooseOther") : t("folderChoose")}
        </button>
        {scan ? <button type="button" disabled={busy !== null} onClick={() => void load(scan.root)} className="border border-rule px-3 py-2 disabled:opacity-50 dark:border-night-rule">{t("folderRescan")}</button> : null}
        {busy ? <button type="button" onClick={() => void api.cancel()} className="px-3 py-2 text-stamp">{t("cancelJob")}</button> : null}
      </div>
      {busy === "scan" && !running ? <p className="mt-3" role="status">{t("folderScanning")}</p> : null}
      {running ? (
        <p className="mt-3" role="status">
          {busy === "scan" ? t("folderScanning") : t("folderImporting")} {progress.total ? `${progress.current} / ${progress.total}` : ""} {progress.label}
        </p>
      ) : null}
      {message ? <p className="mt-3 whitespace-pre-line" role="status">{message}</p> : null}
      {scan && scan.files.length > 0 ? (
        <>
          <p className="mt-4 break-all text-sm text-muted">{scan.root}</p>
          <p className="mt-1 text-sm">
            {t("folderFound").replace("{count}", String(scan.files.length)).replace("{fresh}", String(scan.files.filter((file) => !file.alreadyInCatalog).length))}
            {scan.truncated ? ` ${t("folderTruncated")}` : ""}
          </p>
          <div className="mt-3 flex flex-wrap items-center gap-3">
            <input type="search" value={filter} onChange={(event) => setFilter(event.target.value)} placeholder={t("folderFilter")} className={`${input} max-w-sm`} />
            <label className="flex items-center gap-1 text-sm">
              <input type="checkbox" checked={allPicked} onChange={() => setPicked(allPicked ? picked.filter((path) => !fresh.some((file) => file.relativePath === path)) : Array.from(new Set([...picked, ...fresh.map((file) => file.relativePath)])))} />
              {t("selectAll")}
            </label>
            <button type="button" disabled={busy !== null || picked.length === 0} onClick={() => void importPicked()} className="bg-stamp px-4 py-2 text-sheet disabled:opacity-50">
              {t("folderImport")} ({picked.length})
            </button>
          </div>
          <ul className="mt-4">
            {shown.map((file) => (
              <li key={file.relativePath} className="border-b border-rule py-2 [content-visibility:auto] [contain-intrinsic-size:auto_64px] dark:border-night-rule">
                <label className={`flex items-start gap-3 ${file.alreadyInCatalog ? "opacity-60" : ""}`}>
                  <input
                    type="checkbox"
                    className="mt-1"
                    disabled={file.alreadyInCatalog}
                    checked={picked.includes(file.relativePath)}
                    onChange={() => setPicked((current) => current.includes(file.relativePath) ? current.filter((path) => path !== file.relativePath) : [...current, file.relativePath])}
                  />
                  <span className="min-w-0">
                    <span className="block font-display text-lg leading-tight">{file.title}</span>
                    <span className="block text-sm text-muted">
                      {file.authors.join(", ")} · {file.format.toUpperCase()} · {megabytes(file.size)}
                      {file.alreadyInCatalog ? ` · ${t("folderInCatalog")}` : ""}
                    </span>
                    <span className="block break-all text-xs text-muted">{file.relativePath}</span>
                  </span>
                </label>
              </li>
            ))}
          </ul>
        </>
      ) : null}
    </div>
  );
}
