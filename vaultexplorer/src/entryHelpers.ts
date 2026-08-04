import { Entry } from "./api";
import { kindOf } from "./icons";

// The extension whose "open in the text editor" setting applies to this entry
// (lowercase, no dot), or `null` when the question doesn't apply at all: a
// folder, a file with no extension, a format already edited as text (.txt,
// .md), or one we know is binary -- loading an image/video/archive/office
// file into a text editor and saving it back would corrupt it.
export function editorExtOf(entry: Entry): string | null {
  if (entry.is_dir) return null;
  const dot = entry.name.lastIndexOf(".");
  if (dot <= 0) return null;
  const kind = kindOf(entry);
  if (kind !== "code" && kind !== "generic") return null;
  const ext = entry.name.slice(dot + 1).toLowerCase();
  return ext || null;
}

export function kindLabel(entry: Entry): string {
  if (entry.is_vault) return "Vault";
  if (entry.is_dir) return "Folder";
  const ext = entry.name.includes(".") ? entry.name.split(".").pop()!.toUpperCase() : "";
  return ext ? `${ext} Document` : "Document";
}

// "Hide extensions" is purely cosmetic -- the real name (used for every
// rename/API call) never changes, only what's painted in the tile.
export function displayEntryName(entry: Entry, hideExtensions: boolean): string {
  if (!hideExtensions || entry.is_dir) return entry.name;
  const dot = entry.name.lastIndexOf(".");
  return dot > 0 ? entry.name.slice(0, dot) : entry.name;
}
