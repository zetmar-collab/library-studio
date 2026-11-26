import type { ReactNode } from "react";
import { useI18n, type MessageKey } from "./i18n";
import type { ViewId } from "./types";

const NAV: { id: ViewId; label: MessageKey }[] = [
  { id: "library", label: "navLibrary" },
  { id: "shelves", label: "navShelves" },
  { id: "files", label: "navFiles" },
  { id: "reader", label: "navReader" },
  { id: "stats", label: "navStats" },
  { id: "tools", label: "navTools" },
  { id: "backup", label: "navBackup" },
  { id: "settings", label: "navSettings" },
];

type ShellProps = {
  view: ViewId;
  onView: (view: ViewId) => void;
  onAdd: () => void;
  children: ReactNode;
};

export function Shell({ view, onView, onAdd, children }: ShellProps) {
  const { t } = useI18n();
  return (
    <div className="grid h-full grid-cols-1 bg-paper text-ink md:grid-cols-[232px_minmax(0,1fr)] print:block dark:bg-night dark:text-night-ink">
      <aside className="flex flex-col print:hidden bg-binding text-sheet md:min-h-full dark:bg-binding-deep">
        <div className="px-6 pt-8 pb-6">
          <p className="font-display text-3xl leading-none tracking-tight">Library Studio</p>
          <p className="mt-3 max-w-[18ch] text-sm text-sheet/75">
            {t("appTagline")}
          </p>
        </div>
        <nav className="flex gap-1 overflow-x-auto px-3 pb-4 md:flex-col md:overflow-visible md:px-3">
          {NAV.map((item) => {
            const active = item.id === view;
            return (
              <button
                key={item.id}
                type="button"
                onClick={() => onView(item.id)}
                className={
                  active
                    ? "rounded-sm bg-sheet/15 px-3 py-2 text-left text-sheet"
                    : "rounded-sm px-3 py-2 text-left text-sheet/75 hover:bg-sheet/10 hover:text-sheet"
                }
                aria-current={active ? "page" : undefined}
              >
                {t(item.label)}
              </button>
            );
          })}
        </nav>
        <div className="mt-auto hidden px-6 py-6 md:block">
          <button
            type="button"
            onClick={onAdd}
            className="w-full bg-stamp px-3 py-2 text-sheet hover:bg-[#742624]"
          >
            {t("addBook")}
          </button>
        </div>
      </aside>
      <main className="min-h-0 overflow-auto print:overflow-visible">
        <div className="mx-auto flex max-w-4xl items-start justify-between gap-4 px-6 pt-6 md:hidden print:hidden">
          <button
            type="button"
            onClick={onAdd}
            className="bg-stamp px-3 py-2 text-sheet"
          >
            {t("addBook")}
          </button>
        </div>
        <div className="mx-auto max-w-5xl px-6 py-8 md:px-10 md:py-10 print:max-w-none print:p-0">{children}</div>
      </main>
    </div>
  );
}
