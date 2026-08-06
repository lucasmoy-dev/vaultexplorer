// Minimal vCard (RFC 2426) reader -- just enough for a contacts-list
// preview (name + phone numbers + photo), not a full parser. Real-world
// exports from Android/iOS/Google Contacts fold long fields (PHOTO's
// base64 payload especially, always several KB) across multiple lines --
// each continuation line starts with a single space or tab and is meant
// to be concatenated onto the previous line with that leading whitespace
// stripped (RFC 2426 §2.6) -- so unfolding has to happen before parsing
// each line's KEY:VALUE, not per raw line.
export type ParsedVCard = { name: string; phones: string[]; photoDataUrl: string | null };

function unfold(text: string): string[] {
  const rawLines = text.split(/\r\n|\n|\r/);
  const lines: string[] = [];
  for (const raw of rawLines) {
    if ((raw.startsWith(" ") || raw.startsWith("\t")) && lines.length > 0) {
      lines[lines.length - 1] += raw.slice(1);
    } else {
      lines.push(raw);
    }
  }
  return lines;
}

export function parseVCard(text: string): ParsedVCard {
  let name = "";
  const phones: string[] = [];
  let photoDataUrl: string | null = null;
  for (const rawLine of unfold(text)) {
    const line = rawLine.trim();
    if (!line) continue;
    const colon = line.indexOf(":");
    if (colon < 0) continue;
    const keyParams = line.slice(0, colon).split(";");
    const key = keyParams[0].toUpperCase();
    const params = keyParams.slice(1).map((p) => p.toUpperCase());
    const value = line.slice(colon + 1).trim();
    if (key === "FN" && !name) {
      name = value;
    } else if (key === "N" && !name) {
      // N is "Family;Given;Middle;Prefix;Suffix" -- reassemble as "Given Family".
      const parts = value.split(";");
      name = [parts[1], parts[0]].filter(Boolean).join(" ");
    } else if (key === "TEL") {
      phones.push(value);
    } else if (key === "PHOTO" && !photoDataUrl) {
      // vCard 2.1 exports (what Android's own as_vcard produces) write
      // `PHOTO;ENCODING=BASE64;TYPE=JPEG:<data>` with no data: URI --
      // vCard 4.0 can instead use `PHOTO:data:image/jpeg;base64,<data>`
      // or a plain http(s) URL. Anything that isn't inline base64 (a
      // remote URL) is skipped -- fetching it here would mean this app's
      // contacts list making outbound requests just to render a list.
      if (value.startsWith("data:")) {
        photoDataUrl = value;
      } else if (params.some((p) => p.includes("BASE64") || p === "ENCODING=B")) {
        const type = params.find((p) => p.startsWith("TYPE="))?.slice(5).toLowerCase() || "jpeg";
        const b64 = value.replace(/\s+/g, "");
        if (b64) photoDataUrl = `data:image/${type};base64,${b64}`;
      }
    }
  }
  return { name: name || "(no name)", phones, photoDataUrl };
}

// Digits and a leading "+" only -- what tel:/wa.me links want, whatever
// formatting (spaces, dashes, parens) the vCard itself used.
export function cleanPhoneForLink(phone: string): string {
  return phone.replace(/[^\d+]/g, "");
}
