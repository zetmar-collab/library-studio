import { useEffect, useState } from "react";
import { api } from "../api";
import { useI18n, type MessageKey } from "../i18n";
import { errorText, type FileRow, type JobProgress } from "../types";

const LABEL: Record<string, MessageKey> = {
  present: "filePresent",
  missing: "fileMissing",
  changed: "fileChanged",
  duplicate: "fileDuplicate",
};

export function FilesView({ progress }: { progress: JobProgress | null }) {
  const { t } = useI18n();
  const [files, setFiles] = useState<FileRow[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const [verifying, setVerifying] = useState(false);

  useEffect(() => {
    api.files().then(setFiles).catch((cause) => setMessage(errorText(cause)));
  }, []);

  async function verify() {
    setVerifying(true);
    setMessage(null);
    try {
      setFiles(await api.verifyFiles());
      setMessage(t("verifyDone"));
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setVerifying(false);
    }
  }

  const problems = files.filter((file) => file.status !== "present").length;

  return (
    <section>
      <div className="flex flex-wrap items-end justify-between gap-4">
        <h1 className="font-display text-5xl leading-none">{t("files")}</h1>
        <div className="flex gap-2">
          <button type="button" disabled={verifying} className="bg-binding px-4 py-2 text-sheet disabled:opacity-50" onClick={() => void verify()}>{t("verifyLibrary")}</button>
          {verifying ? <button type="button" className="px-3 py-2 text-stamp" onClick={() => void api.cancel()}>{t("cancelJob")}</button> : null}
        </div>
      </div>
      <p className="mt-3 max-w-[60ch] text-sm text-muted">{t("filesLead")}</p>
      {verifying && progress?.job === "verify" && !progress.done ? (
        <p className="mt-3" role="status">{progress.current} / {progress.total} · {progress.label}</p>
      ) : null}
      {files.length > 0 ? <p className="mt-3 text-sm">{t("filesCount")}: {files.length}{problems ? ` · ${t("filesProblems")}: ${problems}` : ""}</p> : null}
      <ul className="mt-6">
        {files.map((file) => (
          <li key={file.id} className="grid grid-cols-[1fr_auto_auto] gap-4 border-b border-rule py-4 dark:border-night-rule">
            <span>
              <span className="block">{file.fileName}</span>
              <span className="text-sm text-muted">{file.bookTitle}</span>
            </span>
            <span className={file.status === "present" ? "" : "text-stamp"}>{LABEL[file.status] ? t(LABEL[file.status]) : file.status}</span>
            <button type="button" onClick={() => void api.openFile(file.id).catch((cause) => setMessage(errorText(cause)))}>{t("open")}</button>
          </li>
        ))}
      </ul>
      {files.length === 0 ? <p className="mt-8 text-muted">{t("noFiles")}</p> : null}
      {message ? <p className="mt-4 text-stamp" role="status">{message}</p> : null}
    </section>
  );
}
