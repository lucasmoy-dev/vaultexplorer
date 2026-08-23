import type { Question } from "./types";

export function scoreColor(score: number): string {
  // red -> amber -> green across 0..100
  if (score >= 75) return "#22c55e";
  if (score >= 50) return "#84cc16";
  if (score >= 30) return "#f59e0b";
  return "#ef4444";
}

export function fmtNum(n: number): string {
  if (!isFinite(n)) return "—";
  const rounded = Math.round(n * 100) / 100;
  return Number.isInteger(rounded) ? String(rounded) : rounded.toFixed(2).replace(/\.?0+$/, "");
}

/** Human label for a raw value given its question kind. */
export function fmtValue(q: Question, raw: number): string {
  if (raw == null || Number.isNaN(raw)) return "—";
  if (q.kind === "boolean") return raw >= 0.5 ? "Sí" : "No";
  if (q.kind === "scale") return `${fmtNum(raw)}/${q.scaleMax ?? 10}`;
  return q.unit ? `${fmtNum(raw)} ${q.unit}` : fmtNum(raw);
}

export function fmtDate(iso: string): string {
  const d = new Date(iso + (iso.length === 10 ? "T00:00:00" : ""));
  return d.toLocaleDateString("es", { day: "2-digit", month: "short", year: "numeric" });
}

export function fmtDaysHuman(days: number): string {
  const d = Math.round(days);
  if (Math.abs(d) >= 60) return `${(d / 30).toFixed(1)} meses`;
  if (Math.abs(d) >= 14) return `${(d / 7).toFixed(1)} semanas`;
  return `${d} días`;
}
