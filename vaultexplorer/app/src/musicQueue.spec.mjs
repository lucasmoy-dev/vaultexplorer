// A plain node script, because this project has no test runner and adding
// one for six rules is worse than running them. Compiled by tsc into
// build/spec first (see the command in README's "Verifying").
import assert from "node:assert/strict";
import { stepIndex, orderTracks, folderCounts, mostPlayed, displayTitle, displaySubtitle, formatTime } from "./musicQueue.js";

const track = (over) => ({
  path: "/m/" + (over.name ?? "x.mp3"),
  name: over.name ?? "x.mp3",
  folder: over.folder ?? "",
  title: over.title ?? null,
  artist: over.artist ?? null,
  album: over.album ?? null,
  year: over.year ?? null,
  track_no: null,
  duration_secs: over.duration_secs ?? null,
  has_art: false,
  plays: over.plays ?? 0,
});

// The bug that started all of this: the queue has to move on by itself, and
// it has to stop at the end instead of starting over.
assert.equal(stepIndex({ index: 0, length: 3, delta: 1, shuffle: false, repeat: false }), 1);
assert.equal(stepIndex({ index: 2, length: 3, delta: 1, shuffle: false, repeat: false }), null);
assert.equal(stepIndex({ index: 2, length: 3, delta: 1, shuffle: false, repeat: true }), 0);
assert.equal(stepIndex({ index: 0, length: 3, delta: -1, shuffle: false, repeat: false }), 0);
assert.equal(stepIndex({ index: 0, length: 0, delta: 1, shuffle: false, repeat: true }), null);

// Shuffle never hands back the track that is already playing.
for (const r of [0, 0.49, 0.99]) {
  const next = stepIndex({ index: 1, length: 4, delta: 1, shuffle: true, repeat: false, random: () => r });
  assert.notEqual(next, 1, `shuffle repeated the current track at random()=${r}`);
  assert.ok(next >= 0 && next < 4);
}
assert.equal(stepIndex({ index: 0, length: 1, delta: 1, shuffle: true, repeat: false, random: () => 0.5 }), 0);

// Folder navigation: only folders with music, counted, sorted.
const library = [
  track({ name: "a.mp3", folder: "", plays: 2 }),
  track({ name: "b.mp3", folder: "Pink Floyd/1973 - Dark Side", plays: 9 }),
  track({ name: "c.mp3", folder: "Pink Floyd/1973 - Dark Side" }),
  track({ name: "d.mp3", folder: "Bowie", plays: 5 }),
];
assert.deepEqual(folderCounts(library), [
  ["", 1],
  ["Bowie", 1],
  ["Pink Floyd/1973 - Dark Side", 2],
]);

// One folder at a time, and "most played" ordering.
assert.deepEqual(orderTracks(library, "Bowie", "folder").map((t) => t.name), ["d.mp3"]);
assert.deepEqual(orderTracks(library, null, "folder").map((t) => t.name), ["a.mp3", "b.mp3", "c.mp3", "d.mp3"]);
assert.deepEqual(orderTracks(library, null, "most-played").map((t) => t.name), ["b.mp3", "d.mp3", "a.mp3", "c.mp3"]);
// Never-played tracks are not in the "most played" queue at all.
assert.deepEqual(mostPlayed(library).map((t) => t.name), ["b.mp3", "d.mp3", "a.mp3"]);

// What a row says. A phone full of downloads is mostly untagged files.
assert.equal(displayTitle(track({ name: "01 - Money.mp3" })), "01 - Money");
assert.equal(displayTitle(track({ name: "x.mp3", title: "Money" })), "Money");
assert.equal(displaySubtitle(track({ artist: "Pink Floyd", album: "Dark Side", year: 1973 })), "Pink Floyd · Dark Side · 1973");
assert.equal(displaySubtitle(track({})), "");
assert.equal(formatTime(0), "0:00");
assert.equal(formatTime(65), "1:05");
assert.equal(formatTime(NaN), "0:00");

console.log("music queue rules: all assertions passed");
