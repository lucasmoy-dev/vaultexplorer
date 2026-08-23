// ---------------------------------------------------------------------------
// Planner math: turn "where I am" + "where I want to be by when" into a
// measurable pace, and project when the current trend actually gets there.
// Pure + deterministic (today is injected), unit-tested in planner.test.ts.
// ---------------------------------------------------------------------------
import type { Area, DB, Question } from "./types";
import { defaultRaw, normalize } from "./scoring";

const MS_PER_DAY = 86_400_000;

export const todayISO = () => new Date().toISOString().slice(0, 10);

export function parseISO(d: string): number {
  // Parse as UTC so date math never shifts across a local-timezone midnight
  // (snapshots are stamped with the UTC date via todayISO()).
  return new Date(d + "T00:00:00Z").getTime();
}

export function daysBetween(fromISO: string, toISO: string): number {
  return (parseISO(toISO) - parseISO(fromISO)) / MS_PER_DAY;
}

export function addDays(fromISO: string, days: number): string {
  return new Date(parseISO(fromISO) + days * MS_PER_DAY)
    .toISOString()
    .slice(0, 10);
}

/**
 * Least-squares slope (raw units per DAY) of a question's answered history.
 * Returns null with fewer than 2 dated data points.
 */
export function observedSlopePerDay(db: DB, questionId: string): number | null {
  const pts: { x: number; y: number }[] = [];
  for (const s of db.snapshots) {
    const v = s.answers[questionId];
    if (v != null && !Number.isNaN(v)) {
      pts.push({ x: parseISO(s.dateISO) / MS_PER_DAY, y: v });
    }
  }
  if (pts.length < 2) return null;
  const n = pts.length;
  const sx = pts.reduce((a, p) => a + p.x, 0);
  const sy = pts.reduce((a, p) => a + p.y, 0);
  const sxx = pts.reduce((a, p) => a + p.x * p.x, 0);
  const sxy = pts.reduce((a, p) => a + p.x * p.y, 0);
  const denom = n * sxx - sx * sx;
  if (denom === 0) return null;
  return (n * sxy - sx * sy) / denom;
}

export type PlanStatus =
  | "no-target" // no target set
  | "reached" // already at/past target
  | "on-track" // trend reaches target on or before deadline
  | "behind" // trend reaches target after deadline
  | "stalled" // trend moves away from / not toward target
  | "no-history"; // target set but not enough history to project

export interface PlanItem {
  area: Area;
  question: Question;
  current: number;
  target: number;
  deadline?: string;
  status: PlanStatus;
  /** Score gain if target is reached (0..100 delta). */
  scoreGain: number;
  /** Raw units still to move. */
  remaining: number;
  /** Required pace to hit the deadline. */
  requiredPerWeek?: number;
  requiredPerMonth?: number;
  /** Observed pace from history. */
  observedPerWeek?: number;
  /** Projected date the current trend reaches target. */
  projectedDate?: string;
  /** Days early (+) or late (-) vs deadline. */
  slackDays?: number;
}

const near = (a: number, b: number, eps = 1e-9) => Math.abs(a - b) <= eps;

/** Build a plan item for one question that has a target. */
export function planForQuestion(
  db: DB,
  area: Area,
  q: Question,
  today = todayISO(),
): PlanItem | null {
  if (q.target == null) return null;
  const current = db.current[q.id] ?? defaultRaw(q);
  const target = q.target;
  const remaining = target - current;
  const scoreGain = normalize(q, target) - normalize(q, current);

  const item: PlanItem = {
    area,
    question: q,
    current,
    target,
    deadline: q.deadline,
    remaining,
    scoreGain,
    status: "no-target",
  };

  if (near(remaining, 0) || Math.sign(scoreGain) < 0 || scoreGain === 0) {
    // At/beyond target (or target would not improve the score).
    if (near(remaining, 0) || scoreGain <= 0) {
      item.status = "reached";
      return item;
    }
  }

  // Required pace to hit the deadline.
  if (q.deadline) {
    const days = daysBetween(today, q.deadline);
    if (days > 0) {
      item.requiredPerWeek = remaining / (days / 7);
      item.requiredPerMonth = remaining / (days / 30);
    }
  }

  // Project from observed trend.
  const slope = observedSlopePerDay(db, q.id); // raw units / day
  if (slope == null) {
    item.status = "no-history";
    return item;
  }
  item.observedPerWeek = slope * 7;

  // Is the trend moving toward the target?
  if (near(slope, 0) || Math.sign(slope) !== Math.sign(remaining)) {
    item.status = "stalled";
    return item;
  }

  const daysToTarget = remaining / slope; // > 0 here
  item.projectedDate = addDays(today, daysToTarget);

  if (q.deadline) {
    const slack = daysBetween(item.projectedDate, q.deadline);
    item.slackDays = slack;
    item.status = slack >= 0 ? "on-track" : "behind";
  } else {
    item.status = "on-track";
  }
  return item;
}

const STATUS_RISK: Record<PlanStatus, number> = {
  behind: 0,
  stalled: 1,
  "no-history": 2,
  "on-track": 3,
  reached: 4,
  "no-target": 5,
};

/** All plan items across the DB, riskiest first. */
export function buildPlan(db: DB, today = todayISO()): PlanItem[] {
  const items: PlanItem[] = [];
  for (const area of db.areas) {
    for (const sub of area.subs) {
      for (const q of sub.questions) {
        const item = planForQuestion(db, area, q, today);
        if (item) items.push(item);
      }
    }
  }
  items.sort((a, b) => {
    const r = STATUS_RISK[a.status] - STATUS_RISK[b.status];
    if (r !== 0) return r;
    return b.scoreGain - a.scoreGain;
  });
  return items;
}

/** What-if: months to move from current to target at a weekly pace. */
export function etaWeeksFromPace(
  current: number,
  target: number,
  perWeek: number,
): number | null {
  const remaining = target - current;
  if (near(remaining, 0)) return 0;
  if (perWeek === 0 || Math.sign(perWeek) !== Math.sign(remaining)) return null;
  return remaining / perWeek;
}
