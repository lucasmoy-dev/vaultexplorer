import { describe, expect, it } from "vitest";
import {
  areaScore,
  balanceScore,
  normalize,
  overallScore,
} from "./scoring";
import type { Area, DB, Question } from "./types";

const q = (p: Partial<Question>): Question => ({
  id: p.id ?? "q",
  text: "",
  weight: p.weight ?? 1,
  kind: p.kind ?? "numeric",
  ...p,
});

describe("normalize", () => {
  it("interpolates numeric anchors linearly + clamps", () => {
    const passive = q({ kind: "numeric", anchorZero: 0, anchorHundred: 2500 });
    expect(normalize(passive, 0)).toBe(0);
    expect(normalize(passive, 2500)).toBe(100);
    expect(normalize(passive, 50)).toBeCloseTo(2, 5);
    expect(normalize(passive, 5000)).toBe(100); // clamp high
    expect(normalize(passive, -100)).toBe(0); // clamp low
  });

  it("supports lower-is-better (inverse) numeric", () => {
    const weight = q({ kind: "numeric", anchorZero: 100, anchorHundred: 50 });
    expect(normalize(weight, 100)).toBe(0);
    expect(normalize(weight, 50)).toBe(100);
    expect(normalize(weight, 75)).toBeCloseTo(50, 5);
    expect(normalize(weight, 40)).toBe(100); // past goal, clamp
  });

  it("maps a 1..10 scale", () => {
    const s = q({ kind: "scale", scaleMin: 1, scaleMax: 10 });
    expect(normalize(s, 1)).toBe(0);
    expect(normalize(s, 10)).toBe(100);
    expect(normalize(s, 5)).toBeCloseTo((4 / 9) * 100, 5);
  });

  it("maps boolean with defaults", () => {
    const b = q({ kind: "boolean" });
    expect(normalize(b, 1)).toBe(100);
    expect(normalize(b, 0)).toBe(0);
  });
});

describe("areaScore", () => {
  it("weights questions within a subcategory", () => {
    const area: Area = {
      id: "a",
      name: "Health",
      color: "#000",
      icon: "x",
      weight: 1,
      order: 0,
      subs: [
        {
          id: "s",
          name: "General",
          weight: 1,
          questions: [
            q({ id: "hi", kind: "scale", scaleMin: 0, scaleMax: 100, weight: 3 }), // 100
            q({ id: "lo", kind: "scale", scaleMin: 0, scaleMax: 100, weight: 1 }), // 0
          ],
        },
      ],
    };
    const val = (qq: Question) => (qq.id === "hi" ? 100 : 0);
    // (100*3 + 0*1) / 4 = 75
    expect(areaScore(area, val)).toBeCloseTo(75, 5);
  });

  it("weights subcategories within an area (e.g. Enfermedades at 0.7)", () => {
    const mkSub = (id: string, weight: number): Area["subs"][number] => ({
      id,
      name: id,
      weight,
      questions: [q({ id: id + "-q", kind: "scale", scaleMin: 0, scaleMax: 100 })],
    });
    const area: Area = {
      id: "a", name: "Health", color: "#000", icon: "x", weight: 1, order: 0,
      subs: [mkSub("hi", 1), mkSub("lo", 0.5)],
    };
    const val = (qq: Question) => (qq.id === "hi-q" ? 100 : 0);
    // (100*1 + 0*0.5) / 1.5 = 66.67
    expect(areaScore(area, val)).toBeCloseTo(200 / 3, 4);
  });
});

describe("overallScore + balance", () => {
  const mkArea = (id: string, weight: number): Area => ({
    id,
    name: id,
    color: "#000",
    icon: "x",
    weight,
    order: 0,
    // one subcategory with one 0..100 scale question (raw value == score)
    subs: [{ id: id + "-s", name: "g", weight: 1, questions: [q({ id: id + "-q", kind: "scale", scaleMin: 0, scaleMax: 100 })] }],
  });

  it("weights areas toward the overall", () => {
    const db: DB = {
      schemaVersion: 1,
      settings: { updateRepo: "", tagPrefix: "", reminderDays: 7, theme: "dark" },
      areas: [mkArea("A", 3), mkArea("B", 1)],
      current: { "A-q": 100, "B-q": 0 },
      snapshots: [],
    };
    // (100*3 + 0*1)/4 = 75
    expect(overallScore(db, (qq) => db.current[qq.id] ?? 0)).toBeCloseTo(75, 5);
  });

  it("balance is 100 when areas are equal, lower when spread", () => {
    expect(balanceScore([50, 50, 50])).toBe(100);
    expect(balanceScore([0, 100])).toBeLessThan(20);
  });
});
