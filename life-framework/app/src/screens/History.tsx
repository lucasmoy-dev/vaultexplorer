import { useMemo, useState } from "react";
import { useDb } from "../store";
import { LineChart, type LineSeries } from "../components/LineChart";
import { RadarChart, type RadarSeries } from "../components/RadarChart";
import { areaScore, areaScores, defaultRaw, overallScore, todayValue } from "../scoring";
import { parseISO } from "../planner";
import { fmtDate } from "../format";
import type { Question } from "../types";

export function History() {
  const db = useDb();
  const [mode, setMode] = useState<"overall" | "areas">("overall");

  const sortedAreas = useMemo(() => [...db.areas].sort((a, b) => a.order - b.order), [db.areas]);

  const overallSeries: LineSeries[] = useMemo(() => {
    const pts = db.snapshots.map((s) => ({
      x: parseISO(s.dateISO),
      y: overallScore(db, (q) => s.answers[q.id] ?? defaultRaw(q)),
    }));
    return [{ color: "#6366f1", label: "General", points: pts }];
  }, [db]);

  const areaSeries: LineSeries[] = useMemo(
    () =>
      sortedAreas.map((area) => ({
        color: area.color,
        label: area.name,
        points: db.snapshots.map((s) => ({
          x: parseISO(s.dateISO),
          y: areaScore(area, (q) => s.answers[q.id] ?? defaultRaw(q)),
        })),
      })),
    [db, sortedAreas],
  );

  // Radar compare: today vs a chosen past snapshot.
  const [compareId, setCompareId] = useState<string>(() => db.snapshots[0]?.id ?? "");
  const compareSnap = db.snapshots.find((s) => s.id === compareId);
  const today = areaScores(db, todayValue(db));
  const compareSeries: RadarSeries[] = [
    { scores: today.map((t) => t.score), color: "#6366f1", label: "Hoy", fillOpacity: 0.3 },
  ];
  if (compareSnap) {
    const valueOf = (q: Question) => compareSnap.answers[q.id] ?? defaultRaw(q);
    compareSeries.push({
      scores: areaScores(db, valueOf).map((a) => a.score),
      color: "#94a3b8",
      label: fmtDate(compareSnap.dateISO),
      fillOpacity: 0.12,
      dashed: true,
    });
  }

  return (
    <div className="screen history">
      <header className="simple-head">
        <h1>📈 Historial</h1>
        <p className="muted">
          {db.snapshots.length} check-in{db.snapshots.length === 1 ? "" : "s"} registrados.
        </p>
      </header>

      <div className="seg">
        <button className={mode === "overall" ? "on" : ""} onClick={() => setMode("overall")}>
          General
        </button>
        <button className={mode === "areas" ? "on" : ""} onClick={() => setMode("areas")}>
          Por área
        </button>
      </div>

      <div className="card">
        <LineChart series={mode === "overall" ? overallSeries : areaSeries} />
        {mode === "areas" && (
          <div className="legend wrap">
            {sortedAreas.map((a) => (
              <span key={a.id} className="legend-item">
                <i style={{ background: a.color }} /> {a.name}
              </span>
            ))}
          </div>
        )}
      </div>

      {db.snapshots.length >= 1 && (
        <div className="card">
          <div className="compare-head">
            <span>Comparar hoy con:</span>
            <select value={compareId} onChange={(e) => setCompareId(e.target.value)}>
              {db.snapshots.map((s) => (
                <option key={s.id} value={s.id}>
                  {fmtDate(s.dateISO)}
                </option>
              ))}
            </select>
          </div>
          <RadarChart areas={sortedAreas} series={compareSeries} size={300} />
        </div>
      )}
    </div>
  );
}
