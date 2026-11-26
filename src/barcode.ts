// Kod kreskowy EAN-13 (ISBN-13) jako ciąg modułów 0/1, do narysowania w SVG.

const L = ["0001101", "0011001", "0010011", "0111101", "0100011", "0110001", "0101111", "0111011", "0110111", "0001011"];
const G = ["0100111", "0110011", "0011011", "0100001", "0011101", "0111001", "0000101", "0010001", "0001001", "0010111"];
const R = ["1110010", "1100110", "1101100", "1000010", "1011100", "1001110", "1010000", "1000100", "1001000", "1110100"];
const PARITY = ["LLLLLL", "LLGLGG", "LLGGLG", "LLGGGL", "LGLLGG", "LGGLLG", "LGGGLL", "LGLGLG", "LGLGGL", "LGGLGL"];

export function ean13CheckDigit(first12: string): number {
  let sum = 0;
  for (let index = 0; index < 12; index += 1) {
    const digit = Number(first12[index]);
    sum += index % 2 === 0 ? digit : digit * 3;
  }
  return (10 - (sum % 10)) % 10;
}

/** ISBN-10 → ISBN-13 (978…). Zwraca null dla nieprawidłowego numeru. */
export function toIsbn13(raw: string): string | null {
  const value = raw.replace(/[^0-9Xx]/g, "").toUpperCase();
  if (value.length === 13 && /^\d{13}$/.test(value)) {
    return ean13CheckDigit(value.slice(0, 12)) === Number(value[12]) ? value : null;
  }
  if (value.length === 10) {
    const body = `978${value.slice(0, 9)}`;
    return `${body}${ean13CheckDigit(body)}`;
  }
  return null;
}

/** Moduły kodu EAN-13 (95 znaków „0”/„1”) albo null, gdy numer jest błędny. */
export function ean13Modules(code: string): string | null {
  const digits = toIsbn13(code);
  if (!digits) return null;
  const first = Number(digits[0]);
  const parity = PARITY[first] ?? "LLLLLL";
  let bits = "101";
  for (let index = 1; index <= 6; index += 1) {
    const digit = Number(digits[index]);
    bits += parity[index - 1] === "L" ? L[digit] : G[digit];
  }
  bits += "01010";
  for (let index = 7; index <= 12; index += 1) {
    bits += R[Number(digits[index])];
  }
  bits += "101";
  return bits;
}
