import { useState } from "react";
import { api } from "./api";
import { useI18n, type MessageKey } from "./i18n";
import { errorText, type CoverCandidate } from "./types";

const SOURCE: Record<string, MessageKey> = {
  google: "sourceGoogle",
  bn: "sourceBn",
  openlibrary: "sourceOpenLibrary",
};

export function useSourceLabel(): (source: string) => string {
  const { t } = useI18n();
  return (source) => (SOURCE[source] ? t(SOURCE[source]) : source);
}

type CoverPickerProps = {
  title: string;
  author: string;
  isbn?: string;
  onPick: (url: string) => void;
};

export function CoverPicker({ title, author, isbn, onPick }: CoverPickerProps) {
  const { t } = useI18n();
  const sourceLabel = useSourceLabel();
  const [covers, setCovers] = useState<Array<CoverCandidate & { preview: string }>>([]);
  const [address, setAddress] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function search() {
    setBusy(true);
    setMessage(null);
    try {
      const found = await api.searchCovers({ title: title || undefined, author: author || undefined, isbn: isbn || undefined });
      const previews = await Promise.all(
        found.map(async (cover) => ({ ...cover, preview: await api.previewCover(cover.url).catch(() => "") })),
      );
      const visible = previews.filter((cover) => cover.preview);
      setCovers(visible);
      if (visible.length === 0) setMessage(t("noCover"));
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(false);
    }
  }

  async function useAddress() {
    setBusy(true);
    setMessage(null);
    try {
      await api.previewCover(address);
      onPick(address);
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mt-4">
      <div className="flex flex-wrap gap-2">
        <button type="button" className="bg-binding px-3 py-1.5 text-sheet" disabled={busy} onClick={() => void search()}>
          {busy ? t("searchingCover") : t("searchCover")}
        </button>
      </div>
      {covers.length > 0 ? (
        <ul className="mt-3 flex flex-wrap gap-3">
          {covers.map((cover) => (
            <li key={cover.url}>
              <button type="button" className="block text-left" onClick={() => onPick(cover.url)}>
                <img src={cover.preview} alt="" className="h-32 w-20 object-cover" />
                <span className="mt-1 block max-w-20 text-xs text-muted">{sourceLabel(cover.source)}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      <form className="mt-3 flex gap-2" onSubmit={(event) => { event.preventDefault(); void useAddress(); }}>
        <input value={address} onChange={(event) => setAddress(event.target.value)} placeholder={t("coverUrl")} className="w-full border border-rule bg-sheet px-3 py-2 dark:border-night-rule dark:bg-night" />
        <button type="submit" className="border border-rule px-3 py-2" disabled={busy || !address.trim()}>{t("useCover")}</button>
      </form>
      {message ? <p className="mt-2 text-sm text-muted">{message}</p> : null}
    </div>
  );
}
