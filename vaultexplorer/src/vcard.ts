// Minimal vCard (RFC 2426) reader -- just enough for a contacts-list
// preview (name + phone numbers), not a full parser. Deliberately doesn't
// handle folded (continuation) lines or charset/encoding params; real-world
// exports from Android/iOS/Google Contacts don't fold short fields like
// FN/TEL, so this covers the common case without the extra complexity.
export type ParsedVCard = { name: string; phones: string[] };

export function parseVCard(text: string): ParsedVCard {
  let name = "";
  const phones: string[] = [];
  for (const rawLine of text.split(/\r\n|\n|\r/)) {
    const line = rawLine.trim();
    if (!line) continue;
    const colon = line.indexOf(":");
    if (colon < 0) continue;
    const key = line.slice(0, colon).split(";")[0].toUpperCase();
    const value = line.slice(colon + 1).trim();
    if (key === "FN" && !name) {
      name = value;
    } else if (key === "N" && !name) {
      // N is "Family;Given;Middle;Prefix;Suffix" -- reassemble as "Given Family".
      const parts = value.split(";");
      name = [parts[1], parts[0]].filter(Boolean).join(" ");
    } else if (key === "TEL") {
      phones.push(value);
    }
  }
  return { name: name || "(no name)", phones };
}

// Digits and a leading "+" only -- what tel:/wa.me links want, whatever
// formatting (spaces, dashes, parens) the vCard itself used.
export function cleanPhoneForLink(phone: string): string {
  return phone.replace(/[^\d+]/g, "");
}
