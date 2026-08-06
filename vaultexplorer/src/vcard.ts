// Minimal vCard (RFC 2426) reader/writer -- enough for a contacts-list
// preview (name + phone numbers + photo) and a real field-by-field editor,
// not a full parser/generator. Real-world exports from Android/iOS/Google
// Contacts fold long fields (PHOTO's base64 payload especially, always
// several KB) across multiple lines -- each continuation line starts with
// a single space or tab and is meant to be concatenated onto the previous
// line with that leading whitespace stripped (RFC 2426 §2.6) -- so
// unfolding has to happen before parsing each line's KEY:VALUE, not per
// raw line, and serializing back has to fold the same way for anything
// that reads long lines.
export type ParsedVCard = {
  name: string;
  phones: string[];
  emails: string[];
  org: string;
  note: string;
  photoDataUrl: string | null;
};

export function emptyVCard(): ParsedVCard {
  return { name: "", phones: [], emails: [], org: "", note: "", photoDataUrl: null };
}

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

// RFC 2426 §5.8.4 backslash-escapes for TEXT values: comma and semicolon
// (both used as separators in structured values), backslash itself, and a
// literal newline as "\n". Applied here regardless of whether *this*
// particular field is structured -- real-world exports escape TEXT values
// uniformly, so unescaping the same way on read is what actually round-
// trips them.
function unescapeValue(v: string): string {
  let out = "";
  for (let i = 0; i < v.length; i++) {
    if (v[i] === "\\" && i + 1 < v.length) {
      const next = v[i + 1];
      if (next === "n" || next === "N") {
        out += "\n";
        i++;
        continue;
      }
      if (next === "," || next === ";" || next === "\\") {
        out += next;
        i++;
        continue;
      }
    }
    out += v[i];
  }
  return out;
}

// Full escaping for a free-text value (name/phone/email/note) -- every
// reserved char gets escaped.
function escapeText(v: string): string {
  return v.replace(/\\/g, "\\\\").replace(/\n/g, "\\n").replace(/,/g, "\\,").replace(/;/g, "\\;");
}

// ORG's ";" is a real component separator (Company;Department), not
// literal text -- escaping it here would silently merge those components
// on the next save. Everything else about a TEXT value still applies.
function escapeStructured(v: string): string {
  return v.replace(/\\/g, "\\\\").replace(/\n/g, "\\n").replace(/,/g, "\\,");
}

export function parseVCard(text: string): ParsedVCard {
  let name = "";
  const phones: string[] = [];
  const emails: string[] = [];
  let org = "";
  let note = "";
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
      name = unescapeValue(value);
    } else if (key === "N" && !name) {
      // N is "Family;Given;Middle;Prefix;Suffix" -- reassemble as "Given Family".
      const parts = value.split(";").map(unescapeValue);
      name = [parts[1], parts[0]].filter(Boolean).join(" ");
    } else if (key === "TEL") {
      phones.push(unescapeValue(value));
    } else if (key === "EMAIL") {
      emails.push(unescapeValue(value));
    } else if (key === "ORG" && !org) {
      org = unescapeValue(value);
    } else if (key === "NOTE" && !note) {
      note = unescapeValue(value);
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
  return { name, phones, emails, org, note, photoDataUrl };
}

// RFC 2426 §2.6: content lines over 75 characters should be folded, each
// continuation starting with exactly one space. This is the exact inverse
// of `unfold` above (which strips exactly one leading whitespace char per
// continuation and appends the rest) -- any fold width works as long as
// that pairing holds, since this app only ever has to read back what it
// itself wrote (or what unfold already knows how to read from anyone
// else's export).
const FOLD_LIMIT = 75;
function foldLine(line: string): string {
  if (line.length <= FOLD_LIMIT) return line;
  const parts = [line.slice(0, FOLD_LIMIT)];
  let rest = line.slice(FOLD_LIMIT);
  while (rest.length > 0) {
    parts.push(" " + rest.slice(0, FOLD_LIMIT - 1));
    rest = rest.slice(FOLD_LIMIT - 1);
  }
  return parts.join("\r\n");
}

// The inverse of parseVCard -- serializes edited fields back to valid
// vCard 3.0 text. Phones/emails with only whitespace are dropped rather
// than written as empty TEL:/EMAIL: lines.
export function serializeVCard(parsed: ParsedVCard): string {
  const name = parsed.name.trim();
  const lines: string[] = ["BEGIN:VCARD", "VERSION:3.0"];
  lines.push(`FN:${escapeText(name || "New Contact")}`);
  if (name) {
    // No structured given/family split is offered in the editor -- treat
    // the whole name as the family component so it still shows up as a
    // real display name (not blank) in apps that prefer N over FN.
    lines.push(`N:${escapeText(name)};;;;`);
  }
  for (const phone of parsed.phones) {
    if (phone.trim()) lines.push(`TEL;TYPE=CELL:${escapeText(phone.trim())}`);
  }
  for (const email of parsed.emails) {
    if (email.trim()) lines.push(`EMAIL:${escapeText(email.trim())}`);
  }
  if (parsed.org.trim()) lines.push(`ORG:${escapeStructured(parsed.org.trim())}`);
  if (parsed.note.trim()) lines.push(`NOTE:${escapeText(parsed.note.trim())}`);
  if (parsed.photoDataUrl) lines.push(`PHOTO:${parsed.photoDataUrl}`);
  lines.push("END:VCARD");
  return lines.map(foldLine).join("\r\n") + "\r\n";
}

// Digits and a leading "+" only -- what tel:/wa.me links want, whatever
// formatting (spaces, dashes, parens) the vCard itself used.
export function cleanPhoneForLink(phone: string): string {
  return phone.replace(/[^\d+]/g, "");
}
