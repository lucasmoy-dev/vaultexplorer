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
    const style = width ? ` style="width:${width}px"` : "";
    return `<img class="md-img" data-md-src="${src}" alt="${alt}"${style}>`;
  });
  out = out.replace(/\[([^\]]+)\]\(([^)]+)\)/g, (_m, text, url) => {
    const safe = /^https?:\/\//.test(url) ? url : "#";
    return `<a href="${safe}" target="_blank" rel="noopener noreferrer">${text}</a>`;
  });
  return out;
}

export function renderMarkdownToHtml(src: string): string {
  const lines = src.replace(/\r\n/g, "\n").split("\n");
  const html: string[] = [];
  let inList: "ul" | "ol" | null = null;
  let paragraph: string[] = [];

  function flushParagraph() {
    if (paragraph.length) {
      html.push(`<p>${paragraph.map(inline).join("<br>")}</p>`);
      paragraph = [];
    }
  }
  function closeList() {
    if (inList) {
      html.push(`</${inList}>`);
      inList = null;
    }
  }

  for (const line of lines) {
    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    const ul = line.match(/^[-*]\s+(.*)$/);
    const ol = line.match(/^\d+\.\s+(.*)$/);

    if (heading) {
      flushParagraph();
      closeList();
      const level = heading[1].length;
      html.push(`<h${level}>${inline(heading[2])}</h${level}>`);
    } else if (ul) {
      flushParagraph();
      if (inList !== "ul") {
        closeList();
        html.push("<ul>");
        inList = "ul";
      }
      html.push(`<li>${inline(ul[1])}</li>`);
    } else if (ol) {
      flushParagraph();
      if (inList !== "ol") {
        closeList();
        html.push("<ol>");
        inList = "ol";
      }
      html.push(`<li>${inline(ol[1])}</li>`);
    } else if (line.trim() === "") {
      flushParagraph();
      closeList();
    } else {
      closeList();
      paragraph.push(line);
    }
  }
  flushParagraph();
  closeList();
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
      // A resize handle (see the editor pane's ResizeObserver) writes the
      // live width into this same attribute -- read it back from there
      // first, falling back to whatever inline width the element already
      // has (e.g. still mid-resize, observer callback hasn't landed yet).
      const width = el.getAttribute("data-md-width") || (el.style.width ? parseInt(el.style.width, 10) : 0);
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
      return (
        Array.from(el.children)
          .map((li) => `- ${htmlNodeToMarkdown(li)}`)
          .join("\n") + "\n\n"
      );
    case "OL":
      return (
        Array.from(el.children)
          .map((li, i) => `${i + 1}. ${htmlNodeToMarkdown(li)}`)
          .join("\n") + "\n\n"
      );
    case "LI":
      return children;
    case "BR":
      return "\n";
    case "DIV":
    case "P":
      return `${children}\n\n`;
    default:
      return children;
  }
}

export function serializePreviewToMarkdown(root: HTMLElement): string {
  return Array.from(root.childNodes)
    .map(htmlNodeToMarkdown)
    .join("")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
