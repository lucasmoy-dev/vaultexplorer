import { writeText } from "@tauri-apps/plugin-clipboard-manager";

/**
 * Puts text on the clipboard, by whichever route works here.
 *
 * The Tauri plugin talks to the system clipboard through the host toolkit, and
 * on this Linux/WebKitGTK setup that path fails. The webview has its own,
 * older route that does work, so we try the good one first and fall back rather
 * than telling the user to select 80 characters of base64 by hand.
 */
export async function copyText(text: string): Promise<boolean> {
  // The webview's own API first: a Tauri window is a secure context, and this
  // rejects honestly when it fails.
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Fall through.
    }
  }

  try {
    await writeText(text);
    return true;
  } catch {
    // Fall through: the host toolkit's clipboard is not reachable everywhere.
  }

  return copyViaWebview(text);
}

/**
 * The old selection-based route.
 *
 * WebKitGTK has been observed to put the text on the clipboard and still return
 * false from execCommand, so only a thrown error is treated as failure here.
 * Claiming success wrongly is bad; telling someone to hand-copy 160 characters
 * that are already on their clipboard is worse.
 */
function copyViaWebview(text: string): boolean {
  const field = document.createElement("textarea");
  field.value = text;
  field.setAttribute("readonly", "");
  // Off-screen but still selectable; `display: none` cannot be selected.
  field.style.position = "fixed";
  field.style.top = "0";
  field.style.opacity = "0";
  field.style.pointerEvents = "none";
  document.body.appendChild(field);

  const previous = document.getSelection()?.rangeCount
    ? document.getSelection()!.getRangeAt(0)
    : null;

  let copied = true;
  try {
    field.select();
    field.setSelectionRange(0, field.value.length);
    document.execCommand("copy");
  } catch {
    copied = false;
  } finally {
    document.body.removeChild(field);
    if (previous) {
      const selection = document.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(previous);
    }
  }
  return copied;
}
