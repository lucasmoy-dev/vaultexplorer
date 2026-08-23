import { MusicTrack } from "./api";

// The parts of the Music view that are decisions rather than drawing:
// which tracks are on screen, in what order, and what "next" means.
//
// Separated from MusicView so they can be tested without a DOM: every one
// of these rules came out of a bug report ("it doesn't go to the next song",
// "the list jumps", "most played does nothing"), and a rule that is only
// exercised by tapping a phone is a rule nobody checks again.

export type Order = "folder" | "most-played";

export function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  const minutes = Math.floor(total / 60);
  const rest = total % 60;
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}

/** The title to show: the tag if there is one, the file name without its
 *  extension otherwise -- a phone full of downloads is mostly the latter. */
export function displayTitle(track: MusicTrack): string {
  return track.title?.trim() || track.name.replace(/\.[^.]+$/, "");
}

export function displaySubtitle(track: MusicTrack): string {
  const parts: string[] = [];
  if (track.artist?.trim()) parts.push(track.artist.trim());
  if (track.album?.trim()) parts.push(track.album.trim());
  if (track.year) parts.push(String(track.year));
  return parts.join(" · ");
}

/** Folders that actually contain audio, each with how many, sorted by path. */
export function folderCounts(tracks: MusicTrack[]): [string, number][] {
  const counts = new Map<string, number>();
  for (const track of tracks) counts.set(track.folder, (counts.get(track.folder) ?? 0) + 1);
  return [...counts.entries()].sort((a, b) => a[0].localeCompare(b[0]));
}

/** The list as shown: one folder or all of them, in file order or by plays. */
export function orderTracks(
  tracks: MusicTrack[],
  folder: string | null,
  order: Order,
): MusicTrack[] {
  const inFolder = folder === null ? tracks : tracks.filter((t) => t.folder === folder);
  if (order !== "most-played") return inFolder;
  return [...inFolder].sort(
    (a, b) => b.plays - a.plays || displayTitle(a).localeCompare(displayTitle(b)),
  );
}

/** The queue for "play the ones I listen to most", commonest first. */
export function mostPlayed(tracks: MusicTrack[]): MusicTrack[] {
  return tracks.filter((t) => t.plays > 0).sort((a, b) => b.plays - a.plays);
}

export interface StepInput {
  index: number;
  length: number;
  delta: number;
  shuffle: boolean;
  repeat: boolean;
  /** Injected so shuffle is testable; `Math.random` in the app. */
  random?: () => number;
}

/**
 * Where the queue goes next, or `null` for "stop here".
 *
 * The end of the queue is the interesting case: without `repeat` it stops
 * (and the caller shows it paused), and it must never wrap silently -- an
 * album that starts itself over when it ends is the other half of the same
 * complaint as one that stops after track one.
 */
export function stepIndex({
  index,
  length,
  delta,
  shuffle,
  repeat,
  random = Math.random,
}: StepInput): number | null {
  if (length === 0) return null;
  if (shuffle && delta > 0) {
    if (length === 1) return 0;
    // Offset by 1..length-1 so the next track is never the one just played:
    // picking uniformly repeats it one time in N, which reads as a button
    // that did nothing.
    return (index + 1 + Math.floor(random() * (length - 1))) % length;
  }
  const next = index + delta;
  if (next < 0) return 0;
  if (next >= length) return repeat ? 0 : null;
  return next;
}
