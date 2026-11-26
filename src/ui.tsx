import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { useI18n } from "./i18n";

type ConfirmOptions = { title?: string; confirmLabel?: string; danger?: boolean };
type Toast = { id: number; text: string; action?: { label: string; run: () => void } };

type UiValue = {
  /** Okno potwierdzenia w aplikacji. Zwraca true, gdy użytkownik potwierdzi. */
  confirm: (message: string, options?: ConfirmOptions) => Promise<boolean>;
  /** Krótka informacja na dole okna, opcjonalnie z przyciskiem (np. „Cofnij”). */
  notify: (text: string, action?: Toast["action"]) => void;
  /** Czy jest otwarte okno, które przechwytuje klawisz Escape. */
  modalOpen: () => boolean;
};

const UiContext = createContext<UiValue | null>(null);

type Pending = { message: string; options: ConfirmOptions; resolve: (value: boolean) => void };

export function UiProvider({ children }: { children: ReactNode }) {
  const { t } = useI18n();
  const [pending, setPending] = useState<Pending | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const pendingRef = useRef<Pending | null>(null);
  const confirmButton = useRef<HTMLButtonElement>(null);
  const nextId = useRef(1);

  const confirm = useCallback((message: string, options: ConfirmOptions = {}) => {
    return new Promise<boolean>((resolve) => {
      const next = { message, options, resolve };
      pendingRef.current = next;
      setPending(next);
    });
  }, []);

  const close = useCallback((value: boolean) => {
    pendingRef.current?.resolve(value);
    pendingRef.current = null;
    setPending(null);
  }, []);

  const notify = useCallback((text: string, action?: Toast["action"]) => {
    const id = nextId.current++;
    setToasts((current) => [...current.slice(-2), { id, text, action }]);
    window.setTimeout(() => setToasts((current) => current.filter((toast) => toast.id !== id)), action ? 9000 : 5000);
  }, []);

  useEffect(() => {
    if (!pending) return;
    confirmButton.current?.focus();
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        close(false);
      }
    }
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [pending, close]);

  const value: UiValue = { confirm, notify, modalOpen: () => pendingRef.current !== null };

  return (
    <UiContext.Provider value={value}>
      {children}
      {pending ? (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-ink/40 p-6 print:hidden" role="presentation" onClick={() => close(false)}>
          <div
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="confirm-title"
            className="w-full max-w-md bg-sheet p-6 text-ink shadow-xl dark:bg-night-sheet dark:text-night-ink"
            onClick={(event) => event.stopPropagation()}
          >
            <h2 id="confirm-title" className="font-display text-2xl">{pending.options.title ?? t("confirmTitle")}</h2>
            <p className="mt-3 whitespace-pre-line">{pending.message}</p>
            <div className="mt-6 flex justify-end gap-2">
              <button type="button" className="border border-rule px-4 py-2 dark:border-night-rule" onClick={() => close(false)}>{t("cancel")}</button>
              <button
                ref={confirmButton}
                type="button"
                className={pending.options.danger ? "bg-stamp px-4 py-2 text-sheet" : "bg-binding px-4 py-2 text-sheet"}
                onClick={() => close(true)}
              >
                {pending.options.confirmLabel ?? t("confirmYes")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
      <div className="pointer-events-none fixed bottom-4 right-4 z-40 flex max-w-sm flex-col gap-2 print:hidden" aria-live="polite">
        {toasts.map((toast) => (
          <div key={toast.id} className="pointer-events-auto flex items-center gap-3 bg-binding px-4 py-3 text-sheet shadow-lg">
            <span className="flex-1">{toast.text}</span>
            {toast.action ? (
              <button
                type="button"
                className="font-medium underline"
                onClick={() => {
                  toast.action?.run();
                  setToasts((current) => current.filter((item) => item.id !== toast.id));
                }}
              >
                {toast.action.label}
              </button>
            ) : null}
          </div>
        ))}
      </div>
    </UiContext.Provider>
  );
}

export function useUi(): UiValue {
  const value = useContext(UiContext);
  if (!value) throw new Error("UiProvider is missing");
  return value;
}

/** Czy zdarzenie klawisza pochodzi z pola edycji (wtedy skróty globalne nie działają). */
export function typingInField(event: KeyboardEvent): boolean {
  const target = event.target as HTMLElement | null;
  if (!target) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable;
}
