// ---------------------------------------------------------------------------
// Pure scoring math. No React, no I/O -- unit-tested in scoring.test.ts.
// ---------------------------------------------------------------------------
import type { Area, DB, Question, Subcategory } from "./types";

/** Every question in an area, flattened across its subcategories. */
export const areaQuestions = (area: Area): Question[] =>
  area.subs.flatMap((s) => s.questions);

export const clamp = (x: number, lo: number, hi: number) =>
  Math.max(lo, Math.min(hi, x));

const clamp01 = (x: number) => clamp(x, 0, 1);

/** Default raw value used when a question has never been answered. */
export function defaultRaw(q: Question): number {
  switch (q.kind) {
    case "numeric":
      return q.anchorZero ?? 0;
    case "scale":
      return q.scaleMin ?? 1;
    case "boolean":
      return 0;
  }
}

/**
 * Map a raw value to a 0..100 score for a question.
 * - numeric: linear between anchorZero (0%) and anchorHundred (100%),
 *   clamped. Works when anchorHundred < anchorZero (lower-is-better).
 * - scale: linear between scaleMin and scaleMax.
 * - boolean: yesScore when truthy (>= 0.5), else noScore.
 */
export function normalize(q: Question, raw: number): number {
  if (raw == null || Number.isNaN(raw)) return 0;
  switch (q.kind) {
    case "numeric": {
      const zero = q.anchorZero ?? 0;
      const hundred = q.anchorHundred ?? 100;
      if (hundred === zero) return raw >= hundred ? 100 : 0;
      return clamp01((raw - zero) / (hundred - zero)) * 100;
    }
    case "scale": {
      const min = q.scaleMin ?? 1;
      const max = q.scaleMax ?? 10;
      if (max === min) return 100;
      return clamp01((raw - min) / (max - min)) * 100;
    }
    case "boolean": {
      const yes = q.yesScore ?? 100;
      const no = q.noScore ?? 0;
      return raw >= 0.5 ? yes : no;
    }
  }
}

const posWeight = (w: number) => (w > 0 ? w : 0);

/** Weighted 0..100 score of one subcategory. */
export function subScore(
  sub: Subcategory,
  valueOf: (q: Question) => number,
): number {
  const tw = sub.questions.reduce((s, q) => s + posWeight(q.weight), 0);
  if (tw === 0) return 0;
  const sum = sub.questions.reduce(
    (s, q) => s + normalize(q, valueOf(q)) * posWeight(q.weight),
    0,
  );
  return sum / tw;
}

/** Weighted 0..100 score of one area (weighted mean of its subcategories,
 *  each by its own weight — e.g. "Enfermedades" at 70%). Empty subcategories
 *  are ignored so they neither drag nor inflate the average. */
export function areaScore(
  area: Area,
  valueOf: (q: Question) => number,
): number {
  const parts = area.subs
    .filter((sub) => sub.questions.length > 0)
    .map((sub) => ({ w: posWeight(sub.weight), s: subScore(sub, valueOf) }))
    .filter((p) => p.w > 0);
  const tw = parts.reduce((s, p) => s + p.w, 0);
  if (tw === 0) return 0;
  return parts.reduce((s, p) => s + p.s * p.w, 0) / tw;
}

/** Value lookup for the live "today" polygon. */
export const todayValue = (db: DB) => (q: Question) =>
  db.current[q.id] ?? defaultRaw(q);

/** Value lookup for the "next goal" polygon (falls back to today). */
export const goalValue = (db: DB) => (q: Question) =>
  q.target ?? db.current[q.id] ?? defaultRaw(q);

/** Per-area scores for a given value lookup, in area order. */
export function areaScores(
  db: DB,
  valueOf: (q: Question) => number,
): { area: Area; score: number }[] {
  return [...db.areas]
    .sort((a, b) => a.order - b.order)
    .map((area) => ({ area, score: areaScore(area, valueOf) }));
}

/** Weighted overall score across all areas. */
export function overallScore(
  db: DB,
  valueOf: (q: Question) => number,
): number {
  const scored = db.areas.map((area) => ({
    w: area.weight > 0 ? area.weight : 0,
    s: areaScore(area, valueOf),
  }));
  const tw = scored.reduce((s, x) => s + x.w, 0);
  if (tw === 0) return 0;
  return scored.reduce((s, x) => s + x.s * x.w, 0) / tw;
}

/**
 * Balance: 100 when every area is equal, dropping as the spread widens.
 * Uses population std-dev of area scores, mapped so 50-point spread ~ 0.
 */
export function balanceScore(scores: number[]): number {
  if (scores.length < 2) return 100;
  const mean = scores.reduce((s, x) => s + x, 0) / scores.length;
  const variance =
    scores.reduce((s, x) => s + (x - mean) ** 2, 0) / scores.length;
  const std = Math.sqrt(variance);
  // std of 0 -> 100; std of 50 (max meaningful spread) -> 0.
  return clamp(100 - std * 2, 0, 100);
}

/** Overall score of a historical snapshot. */
export function snapshotOverall(
  db: DB,
  answers: Record<string, number>,
): number {
  return overallScore(db, (q) => answers[q.id] ?? defaultRaw(q));
}
