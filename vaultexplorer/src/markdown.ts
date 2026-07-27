// Minimal, hand-rolled Markdown -> HTML renderer for the list-with-preview
// view's .md pane -- not CommonMark-complete, just the handful of things
// the toolbar next to it can produce (headings, bold/italic/underline/
// strikethrough, inline code, links, lists) plus paragraphs. Everything
// is HTML-escaped first; `<u>`/`</u>` is the one tag deliberately let back
// through afterwards, since that's how the Underline button expresses
// itself (Markdown has no native underline syntax).

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function inline(s: string): string {
  let out = escapeHtml(s);
  // A task item's own soft line breaks (Shift+Enter, see parseListBlock's
  // continuation-line handling) survive as literal "\n" in the parsed
  // text -- turn them back into visible breaks instead of letting HTML
  // whitespace-collapse swallow them.
  out = out.replace(/\n/g, "<br>");
  out = out.replace(/&lt;u&gt;/g, "<u>").replace(/&lt;\/u&gt;/g, "</u>");
  out = out.replace(/`([^`]+)`/g, "<code>$1</code>");
  out = out.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  out = out.replace(/__([^_]+)__/g, "<strong>$1</strong>");
  out = out.replace(/~~([^~]+)~~/g, "<del>$1</del>");
  out = out.replace(/\*([^*]+)\*/g, "<em>$1</em>");
  out = out.replace(/(^|[^\w])_([^_]+)_(?!\w)/g, "$1<em>$2</em>");
  // Must run before the plain-link regex right below, since `![alt](src)`
  // would otherwise leave a stray `!` in front of a parsed link. The
  // optional ` =WIDTHx` suffix is how a resize (see `htmlNodeToMarkdown`'s
  // IMG case) survives a save/reload round-trip -- not standard Markdown,
  // but a widely-recognized convention (Obsidian, Pandoc) for the one
  // thing plain `![]()` can't express. `data-md-src` is resolved to an
  // actual displayable `src` later (async, needs a Tauri call to read the
  // file) by the editor pane itself, not here.
  out = out.replace(/!\[([^\]]*)\]\(([^)\s]+)(?:\s+=(\d+)x)?\)/g, (_m, alt, src, width) => {
    // The resize handle lives on this wrapping <span>, not the <img> itself
    // -- WebKitGTK (this app's Linux webview) never draws a resize grip on
    // replaced elements like img/video, only on ordinary block containers.
    const style = width ? ` style="width:${width}px"` : "";
    // A base64 data: URI is self-contained -- render its `src` directly so
    // it shows without the async Tauri resolve step used for file refs (the
    // editor's resolver skips imgs that already have a `src`). data-md-src
    // still carries it so serialization round-trips it back to `![](...)`.
    const imgSrc = src.startsWith("data:") ? ` src="${src}"` : "";
    return `<span class="md-img-wrap"${style}><img class="md-img" data-md-src="${src}" alt="${alt}"${imgSrc}></span>`;
  });
  out = out.replace(/\[([^\]]+)\]\(([^)]+)\)/g, (_m, text, url) => {
    const safe = /^https?:\/\//.test(url) ? url : "#";
    return `<a href="${safe}" target="_blank" rel="noopener noreferrer">${text}</a>`;
  });
  return out;
}

// One parsed list item. `task` is null for a plain bullet/number, or a
// boolean (checked) for a GFM `- [ ]` / `- [x]` task item. Nesting is by
// indentation (two spaces per level on serialize).
type ListItem = { ordered: boolean; task: boolean | null; text: string; children: ListItem[] };

const LIST_LINE = /^(\s*)([-*]|\d+\.)\s+(.*)$/;

// Build a forest of list items from a run of list lines, using an indent
// stack so arbitrary nesting depth works (Tab / Shift+Tab in the editor).
function parseListBlock(lines: string[]): ListItem[] {
  const roots: ListItem[] = [];
  const stack: { indent: number; item: ListItem }[] = [];
  for (const line of lines) {
    const m = line.match(LIST_LINE);
    if (!m) {
      // A continuation line (soft line break inside the previous item,
      // e.g. Shift+Enter in a task) -- fold it into that item's text
      // instead of silently dropping it, which used to make everything
      // after the first line break vanish on save/reload.
      if (stack.length) stack[stack.length - 1].item.text += "\n" + line;
      continue;
    }
    const indent = m[1].replace(/\t/g, "  ").length;
    const ordered = /\d+\./.test(m[2]);
    let text = m[3];
    let task: boolean | null = null;
    const tm = text.match(/^\[([ xX])\]\s+(.*)$/);
    if (tm) {
      task = tm[1].toLowerCase() === "x";
      text = tm[2];
    }
    const item: ListItem = { ordered, task, text, children: [] };
    while (stack.length && stack[stack.length - 1].indent >= indent) stack.pop();
    if (stack.length) stack[stack.length - 1].item.children.push(item);
    else roots.push(item);
    stack.push({ indent, item });
  }
  return roots;
}

function renderListItem(it: ListItem): string {
  const kids = it.children.length ? renderListForest(it.children) : "";
  if (it.task !== null) {
    const checked = it.task;
    // A styled span (contenteditable=false) instead of a real <input>: a
    // form control inside a contentEditable region misbehaves (caret traps,
    // inconsistent click toggling). The editor toggles `data-checked` on
    // click. `md-task-text` keeps the label a single editable run.
    return (
      `<li class="md-task${checked ? " done" : ""}" data-task="1" data-checked="${checked}" draggable="true">` +
      `<span class="md-task-check" contenteditable="false"></span>` +
      `<span class="md-task-text">${inline(it.text)}</span>` +
      kids +
      `</li>`
    );
  }
  return `<li>${inline(it.text)}${kids}</li>`;
}

function renderListForest(items: ListItem[]): string {
  if (!items.length) return "";
  const isTaskList = items.some((i) => i.task !== null);
  const ordered = !isTaskList && items[0].ordered;
  const tag = ordered ? "ol" : "ul";
  const cls = isTaskList ? ' class="md-tasklist"' : "";
  return `<${tag}${cls}>${items.map(renderListItem).join("")}</${tag}>`;
}

// ---- GFM pipe tables ----
// `| a | b |` header, then a `|---|---|` delimiter row (only "-", ":" and
// "|"), then any number of `| ... | ... |` body rows. No escaped-pipe
// support -- matching the rest of this file's "not CommonMark-complete"
// scope.
function splitTableRow(line: string): string[] {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|")) s = s.slice(0, -1);
  return s.split("|").map((c) => c.trim());
}
function isTableDelimiterRow(line: string): boolean {
  if (!line.includes("|") && !line.includes("-")) return false;
  const cells = splitTableRow(line);
  return cells.length > 0 && cells.every((c) => /^:?-+:?$/.test(c));
}
type CellAlign = "left" | "right" | "center" | null;
function cellAlign(delim: string): CellAlign {
  const left = delim.startsWith(":");
  const right = delim.endsWith(":");
  if (left && right) return "center";
  if (right) return "right";
  if (left) return "left";
  return null;
}
function renderTable(headerLine: string, delimLine: string, bodyLines: string[]): string {
  const headers = splitTableRow(headerLine);
  const aligns = splitTableRow(delimLine).map(cellAlign);
  const alignStyle = (i: number) => (aligns[i] ? ` style="text-align:${aligns[i]}"` : "");
  const th = headers.map((h, i) => `<th${alignStyle(i)}>${inline(h)}</th>`).join("");
  const rows = bodyLines
    .map((line) => {
      const cells = splitTableRow(line);
      const td = headers.map((_, i) => `<td${alignStyle(i)}>${inline(cells[i] ?? "")}</td>`).join("");
      return `<tr>${td}</tr>`;
    })
    .join("");
  return `<table class="md-table"><thead><tr>${th}</tr></thead><tbody>${rows}</tbody></table>`;
}

export function renderMarkdownToHtml(src: string): string {
  const lines = src.replace(/\r\n/g, "\n").split("\n");
  const html: string[] = [];
  let paragraph: string[] = [];
  let listRun: string[] = [];

  function flushParagraph() {
    if (paragraph.length) {
      html.push(`<p>${paragraph.map(inline).join("<br>")}</p>`);
      paragraph = [];
    }
  }
  function flushList() {
    if (listRun.length) {
      html.push(renderListForest(parseListBlock(listRun)));
      listRun = [];
    }
  }

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    // A table is recognized by its delimiter row -- the line *after* this
    // one -- so needs one line of lookahead the other block types don't.
    if (line.includes("|") && !LIST_LINE.test(line) && i + 1 < lines.length && isTableDelimiterRow(lines[i + 1])) {
      flushParagraph();
      flushList();
      const bodyLines: string[] = [];
      let j = i + 2;
      while (j < lines.length && lines[j].trim() !== "" && lines[j].includes("|") && !LIST_LINE.test(lines[j])) {
        bodyLines.push(lines[j]);
        j++;
      }
      html.push(renderTable(line, lines[i + 1], bodyLines));
      i = j - 1;
    } else if (LIST_LINE.test(line)) {
      flushParagraph();
      listRun.push(line);
    } else if (heading) {
      flushParagraph();
      flushList();
      const level = heading[1].length;
      html.push(`<h${level}>${inline(heading[2])}</h${level}>`);
    } else if (line.trim() === "") {
      flushParagraph();
      flushList();
    } else {
      flushList();
      paragraph.push(line);
    }
  }
  flushParagraph();
  flushList();
  return html.join("\n");
}

// The other direction, for the editable-preview (WYSIWYG) pane: walks the
// contentEditable's DOM back into markdown source after the user types or
// a toolbar button (execCommand, or this file's own Range-based tag
// wrapping) changes it. Only understands the tags this renderer itself
// produces plus what WebKit's contentEditable/execCommand commonly emits
// for the same operations (<b>/<strong>, <i>/<em>, <s>/<strike>/<del>) --
// arbitrary pasted-in HTML isn't a target here.
function htmlNodeToMarkdown(node: Node): string {
  if (node.nodeType === Node.TEXT_NODE) {
    return node.textContent ?? "";
  }
  if (node.nodeType !== Node.ELEMENT_NODE) return "";
  const el = node as HTMLElement;
  const children = Array.from(el.childNodes).map(htmlNodeToMarkdown).join("");
  switch (el.tagName) {
    case "STRONG":
    case "B":
      return children.trim() ? `**${children}**` : children;
    case "EM":
    case "I":
      return children.trim() ? `*${children}*` : children;
    case "U":
      return children.trim() ? `<u>${children}</u>` : children;
    case "S":
    case "STRIKE":
    case "DEL":
      return children.trim() ? `~~${children}~~` : children;
    case "CODE":
      return children.trim() ? `\`${children}\`` : children;
    case "A":
      return `[${children}](${el.getAttribute("href") ?? ""})`;
    case "IMG": {
      const src = el.getAttribute("data-md-src") ?? "";
      const alt = el.getAttribute("alt") ?? "";
      // The resize handle (see the editor pane's ResizeObserver) sits on the
      // `.md-img-wrap` span around this img, not the img itself -- that's
      // where the live width ends up.
      const wrap = el.parentElement?.classList.contains("md-img-wrap") ? el.parentElement : null;
      const width =
        wrap?.getAttribute("data-md-width") ||
        (wrap?.style.width ? parseInt(wrap.style.width, 10) : 0) ||
        el.getAttribute("data-md-width") ||
        (el.style.width ? parseInt(el.style.width, 10) : 0);
      return width ? `![${alt}](${src} =${width}x)` : `![${alt}](${src})`;
    }
    case "H1":
      return `# ${children}\n\n`;
    case "H2":
      return `## ${children}\n\n`;
    case "H3":
      return `### ${children}\n\n`;
    case "H4":
    case "H5":
    case "H6":
      return `#### ${children}\n\n`;
    case "UL":
    case "OL":
      // Nested lists are handled recursively inside serializeList, so this
      // top-level case only ever runs for a root list.
      return "\n" + serializeList(el, 0) + "\n\n";
    case "LI":
      // A stray LI outside serializeList (shouldn't normally happen) --
      // fall back to its inline content.
      return children;
    case "TABLE":
      return "\n" + serializeTable(el) + "\n\n";
    case "BR":
      return "\n";
    case "DIV":
    case "P":
      return `${children}\n\n`;
    default:
      return children;
  }
}

// Serialize a <table> (from renderTable's <thead>/<tbody> output) back to a
// GFM pipe table. Bypasses the generic `children` join above -- a table's
// row/column structure needs its own delimiters, which plain concatenation
// of cell text can't express (and without this, any edit anywhere else in
// the note would silently flatten every table on the page into a single
// run of unstructured text on the next save).
function serializeTable(table: HTMLElement): string {
  const rows = Array.from(table.querySelectorAll(":scope > thead > tr, :scope > tbody > tr")) as HTMLElement[];
  if (!rows.length) return "";
  const cellText = (c: HTMLElement) =>
    Array.from(c.childNodes)
      .map(htmlNodeToMarkdown)
      .join("")
      .trim()
      .replace(/\|/g, "\\|")
      .replace(/\n+/g, " ");
  const cellAlignDelim = (c: HTMLElement) => {
    const a = c.style.textAlign;
    if (a === "center") return ":-:";
    if (a === "right") return "-:";
    if (a === "left") return ":-";
    return "-";
  };
  const rowLine = (r: HTMLElement) => `| ${Array.from(r.children).map((c) => cellText(c as HTMLElement)).join(" | ")} |`;
  const headerCells = Array.from(rows[0].children) as HTMLElement[];
  const delimLine = `| ${headerCells.map(cellAlignDelim).join(" | ")} |`;
  const lines = [rowLine(rows[0]), delimLine, ...rows.slice(1).map(rowLine)];
  return lines.join("\n");
}

// Serialize a <ul>/<ol> (and any nested lists) back to Markdown, two spaces
// of indent per nesting level. Task items round-trip as `- [ ]` / `- [x]`,
// their checked state read from the `data-checked` the editor toggles.
function serializeList(list: HTMLElement, depth: number): string {
  const ordered = list.tagName === "OL";
  const items = Array.from(list.children).filter((c) => c.tagName === "LI") as HTMLElement[];
  const indent = "  ".repeat(depth);
  return items
    .map((li, i) => {
      let text = "";
      let nested = "";
      for (const c of Array.from(li.childNodes)) {
        if (c.nodeType === Node.ELEMENT_NODE) {
          const cel = c as HTMLElement;
          if (cel.tagName === "UL" || cel.tagName === "OL") {
            nested += "\n" + serializeList(cel, depth + 1);
            continue;
          }
          if (cel.classList?.contains("md-task-check")) continue; // marker, not text
        }
        text += htmlNodeToMarkdown(c);
      }
      const isTask = li.getAttribute("data-task") === "1";
      const bullet = ordered ? `${i + 1}.` : "-";
      const check = isTask ? (li.getAttribute("data-checked") === "true" ? "[x] " : "[ ] ") : "";
      return `${indent}${bullet} ${check}${text.trim()}${nested}`;
    })
    .join("\n");
}

export function serializePreviewToMarkdown(root: HTMLElement): string {
  return Array.from(root.childNodes)
    .map(htmlNodeToMarkdown)
    .join("")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
