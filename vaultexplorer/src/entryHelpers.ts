import { Entry } from "./api";

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
