import { describe, expect, it } from "vitest";
import { ean13CheckDigit, ean13Modules, toIsbn13 } from "./barcode";
import { fold, highlight, lines } from "./text";

describe("EAN-13", () => {
  it("liczy cyfrę kontrolną", () => {
    expect(ean13CheckDigit("978014032872")).toBe(1);
  });

  it("zamienia ISBN-10 na ISBN-13 i odrzuca błędny numer", () => {
    expect(toIsbn13("0-14-032872-6")).toBe("9780140328721");
    expect(toIsbn13("9780140328721")).toBe("9780140328721");
    expect(toIsbn13("9780140328720")).toBeNull();
    expect(toIsbn13("abc")).toBeNull();
  });

  it("daje 95 modułów z ramkami startu, środka i końca", () => {
    const bits = ean13Modules("9780140328721");
    expect(bits).not.toBeNull();
    expect(bits).toHaveLength(95);
    expect(bits?.startsWith("101")).toBe(true);
    expect(bits?.endsWith("101")).toBe(true);
    expect(bits?.slice(45, 50)).toBe("01010");
  });
});

describe("tekst", () => {
  it("składa polskie litery", () => {
    expect(fold("Żółć i Łódź")).toBe("zolc i lodz");
  });

  it("podświetla trafienie bez względu na ogonki", () => {
    const parts = highlight("Pan Tadeusz, czyli Ostatni zajazd na Litwie", "zajazd");
    expect(parts.find((part) => part.hit)?.text).toBe("zajazd");
    expect(highlight("Łódź", "lodz").find((part) => part.hit)?.text).toBe("Łódź");
  });

  it("dzieli tekst na niepuste linie", () => {
    expect(lines(" a \r\n\n b ")).toEqual(["a", "b"]);
  });
});
