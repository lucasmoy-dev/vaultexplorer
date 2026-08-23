import { useMemo, useState } from "react";
import { useStore } from "../store";
import { QuestionInput } from "../components/QuestionInput";
import { defaultRaw, overallScore, snapshotOverall, todayValue } from "../scoring";
import type { Area, Question, Subcategory } from "../types";
import { scoreColor } from "../format";

export function CheckIn({ onDone }: { onDone: () => void }) {
  const { db, dispatch } = useStore();
  const flat = useMemo(
    () =>
      [...db.areas]
        .sort((a, b) => a.order - b.order)
        .flatMap((area) => area.subs.flatMap((sub) => sub.questions.map((q) => ({ area, sub, q })))),
    [db.areas],
  );

  const [step, setStep] = useState(0);
  const [note, setNote] = useState("");
  const [saved, setSaved] = useState(false);

  if (flat.length === 0) {
    return (
      <div className="screen checkin">
        <p className="muted">No hay preguntas todavía. Agregá áreas y preguntas desde Hoy.</p>
      </div>
    );
  }

  const lastSnap = db.snapshots[db.snapshots.length - 1];
  const before = lastSnap ? snapshotOverall(db, lastSnap.answers) : null;
  const after = overallScore(db, todayValue(db));

  if (saved || step >= flat.length) {
    return (
      <div className="screen checkin done">
        <div className="checkin-summary">
          <h1>{saved ? "Check-in guardado ✅" : "Resumen"}</h1>
          <div className="big-score" style={{ color: scoreColor(after) }}>
            {Math.round(after)}
            <span>/100</span>
          </div>
          {before != null && (
            <p className={after - before >= 0 ? "up" : "down"}>
              {after - before >= 0 ? "▲" : "▼"} {Math.abs(after - before).toFixed(1)} vs. tu último check-in
            </p>
          )}
          {!saved && (
            <>
              <label className="field">
                <span>Nota (opcional)</span>
                <textarea value={note} onChange={(e) => setNote(e.target.value)} placeholder="¿Cómo venís?" />
              </label>
              <div className="row-btns">
                <button onClick={() => setStep(0)}>‹ Revisar</button>
                <button
                  className="primary"
                  onClick={() => {
                    dispatch({ type: "CHECK_IN", note: note.trim() || undefined });
                    setSaved(true);
                  }}
                >
                  Guardar check-in
                </button>
              </div>
            </>
          )}
          {saved && (
            <button className="primary" onClick={onDone}>
              Ver mi radar
            </button>
          )}
        </div>
      </div>
    );
  }

  const { area, sub, q }: { area: Area; sub: Subcategory; q: Question } = flat[step];
  const value = db.current[q.id] ?? defaultRaw(q);
  const progress = ((step + 1) / flat.length) * 100;

  return (
    <div className="screen checkin">
      <div className="progress">
        <div className="progress-fill" style={{ width: `${progress}%`, background: area.color }} />
      </div>
      <div className="checkin-count">
        {step + 1} / {flat.length}
      </div>
      <div className="checkin-area" style={{ color: area.color }}>
        {area.icon} {area.name} · {sub.name}
      </div>
      <h1 className="checkin-q">{q.text}</h1>

      <QuestionInput q={q} value={value} onChange={(v) => dispatch({ type: "SET_CURRENT", questionId: q.id, value: v })} />

      <div className="row-btns">
        <button disabled={step === 0} onClick={() => setStep((s) => Math.max(0, s - 1))}>
          ‹ Atrás
        </button>
        <button className="primary" onClick={() => setStep((s) => s + 1)}>
          {step === flat.length - 1 ? "Terminar" : "Siguiente ›"}
        </button>
      </div>
    </div>
  );
}
