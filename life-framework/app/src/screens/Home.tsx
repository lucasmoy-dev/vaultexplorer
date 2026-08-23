import { useMemo, useState } from "react";
import { useDb } from "../store";
import { RadarChart, type RadarSeries } from "../components/RadarChart";
import {
  areaScores,
  balanceScore,
  goalValue,
  overallScore,
  snapshotOverall,
  todayValue,
} from "../scoring";
import { scoreColor } from "../format";

export function Home({ onOpenArea }: { onOpenArea: (areaId: string) => void }) {
  const db = useDb();
  const [showGoal, setShowGoal] = useState(true);

  const sortedAreas = useMemo(
    () => [...db.areas].sort((a, b) => a.order - b.order),
    [db.areas],
  );

  const today = useMemo(() => areaScores(db, todayValue(db)), [db]);
  const goal = useMemo(() => areaScores(db, goalValue(db)), [db]);
  const overall = useMemo(() => overallScore(db, todayValue(db)), [db]);
  const goalOverall = useMemo(() => overallScore(db, goalValue(db)), [db]);
  const balance = useMemo(() => balanceScore(today.map((t) => t.score)), [today]);

  // Delta vs last check-in.
  const lastSnap = db.snapshots[db.snapshots.length - 1];
  const delta = lastSnap ? overall - snapshotOverall(db, lastSnap.answers) : null;

  const series: RadarSeries[] = [
    { scores: today.map((t) => t.score), color: "#6366f1", label: "Hoy", fillOpacity: 0.35 },
  ];
  if (showGoal) {
    series.push({
      scores: goal.map((g) => g.score),
      color: "#f5c542",
      label: "Next Goal",
      fillOpacity: 0.1,
      dashed: true,
    });
  }

  const insights = useMemo(() => buildInsights(today, goal, db.snapshots.length), [today, goal, db.snapshots.length]);

  return (
    <div className="screen home">
      <header className="home-head">
        <div className="overall">
          <div
            className="overall-ring"
            style={{ "--c": scoreColor(overall), "--pct": Math.round(overall) } as React.CSSProperties}
          >
            <span className="overall-num">{Math.round(overall)}</span>
            <span className="overall-unit">/100</span>
          </div>
          <div className="overall-meta">
            <h1>¿Cómo estoy hoy?</h1>
            <div className="chips">
              <span className="chip">⚖️ Balance {Math.round(balance)}</span>
              {delta != null && (
                <span className={"chip " + (delta >= 0 ? "up" : "down")}>
                  {delta >= 0 ? "▲" : "▼"} {Math.abs(delta).toFixed(1)} desde el último check-in
                </span>
              )}
              <span className="chip goal">🎯 Meta {Math.round(goalOverall)}</span>
            </div>
          </div>
        </div>
      </header>

      <RadarChart areas={sortedAreas} series={series} size={340} onSelectAxis={onOpenArea} />

      <div className="legend">
        <span className="legend-item">
          <i style={{ background: "#6366f1" }} /> Hoy
        </span>
        <button className={"legend-item toggle-legend" + (showGoal ? " on" : "")} onClick={() => setShowGoal((s) => !s)}>
          <i className="dashed" style={{ background: "#f5c542" }} /> Next Goal
        </button>
      </div>

      <section className="area-scorelist">
        {today.map((t) => (
          <button key={t.area.id} className="area-scoreitem" onClick={() => onOpenArea(t.area.id)}>
            <span className="asi-icon">{t.area.icon}</span>
            <span className="asi-name">{t.area.name}</span>
            <span className="asi-bar">
              <span style={{ width: `${t.score}%`, background: t.area.color }} />
            </span>
            <span className="asi-num" style={{ color: scoreColor(t.score) }}>
              {Math.round(t.score)}
            </span>
          </button>
        ))}
      </section>

      {insights.length > 0 && (
        <section className="insights">
          {insights.map((ins, i) => (
            <div key={i} className="insight-card">
              <span className="insight-emoji">{ins.emoji}</span>
              <div>
                <div className="insight-title">{ins.title}</div>
                <div className="insight-body">{ins.body}</div>
              </div>
            </div>
          ))}
        </section>
      )}
    </div>
  );
}

type Scored = { area: { id: string; name: string }; score: number };

function buildInsights(today: Scored[], goal: Scored[], snaps: number) {
  const out: { emoji: string; title: string; body: string }[] = [];
  if (today.length === 0) return out;

  const weakest = [...today].sort((a, b) => a.score - b.score)[0];
  out.push({
    emoji: "🔧",
    title: "Área más floja",
    body: `${weakest.area.name} está en ${Math.round(weakest.score)}. Un pequeño empujón acá mueve mucho el promedio.`,
  });

  // Biggest gap to its own goal = highest-leverage next move.
  const gaps = today.map((t) => {
    const g = goal.find((x) => x.area.id === t.area.id);
    return { name: t.area.name, gap: (g?.score ?? t.score) - t.score };
  });
  const topGap = [...gaps].sort((a, b) => b.gap - a.gap)[0];
  if (topGap && topGap.gap > 1) {
    out.push({
      emoji: "🎯",
      title: "Mayor distancia a tu meta",
      body: `${topGap.name}: te faltan ${Math.round(topGap.gap)} puntos para tu Next Goal.`,
    });
  }

  if (snaps === 0) {
    out.push({
      emoji: "📝",
      title: "Todavía sin historial",
      body: "Hacé tu primer check-in para empezar a medir tu progreso en el tiempo.",
    });
  }
  return out;
}
