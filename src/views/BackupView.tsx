import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { useI18n } from "../i18n";
import { useUi } from "../ui";
import { errorText, type BackupRow, type ConflictRow, type JobProgress } from "../types";

export function BackupView({ progress, onChanged }: { progress: JobProgress | null; onChanged: () => void }) {
  const { t, text } = useI18n();
  const { confirm } = useUi();
  const [rows, setRows] = useState<BackupRow[]>([]);
  const [conflicts, setConflicts] = useState<ConflictRow[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    const [nextRows, nextConflicts] = await Promise.all([api.backups(), api.conflicts()]);
    setRows(nextRows);
    setConflicts(nextConflicts);
  }, []);

  useEffect(() => {
    load().catch((cause) => setMessage(errorText(cause)));
  }, [load]);

  async function job(work: () => Promise<string | void>) {
    setMessage(null);
    setBusy(true);
    try {
      const result = await work();
      if (result) setMessage(result);
      await load();
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(false);
    }
  }

  async function restore(path: string) {
    const ok = await confirm(t("restoreConfirm").replace("{path}", path), { danger: true, confirmLabel: t("restoreCopy") });
    if (!ok) return;
    await job(async () => {
      const safety = await api.restore(path);
      onChanged();
      return `${t("restored")} ${t("restoredSafety")} ${safety}`;
    });
  }

  return (
    <section>
      <h1 className="font-display text-5xl leading-none">{t("backupTitle")}</h1>
      <p className="mt-3 max-w-[48ch] text-muted">{t("backupLead")}</p>
      <div className="mt-6 flex flex-wrap gap-2">
        <button type="button" disabled={busy} className="bg-binding px-4 py-2 text-sheet disabled:opacity-50" onClick={() => void job(async () => { await api.backup("database"); return t("backupSaved"); })}>{t("backupDatabase")}</button>
        <button type="button" disabled={busy} className="border border-rule px-4 py-2 disabled:opacity-50" onClick={() => void job(async () => { await api.backup("files"); return t("backupSaved"); })}>{t("backupFiles")}</button>
        <button type="button" disabled={busy} className="border border-rule px-4 py-2 disabled:opacity-50" onClick={() => void job(async () => { await api.backup("full"); return t("backupSaved"); })}>{t("backupFull")}</button>
        <button
          type="button"
          disabled={busy}
          className="bg-stamp px-4 py-2 text-sheet disabled:opacity-50"
          onClick={() =>
            void job(async () => {
              const report = await api.sync();
              onChanged();
              return `${text(report.message)} ↑${report.pushed} ↓${report.pulled}`;
            })
          }
        >
          {t("sync")}
        </button>
        <button type="button" disabled={!busy} className="px-3 py-2 disabled:opacity-40" onClick={() => void api.cancel()}>{t("cancelJob")}</button>
      </div>
      {busy && progress && !progress.done ? <p className="mt-4" role="status">{text(progress.label)} {progress.total ? `${progress.current} / ${progress.total}` : ""}</p> : null}
      {message ? <p className="mt-4" role="status">{text(message)}</p> : null}
      <h2 className="mt-8 font-display text-3xl">{t("history")}</h2>
      <button
        type="button"
        disabled={busy}
        className="mt-2 text-stamp"
        onClick={() => void api.pickBackupFile().then((path) => { if (path) void restore(path); }).catch((cause) => setMessage(errorText(cause)))}
      >
        {t("restoreFromFile")}
      </button>
      <ul>
        {rows.map((row) => (
          <li key={row.id} className="border-b border-rule py-3 dark:border-night-rule">
            <span className="block">{row.kind} · {row.status} · {row.startedAt.replace("T", " ").slice(0, 16)}</span>
            <span className="break-all text-sm text-muted">{row.destination}</span>
            {row.kind === "database" && row.status === "ok" ? (
              <button type="button" disabled={busy} className="mt-1 block text-stamp" onClick={() => void restore(row.destination)}>{t("restoreCopy")}</button>
            ) : null}
          </li>
        ))}
      </ul>
      <h2 className="mt-8 font-display text-3xl">{t("conflicts")}</h2>
      <ul>
        {conflicts.map((conflict) => (
          <li key={conflict.id} className="border-b border-rule py-3 dark:border-night-rule">
            <p>{text(conflict.summary)}</p>
            <p className="text-sm text-muted">{t("conflictLocal")}: {conflict.localUpdatedAt ?? "—"} · {t("conflictRemote")}: {conflict.remoteUpdatedAt ?? "—"}</p>
            <div className="mt-2 flex gap-2">
              <button type="button" disabled={busy} className="border border-rule px-3 py-1 dark:border-night-rule" onClick={() => void job(async () => { await api.resolveConflict(conflict.id, true); onChanged(); })}>{t("keepLocal")}</button>
              <button type="button" disabled={busy} className="border border-rule px-3 py-1 dark:border-night-rule" onClick={() => void job(async () => { await api.resolveConflict(conflict.id, false); onChanged(); })}>{t("takeRemote")}</button>
            </div>
          </li>
        ))}
      </ul>
      {conflicts.length === 0 ? <p className="mt-3 text-muted">{t("noConflicts")}</p> : null}
    </section>
  );
}
