import { useEffect, useState, type ReactNode } from "react";
import { api } from "../api";
import { useI18n, type Locale } from "../i18n";
import { errorText, type Settings } from "../types";

export function SettingsView({ theme, onTheme, onLocale, onChanged }: { theme: "light" | "dark"; onTheme: (theme: "light" | "dark") => void; onLocale: (locale: Locale) => void; onChanged: (settings: Settings) => void }) {
  const { t, text } = useI18n();
  const [settings, setSettings] = useState<Settings | null>(null);
  const [googleKey, setGoogleKey] = useState("");
  const [webdavPassword, setWebdavPassword] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [goal, setGoal] = useState("");

  useEffect(() => {
    api.settings().then((next) => { setSettings(next); setGoal(next.readingGoal?.toString() ?? ""); }).catch((cause) => setMessage(errorText(cause)));
  }, []);

  async function save(patch: Partial<Settings>) {
    try {
      const next = await api.saveSettings(patch);
      setSettings(next);
      onChanged(next);
      setMessage(t("saved"));
    } catch (cause) {
      setMessage(errorText(cause));
    }
  }

  async function choose(field: "libraryDir" | "backupDir" | "syncDir") {
    try {
      const folder = await api.pickFolder();
      if (folder) await save({ [field]: folder });
    } catch (cause) {
      setMessage(errorText(cause));
    }
  }

  if (!settings) return <p>{message ?? t("loadingSettings")}</p>;

  return (
    <section className="max-w-2xl">
      <h1 className="font-display text-5xl leading-none">{t("settings")}</h1>
      <div className="mt-8 border-t border-rule dark:border-night-rule">
        <Row title={t("libraryFolder")} text={settings.libraryDir ?? t("notChosenYet")}>
          <button type="button" className="bg-binding px-3 py-1.5 text-sheet" onClick={() => void choose("libraryDir")}>{t("choose")}</button>
        </Row>
        <Row title={t("theme")} text={t("themeHint")}>
          <button type="button" className={theme === "light" ? "bg-binding px-3 py-1.5 text-sheet" : "border border-rule px-3 py-1.5"} onClick={() => { onTheme("light"); void save({ theme: "light" }); }}>{t("light")}</button>
          <button type="button" className={theme === "dark" ? "bg-binding px-3 py-1.5 text-sheet" : "border border-rule px-3 py-1.5"} onClick={() => { onTheme("dark"); void save({ theme: "dark" }); }}>{t("dark")}</button>
        </Row>
        <Row title={t("language")} text={t("languageHint")}>
          <button type="button" className={settings.locale === "pl" ? "bg-binding px-3 py-1.5 text-sheet" : "border border-rule px-3 py-1.5"} onClick={() => { onLocale("pl"); void save({ locale: "pl" }); }}>{t("polish")}</button>
          <button type="button" className={settings.locale === "en" ? "bg-binding px-3 py-1.5 text-sheet" : "border border-rule px-3 py-1.5"} onClick={() => { onLocale("en"); void save({ locale: "en" }); }}>{t("english")}</button>
        </Row>
        <Row title={t("openLibrary")} text={t("openLibraryHint")}>
          <input type="checkbox" checked={settings.openLibrary} onChange={(event) => void save({ openLibrary: event.target.checked })} />
        </Row>
        <Row title={t("nationalLibrary")} text={t("nationalLibraryHint")}>
          <input type="checkbox" checked={settings.nationalLibrary} onChange={(event) => void save({ nationalLibrary: event.target.checked })} />
        </Row>
        <Row title={t("googleBooks")} text={settings.googleBooksHasKey ? t("googleKeySaved") : t("googleKeyMissing")}>
          <input type="checkbox" checked={settings.googleBooks} onChange={(event) => void save({ googleBooks: event.target.checked })} />
        </Row>
        <form className="grid gap-2 border-b border-rule py-4 dark:border-night-rule" onSubmit={(event) => { event.preventDefault(); void api.setSecret("google-books", googleKey).then(() => api.settings()).then((next) => { setSettings(next); setGoogleKey(""); setMessage(t("keySaved")); }).catch((cause) => setMessage(errorText(cause))); }}>
          <label>{t("googleKey")}<input type="password" value={googleKey} onChange={(event) => setGoogleKey(event.target.value)} className="mt-1 w-full border border-rule bg-sheet px-3 py-2 dark:border-night-rule dark:bg-night" /></label>
          <button className="w-fit border border-rule px-3 py-1.5">{t("saveKey")}</button>
        </form>
        <Row title={t("lubimy")} text={t("lubimyHint")}>
          <input type="checkbox" checked={settings.lubimyCzytac} onChange={(event) => void save({ lubimyCzytac: event.target.checked })} />
        </Row>
        <Row title={t("upoluj")} text={t("upolujHint")}>
          <input type="checkbox" checked={settings.upolujebooka} onChange={(event) => void save({ upolujebooka: event.target.checked })} />
        </Row>
        <Row title="Empik" text={t("empikHint")}>
          <input type="checkbox" checked={settings.empik} onChange={(event) => void save({ empik: event.target.checked })} />
        </Row>
        <Row title="Apple Books" text={t("appleHint")}>
          <input type="checkbox" checked={settings.appleBooks} onChange={(event) => void save({ appleBooks: event.target.checked })} />
        </Row>
        <form className="flex flex-wrap items-center justify-between gap-4 border-b border-rule py-4 dark:border-night-rule" onSubmit={(event) => { event.preventDefault(); void save({ readingGoal: goal.trim() ? Number(goal) : 0 }); }}>
          <div>
            <h2 className="text-base">{t("readingGoal")}</h2>
            <p className="max-w-md text-sm text-muted">{t("readingGoalHint")}</p>
          </div>
          <div className="flex gap-2">
            <input type="number" min={0} max={10000} value={goal} onChange={(event) => setGoal(event.target.value)} aria-label={t("readingGoal")} className="w-24 border border-rule bg-sheet px-3 py-1.5 dark:border-night-rule dark:bg-night" />
            <button className="border border-rule px-3 py-1.5">{t("save")}</button>
          </div>
        </form>
        <Row title={t("dailyBackup")} text={t("dailyBackupHint")}>
          <input type="checkbox" checked={settings.autoBackup} onChange={(event) => void save({ autoBackup: event.target.checked })} />
        </Row>
        <Row title={t("backupFolder")} text={settings.backupDir ?? t("notChosen")}>
          <button type="button" onClick={() => void choose("backupDir")}>{t("choose")}</button>
        </Row>
        <Row title={t("syncTitle")} text={t("syncHint")}>
          <select value={settings.syncProvider} onChange={(event) => void save({ syncProvider: event.target.value })} className="border border-rule bg-sheet px-2 py-1 dark:border-night-rule dark:bg-night">
            <option value="folder">{t("folder")}</option>
            <option value="webdav">WebDAV</option>
          </select>
        </Row>
        <Row title={t("syncFolder")} text={settings.syncDir ?? t("notChosen")}>
          <button type="button" onClick={() => void choose("syncDir")}>{t("choose")}</button>
        </Row>
        <form className="grid gap-2 border-b border-rule py-4 dark:border-night-rule" onSubmit={(event) => { event.preventDefault(); void save({ webdavUrl: settings.webdavUrl ?? "", webdavUser: settings.webdavUser ?? "" }); }}>
          <label>{t("webdavUrl")}<input value={settings.webdavUrl ?? ""} onChange={(event) => setSettings({ ...settings, webdavUrl: event.target.value })} className="mt-1 w-full border border-rule bg-sheet px-3 py-2 dark:border-night-rule dark:bg-night" /></label>
          <label>{t("webdavUser")}<input value={settings.webdavUser ?? ""} onChange={(event) => setSettings({ ...settings, webdavUser: event.target.value })} className="mt-1 w-full border border-rule bg-sheet px-3 py-2 dark:border-night-rule dark:bg-night" /></label>
          <button className="w-fit border border-rule px-3 py-1.5">{t("saveWebdav")}</button>
        </form>
        <form className="grid gap-2 border-b border-rule py-4 dark:border-night-rule" onSubmit={(event) => { event.preventDefault(); void api.setSecret("webdav-password", webdavPassword).then(() => { setWebdavPassword(""); setMessage(settings.webdavHasPassword ? t("passwordChanged") : t("passwordSaved")); return api.settings(); }).then(setSettings).catch((cause) => setMessage(errorText(cause))); }}>
          <label>{t("webdavPassword")}<input type="password" value={webdavPassword} onChange={(event) => setWebdavPassword(event.target.value)} className="mt-1 w-full border border-rule bg-sheet px-3 py-2 dark:border-night-rule dark:bg-night" /></label>
          <button className="w-fit border border-rule px-3 py-1.5">{t("savePassword")}</button>
        </form>
        <div className="flex flex-wrap gap-2 py-4">
          <button type="button" className="bg-binding px-4 py-2 text-sheet" onClick={() => void (async () => { const folder = await api.pickFolder(); if (!folder) return; await api.exportLibrary(folder); setMessage(t("exported")); })().catch((cause) => setMessage(errorText(cause)))}>{t("export")}</button>
          <button type="button" className="border border-rule px-4 py-2" onClick={() => void (async () => { const folder = await api.pickFolder(); if (!folder) return; setMessage(text(await api.importLibrary(folder))); })().catch((cause) => setMessage(errorText(cause)))}>{t("import")}</button>
        </div>
        <div className="py-4 text-sm text-muted">
          <p>{t("shortcuts")}</p>
          <p className="mt-2">{t("privacy")}</p>
        </div>
      </div>
      {message ? <p className="text-stamp" role="status">{message}</p> : null}
    </section>
  );
}

function Row({ title, text, children }: { title: string; text: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4 border-b border-rule py-4 dark:border-night-rule">
      <div>
        <h2 className="text-base">{title}</h2>
        <p className="max-w-md text-sm text-muted">{text}</p>
      </div>
      <div className="flex gap-2">{children}</div>
    </div>
  );
}
