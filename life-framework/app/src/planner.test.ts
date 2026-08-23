import { describe, expect, it } from "vitest";
import {
  addDays,
  buildPlan,
  daysBetween,
  etaWeeksFromPace,
  observedSlopePerDay,
  planForQuestion,
} from "./planner";
import type { Area, DB, Question } from "./types";

const numericQ = (p: Partial<Question>): Question => ({
  id: p.id ?? "q",
  text: "",
  weight: 1,
  kind: "numeric",
  anchorZero: 0,
  anchorHundred: 100,
  ...p,
});

const mkDb = (q: Question, current: number, snapshots: DB["snapshots"]): DB => {
  const area: Area = {
    id: "a",
    name: "A",
    color: "#000",
    icon: "x",
    weight: 1,
    order: 0,
    subs: [{ id: "s", name: "g", weight: 1, questions: [q] }],
  };
  return {
    schemaVersion: 1,
    settings: { updateRepo: "", tagPrefix: "", reminderDays: 7, theme: "dark" },
    areas: [area],
    current: { [q.id]: current },
    snapshots,
  };
};

describe("date helpers", () => {
  it("daysBetween + addDays round-trip", () => {
    expect(daysBetween("2025-01-01", "2025-01-11")).toBeCloseTo(10, 5);
    expect(addDays("2025-01-01", 10)).toBe("2025-01-11");
  });
});

describe("observedSlopePerDay", () => {
  it("recovers a known linear slope", () => {
    const q = numericQ({ id: "w" });
    const db = mkDb(q, 80, [
      { id: "s1", dateISO: "2025-01-01", answers: { w: 100 } },
      { id: "s2", dateISO: "2025-01-11", answers: { w: 90 } }, // -1/day
    ]);
    expect(observedSlopePerDay(db, "w")).toBeCloseTo(-1, 5);
  });

  it("is null without enough history", () => {
    const q = numericQ({ id: "w" });
    const db = mkDb(q, 80, [{ id: "s1", dateISO: "2025-01-01", answers: { w: 100 } }]);
    expect(observedSlopePerDay(db, "w")).toBeNull();
  });
});

describe("planForQuestion", () => {
  it("computes required weekly pace for a deadline", () => {
    // lose weight: 90 -> 80 over 10 weeks => 1/week
    const q = numericQ({ id: "w", anchorZero: 120, anchorHundred: 70, target: 80, deadline: "2025-03-12" });
    const db = mkDb(q, 90, []);
    const item = planForQuestion(db, db.areas[0], q, "2025-01-01")!;
    expect(item.remaining).toBe(-10);
    // 70 days -> 10 weeks -> -1/week
    expect(item.requiredPerWeek).toBeCloseTo(-1, 2);
    expect(item.status).toBe("no-history");
  });

  it("projects on-track vs behind from observed slope", () => {
    const q = numericQ({ id: "w", anchorZero: 120, anchorHundred: 70, target: 80, deadline: "2025-04-01" });
    // trend -1/day from history; needs to drop 10 -> ~10 days -> well before deadline
    const db = mkDb(q, 90, [
      { id: "s1", dateISO: "2025-01-01", answers: { w: 100 } },
      { id: "s2", dateISO: "2025-01-11", answers: { w: 90 } },
    ]);
    const item = planForQuestion(db, db.areas[0], q, "2025-01-11")!;
    expect(item.status).toBe("on-track");
    expect(item.projectedDate).toBe(addDays("2025-01-11", 10));
  });

  it("flags a stalled goal when the trend moves the wrong way", () => {
    const q = numericQ({ id: "w", anchorZero: 120, anchorHundred: 70, target: 80 });
    const db = mkDb(q, 90, [
      { id: "s1", dateISO: "2025-01-01", answers: { w: 88 } },
      { id: "s2", dateISO: "2025-01-11", answers: { w: 92 } }, // going up, target is down
    ]);
    const item = planForQuestion(db, db.areas[0], q, "2025-01-11")!;
    expect(item.status).toBe("stalled");
  });
});

describe("etaWeeksFromPace", () => {
  it("returns weeks to target at a valid pace", () => {
    expect(etaWeeksFromPace(90, 80, -1)).toBeCloseTo(10, 5);
  });
  it("is null when pace pushes away from the target", () => {
    expect(etaWeeksFromPace(90, 80, 1)).toBeNull();
  });
});

describe("buildPlan", () => {
  it("returns riskiest first and skips targetless questions", () => {
    const withTarget = numericQ({ id: "t", target: 100 });
    const noTarget = numericQ({ id: "n" });
    const area: Area = {
      id: "a",
      name: "A",
      color: "#000",
      icon: "x",
      weight: 1,
      order: 0,
      subs: [{ id: "s", name: "g", weight: 1, questions: [withTarget, noTarget] }],
    };
    const db: DB = {
      schemaVersion: 1,
      settings: { updateRepo: "", tagPrefix: "", reminderDays: 7, theme: "dark" },
      areas: [area],
      current: { t: 20, n: 5 },
      snapshots: [],
    };
    const plan = buildPlan(db);
    expect(plan.length).toBe(1);
    expect(plan[0].question.id).toBe("t");
  });
});
