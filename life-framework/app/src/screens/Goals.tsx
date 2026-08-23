import { useMemo, useState } from "react";
import { useDb } from "../store";
import { buildPlan, etaWeeksFromPace, type PlanItem, type PlanStatus } from "../planner";
import { fmtDate, fmtDaysHuman, fmtNum, fmtValue } from "../format";

const STATUS_LABEL: Record<PlanStatus, { text: string; cls: string }> = {
  "on-track": { text: "En camino", cls: "ok" },
  behind: { text: "Atrasado", cls: "bad" },
  stalled: { text: "Estancado", cls: "warn" },
  "no-history": { text: "Sin datos aún", cls: "muted" },
  reached: { text: "Logrado", cls: "done" },
  "no-target": { text: "Sin meta", cls: "muted" },
};

export function Goals({ onOpenArea }: { onOpenArea: (areaId: string) => void }) {
  const db = useDb();
  const plan = useMemo(() => buildPlan(db), [db]);
  const active = plan.filter((p) => p.status !== "reached");

  return (
    <div className="screen goals">
      <header className="simple-head">
        <h1>🎯 Metas y plan</h1>
        <p className="muted">Dónde querés llegar y el ritmo medible que te lleva ahí.</p>
      </header>

      {plan.length === 0 && (
        <div className="empty-hint">
          Todavía no definiste metas. Abrí una pregunta desde <b>Hoy</b> y poné un valor <b>Meta</b> y una <b>fecha objetivo</b>.
        </div>
      )}

      {active.map((p) => (
        <GoalCard key={p.question.id} item={p} onOpenArea={onOpenArea} />
      ))}

      {plan.some((p) => p.status === "reached") && (
        <section className="reached-list">
          <h2>Logradas</h2>
          {plan
            .filter((p) => p.status === "reached")
            .map((p) => (
              <div key={p.question.id} className="reached-item">
                ✅ {p.area.icon} {p.question.text}
              </div>
            ))}
        </section>
      )}
    </div>
  );
}

function GoalCard({ item, onOpenArea }: { item: PlanItem; onOpenArea: (id: string) => void }) {
  const s = STATUS_LABEL[item.status];
  const [pace, setPace] = useState<number>(
    () => item.requiredPerWeek ?? item.observedPerWeek ?? 1,
  );
  const etaWeeks = etaWeeksFromPace(item.current, item.target, pace);

  return (
    <div className="card goal-card" style={{ borderLeftColor: item.area.color }}>
      <div className="goal-top" onClick={() => onOpenArea(item.area.id)}>
        <span className="goal-area">
          {item.area.icon} {item.area.name}
        </span>
        <span className={"status " + s.cls}>{s.text}</span>
      </div>
      <div className="goal-text">{item.question.text}</div>

      <div className="goal-nums">
        <div>
          <span className="lbl">Ahora</span>
          <span className="val">{fmtValue(item.question, item.current)}</span>
        </div>
        <div className="arrow">→</div>
        <div>
          <span className="lbl">Meta</span>
          <span className="val">{fmtValue(item.question, item.target)}</span>
        </div>
        {item.deadline && (
          <div>
            <span className="lbl">Para</span>
            <span className="val">{fmtDate(item.deadline)}</span>
          </div>
        )}
      </div>

      <div className="goal-facts">
        {item.requiredPerWeek != null && (
          <div className="fact">
            Ritmo necesario: <b>{fmtNum(item.requiredPerWeek)}{item.question.unit ? " " + item.question.unit : ""}/sem</b>
            {" "}({fmtNum(item.requiredPerMonth ?? 0)}/mes)
          </div>
        )}
        {item.observedPerWeek != null && (
          <div className="fact">
            Ritmo actual (según historial): <b>{fmtNum(item.observedPerWeek)}/sem</b>
          </div>
        )}
        {item.projectedDate && (
          <div className="fact">
            A este ritmo llegás <b>{fmtDate(item.projectedDate)}</b>
            {item.slackDays != null && (
              <span className={item.slackDays >= 0 ? "up" : "down"}>
                {" "}({item.slackDays >= 0 ? "+" : "−"}
                {fmtDaysHuman(Math.abs(item.slackDays))} vs. meta)
              </span>
            )}
          </div>
        )}
        {item.status === "no-history" && (
          <div className="fact muted">Hacé algunos check-ins y proyecto tu fecha de llegada.</div>
        )}
      </div>

      <div className="whatif">
        <div className="whatif-head">Simulador de ritmo</div>
        <input
          type="range"
          min={0}
          max={Math.max(2, Math.abs(item.remaining))}
          step={Math.max(0.1, Math.abs(item.remaining) / 50)}
          value={Math.abs(pace)}
          onChange={(e) => setPace(Math.sign(item.remaining || 1) * Number(e.target.value))}
        />
        <div className="whatif-out">
          A <b>{fmtNum(Math.abs(pace))}{item.question.unit ? " " + item.question.unit : ""}/semana</b>:{" "}
          {etaWeeks == null ? (
            <span className="down">no llegás a la meta con ese ritmo</span>
          ) : (
            <b>{fmtDaysHuman(etaWeeks * 7)}</b>
          )}
        </div>
      </div>
    </div>
  );
}
