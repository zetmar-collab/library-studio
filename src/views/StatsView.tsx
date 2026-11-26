import { useEffect, useState } from "react";
import { api } from "../api";
import { useI18n } from "../i18n";
import { errorText, type Count, type Stats } from "../types";

const MONTHS_PL = ["sty", "lut", "mar", "kwi", "maj", "cze", "lip", "sie", "wrz", "paź", "lis", "gru"];
const MONTHS_EN = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

export function StatsView() {
  const { t, locale } = useI18n();
  const [year, setYear] = useState(new Date().getFullYear());
  const [stats, setStats] = useState<Stats | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    api.stats(year).then(setStats).catch((cause) => setMessage(errorText(cause)));
  }, [year]);

  if (!stats) return <p>{message ?? t("loading")}</p>;
  const months = locale === "en" ? MONTHS_EN : MONTHS_PL;
  const peak = Math.max(1, ...stats.byMonth);
  const goalShare = stats.goal ? Math.min(100, Math.round((stats.finished / stats.goal) * 100)) : null;

  return (
    <section>
      <div className="flex flex-wrap items-end justify-between gap-4">
        <h1 className="font-display text-5xl leading-none">{t("statsTitle")}</h1>
        <div className="flex items-center gap-2">
          <button type="button" aria-label={t("previousYear")} onClick={() => setYear(year - 1)} className="border border-rule px-3 py-1.5 dark:border-night-rule">←</button>
          <span className="font-display text-2xl tabular-nums">{year}</span>
          <button type="button" aria-label={t("nextYear")} onClick={() => setYear(year + 1)} className="border border-rule px-3 py-1.5 dark:border-night-rule">→</button>
        </div>
      </div>

      <dl className="mt-8 grid grid-cols-2 gap-4 sm:grid-cols-4">
        <Tile label={t("statsFinished")} value={stats.finished} />
        <Tile label={t("statsPages")} value={stats.pages} />
        <Tile label={t("statsTotal")} value={stats.totalBooks} />
        <Tile label={t("unread")} value={stats.unread} />
        <Tile label={t("filterPaper")} value={stats.paper} />
        <Tile label={t("filterEbook")} value={stats.ebooks} />
        <Tile label={t("filterLoaned")} value={stats.loaned} />
        <Tile label={t("filterOverdue")} value={stats.overdue} alert={stats.overdue > 0} />
      </dl>

      <h2 className="mt-10 font-display text-3xl">{t("readingGoal")}</h2>
      {stats.goal ? (
        <div className="mt-3 max-w-xl">
          <p>{stats.finished} / {stats.goal} ({goalShare}%)</p>
          <div className="mt-2 h-3 w-full bg-rule dark:bg-night-rule" role="progressbar" aria-valuemin={0} aria-valuemax={stats.goal} aria-valuenow={stats.finished}>
            <div className="h-3 bg-moss" style={{ width: `${goalShare}%` }} />
          </div>
        </div>
      ) : (
        <p className="mt-3 text-muted">{t("readingGoalUnset")}</p>
      )}

      <h2 className="mt-10 font-display text-3xl">{t("statsByMonth")}</h2>
      <div className="mt-4 flex h-48 items-end gap-2" role="img" aria-label={stats.byMonth.map((n, i) => `${months[i]}: ${n}`).join(", ")}>
        {stats.byMonth.map((count, index) => (
          <div key={index} className="flex flex-1 flex-col items-center justify-end gap-1">
            <span className="text-xs tabular-nums text-muted">{count || ""}</span>
            <div className="w-full bg-binding dark:bg-moss" style={{ height: `${(count / peak) * 140}px`, minHeight: count ? 4 : 1 }} />
            <span className="text-xs text-muted">{months[index]}</span>
          </div>
        ))}
      </div>

      <div className="mt-10 grid gap-8 sm:grid-cols-3">
        <Ranking title={t("statsGenres")} rows={stats.topGenres} />
        <Ranking title={t("statsAuthors")} rows={stats.topAuthors} />
        <Ranking title={t("statsYears")} rows={stats.byYear} />
      </div>
      {message ? <p className="mt-4 text-stamp">{message}</p> : null}
    </section>
  );
}

function Tile({ label, value, alert }: { label: string; value: number; alert?: boolean }) {
  return (
    <div className="border border-rule p-4 dark:border-night-rule">
      <dt className="text-sm text-muted">{label}</dt>
      <dd className={`mt-1 font-display text-4xl tabular-nums ${alert ? "text-stamp" : ""}`}>{value}</dd>
    </div>
  );
}

function Ranking({ title, rows }: { title: string; rows: Count[] }) {
  const { t } = useI18n();
  const peak = Math.max(1, ...rows.map((row) => row.count));
  return (
    <div>
      <h3 className="font-display text-2xl">{title}</h3>
      {rows.length === 0 ? <p className="mt-2 text-muted">{t("statsEmpty")}</p> : null}
      <ul className="mt-2">
        {rows.map((row) => (
          <li key={row.name} className="py-1">
            <span className="flex justify-between gap-2 text-sm"><span className="truncate">{row.name}</span><span className="tabular-nums">{row.count}</span></span>
            <span className="mt-1 block h-1.5 bg-moss" style={{ width: `${(row.count / peak) * 100}%` }} />
          </li>
        ))}
      </ul>
    </div>
  );
}
