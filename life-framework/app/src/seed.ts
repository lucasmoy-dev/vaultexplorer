// ---------------------------------------------------------------------------
// First-run "Wheel of Life" template so the radar isn't empty on install.
// Areas → subcategorías → preguntas. Everything is editable afterwards.
// ---------------------------------------------------------------------------
import type { Area, DB, Question, Subcategory } from "./types";
import { SCHEMA_VERSION } from "./types";

let counter = 0;
export const uid = (prefix = "id") =>
  `${prefix}-${Date.now().toString(36)}-${(counter++).toString(36)}`;

const sub = (name: string, weight: number, questions: Omit<Question, "id">[]): Subcategory => ({
  id: uid("sub"),
  name,
  weight,
  questions: questions.map((q) => ({ ...q, id: uid("q") })),
});

function isoInMonths(m: number): string {
  const d = new Date();
  d.setMonth(d.getMonth() + m);
  return d.toISOString().slice(0, 10);
}

function seedAreas(): Area[] {
  const mk = (
    name: string,
    color: string,
    icon: string,
    order: number,
    subs: Subcategory[],
  ): Area => ({ id: uid("area"), name, color, icon, weight: 1, order, subs });

  return [
    mk("Health", "#22c55e", "🫀", 0, [
      sub("Salud mental", 1, [
        { text: "¿Cómo está mi salud mental? (1–10)", weight: 1, kind: "scale", scaleMin: 1, scaleMax: 10, target: 8 },
      ]),
      sub("Ejercicio", 1, [
        { text: "¿Cuántos días entreno por semana?", weight: 1, kind: "numeric", unit: "días", anchorZero: 0, anchorHundred: 5, target: 4 },
      ]),
      sub("Enfermedades", 0.7, [
        { text: "¿Libre de enfermedades / dolencias hoy?", weight: 1, kind: "boolean", yesScore: 100, noScore: 0 },
      ]),
    ]),
    mk("Family", "#f59e0b", "👨‍👩‍👧", 1, [
      sub("Conexión", 1, [
        { text: "¿Qué tan conectado me siento? (1–10)", weight: 1, kind: "scale", scaleMin: 1, scaleMax: 10, target: 9 },
      ]),
      sub("Tiempo", 1, [
        { text: "Tiempo de calidad con la familia (hrs/semana)", weight: 1, kind: "numeric", unit: "hrs", anchorZero: 0, anchorHundred: 15, target: 10 },
      ]),
    ]),
    mk("Finances", "#3b82f6", "💰", 2, [
      sub("Ingresos", 1.5, [
        { text: "Ingresos pasivos mensuales", weight: 1, kind: "numeric", unit: "€", anchorZero: 0, anchorHundred: 2500, target: 1000, deadline: isoInMonths(12) },
      ]),
      sub("Seguridad", 1, [
        { text: "Meses de fondo de emergencia", weight: 1, kind: "numeric", unit: "meses", anchorZero: 0, anchorHundred: 6, target: 6 },
      ]),
    ]),
    mk("Career", "#a855f7", "🚀", 3, [
      sub("Rumbo", 1, [
        { text: "¿Qué tan alineado estoy con mis metas? (1–10)", weight: 1, kind: "scale", scaleMin: 1, scaleMax: 10, target: 8 },
      ]),
      sub("Aprendizaje", 1, [
        { text: "Horas de aprendizaje profundo por semana", weight: 1, kind: "numeric", unit: "hrs", anchorZero: 0, anchorHundred: 10, target: 6 },
      ]),
    ]),
    mk("Growth", "#14b8a6", "🌱", 4, [
      sub("Consumo", 1, [
        { text: "Libros / cursos terminados este mes", weight: 1, kind: "numeric", unit: "u", anchorZero: 0, anchorHundred: 2, target: 1 },
      ]),
      sub("Hábitos", 1, [
        { text: "¿Mantuve mis hábitos clave hoy?", weight: 1, kind: "boolean" },
      ]),
    ]),
    mk("Fun", "#ec4899", "🎉", 5, [
      sub("Ocio", 1, [
        { text: "Satisfacción con mi tiempo libre (1–10)", weight: 1, kind: "scale", scaleMin: 1, scaleMax: 10, target: 8 },
      ]),
    ]),
  ];
}

export function seedDb(): DB {
  const areas = seedAreas();
  // Prime "current" so the first radar already shows some shape.
  const current: Record<string, number> = {};
  for (const a of areas) {
    for (const s of a.subs) {
      for (const q of s.questions) {
        if (q.kind === "scale") current[q.id] = Math.round(((q.scaleMax ?? 10) + (q.scaleMin ?? 1)) / 3);
        else if (q.kind === "boolean") current[q.id] = 0;
        else current[q.id] = (q.anchorHundred ?? 100) * 0.2;
      }
    }
  }
  return {
    schemaVersion: SCHEMA_VERSION,
    settings: {
      updateRepo: "lucasmoy-dev/vaultexplorer",
      tagPrefix: "life-framework-v",
      reminderDays: 7,
      theme: "dark",
    },
    areas,
    current,
    snapshots: [],
  };
}
