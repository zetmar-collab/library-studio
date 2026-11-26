const MAP: Record<string, string> = {
  ą: "a", ć: "c", ę: "e", ł: "l", ń: "n", ó: "o", ś: "s", ż: "z", ź: "z",
  Ą: "a", Ć: "c", Ę: "e", Ł: "l", Ń: "n", Ó: "o", Ś: "s", Ż: "z", Ź: "z",
};

export function fold(value: string): string {
  return value
    .split("")
    .map((char) => MAP[char] ?? char.toLowerCase())
    .join("")
    .replace(/[^a-z0-9]+/g, " ")
    .trim();
}

export function highlight(text: string, query: string): Array<{ text: string; hit: boolean }> {
  const needle = fold(query);
  if (!needle) return [{ text, hit: false }];
  let folded = "";
  const starts: number[] = [];
  for (let index = 0; index < text.length; index += 1) {
    const piece = fold(text[index] ?? "");
    if (!piece) continue;
    for (const _char of piece) {
      starts.push(index);
      folded += _char;
    }
  }
  const at = folded.indexOf(needle);
  if (at < 0) return [{ text, hit: false }];
  const from = starts[at] ?? 0;
  const to = (starts[at + needle.length - 1] ?? from) + 1;
  return [
    { text: text.slice(0, from), hit: false },
    { text: text.slice(from, to), hit: true },
    { text: text.slice(to), hit: false },
  ].filter((part) => part.text);
}

export function lines(value: string): string[] {
  return value.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
}
