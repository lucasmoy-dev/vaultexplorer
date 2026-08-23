import { useState } from "react";
import { useStore } from "../store";
import { uid } from "../seed";
import type { Question, QuestionKind, Subcategory } from "../types";
import { areaScore, defaultRaw, normalize, subScore, todayValue } from "../scoring";
import { QuestionInput } from "../components/QuestionInput";
import { Sparkline } from "../components/Sparkline";
import { fmtValue, scoreColor } from "../format";

const PALETTE = ["#22c55e", "#f59e0b", "#3b82f6", "#a855f7", "#14b8a6", "#ec4899", "#ef4444", "#eab308"];
const ICONS = ["🫀", "👨‍👩‍👧", "💰", "🚀", "🌱", "🎉", "🧠", "💪", "🙏", "🏠", "❤️", "📚"];

export function AreaDetail({ areaId, onBack }: { areaId: string; onBack: () => void }) {
  const { db, dispatch } = useStore();
  const [editingArea, setEditingArea] = useState(false);
  const [openQ, setOpenQ] = useState<string | null>(null);
  const [openSub, setOpenSub] = useState<string | null>(null);

  const area = db.areas.find((a) => a.id === areaId);
  if (!area) {
    return (
      <div className="screen">
        <p>Esta área ya no existe.</p>
        <button onClick={onBack}>Volver</button>
      </div>
    );
  }

  const score = areaScore(area, todayValue(db));

  const addSub = () => {
    const sub: Subcategory = { id: uid("sub"), name: "Nueva subcategoría", weight: 1, questions: [] };
    dispatch({ type: "ADD_SUB", areaId, sub });
    setOpenSub(sub.id);
  };

  const addQuestion = (subId: string) => {
    const q: Question = { id: uid("q"), text: "Nueva pregunta", weight: 1, kind: "scale", scaleMin: 1, scaleMax: 10 };
    dispatch({ type: "ADD_QUESTION", areaId, subId, question: q });
    setOpenQ(q.id);
  };

  return (
    <div className="screen area-detail">
      <header className="detail-head" style={{ borderColor: area.color }}>
        <button className="back" onClick={onBack}>‹</button>
        <span className="detail-icon">{area.icon}</span>
        <div className="detail-title">
          <h1>{area.name}</h1>
          <span className="detail-score" style={{ color: scoreColor(score) }}>{Math.round(score)}/100</span>
        </div>
        <button className="icon-btn" onClick={() => setEditingArea((e) => !e)}>⚙️</button>
      </header>

      {editingArea && (
        <div className="card area-editor">
          <label className="field">
            <span>Nombre</span>
            <input value={area.name} onChange={(e) => dispatch({ type: "UPDATE_AREA", areaId, patch: { name: e.target.value } })} />
          </label>
          <label className="field">
            <span>Peso en el total: {area.weight}</span>
            <input type="range" min={0.25} max={3} step={0.25} value={area.weight}
              onChange={(e) => dispatch({ type: "UPDATE_AREA", areaId, patch: { weight: Number(e.target.value) } })} />
          </label>
          <div className="field">
            <span>Color</span>
            <div className="swatches">
              {PALETTE.map((c) => (
                <button key={c} className={"swatch" + (area.color === c ? " on" : "")} style={{ background: c }}
                  onClick={() => dispatch({ type: "UPDATE_AREA", areaId, patch: { color: c } })} />
              ))}
            </div>
          </div>
          <div className="field">
            <span>Icono</span>
            <div className="icons">
              {ICONS.map((ic) => (
                <button key={ic} className={"iconpick" + (area.icon === ic ? " on" : "")}
                  onClick={() => dispatch({ type: "UPDATE_AREA", areaId, patch: { icon: ic } })}>{ic}</button>
              ))}
            </div>
          </div>
          <div className="reorder">
            <button onClick={() => dispatch({ type: "REORDER_AREA", areaId, dir: -1 })}>◀ Mover</button>
            <button onClick={() => dispatch({ type: "REORDER_AREA", areaId, dir: 1 })}>Mover ▶</button>
          </div>
          <button className="danger" onClick={() => {
            if (confirm(`¿Eliminar el área "${area.name}" con sus subcategorías?`)) {
              dispatch({ type: "DELETE_AREA", areaId });
              onBack();
            }
          }}>Eliminar área</button>
        </div>
      )}

      {area.subs.map((sub) => {
        const ss = subScore(sub, todayValue(db));
        return (
          <section className="sub-block" key={sub.id}>
            <div className="sub-head">
              <input
                className="sub-name"
                value={sub.name}
                onChange={(e) => dispatch({ type: "UPDATE_SUB", areaId, subId: sub.id, patch: { name: e.target.value } })}
              />
              {sub.questions.length > 0 && (
                <span className="sub-score" style={{ color: scoreColor(ss) }}>{Math.round(ss)}</span>
              )}
              <button className="icon-btn small" onClick={() => setOpenSub(openSub === sub.id ? null : sub.id)}>⚙️</button>
            </div>

            {openSub === sub.id && (
              <div className="card sub-config">
                <label className="field">
                  <span>Peso de la subcategoría en el área: {sub.weight}</span>
                  <input type="range" min={0.1} max={3} step={0.05} value={sub.weight}
                    onChange={(e) => dispatch({ type: "UPDATE_SUB", areaId, subId: sub.id, patch: { weight: Number(e.target.value) } })} />
                </label>
                <button className="danger small" onClick={() => {
                  if (confirm(`¿Eliminar la subcategoría "${sub.name}"?`)) {
                    dispatch({ type: "DELETE_SUB", areaId, subId: sub.id });
                    setOpenSub(null);
                  }
                }}>Eliminar subcategoría</button>
              </div>
            )}

            {sub.questions.map((q) => {
              const value = db.current[q.id] ?? defaultRaw(q);
              const spark = db.snapshots.map((s) => s.answers[q.id]).filter((v) => v != null).map((v) => normalize(q, v));
              return (
                <div key={q.id} className="card question">
                  <div className="q-head">
                    <div className="q-text">{q.text}</div>
                    <div className="q-meta">
                      <Sparkline values={spark} color={area.color} />
                      <span className="q-cur">{fmtValue(q, value)}</span>
                      <button className="icon-btn" onClick={() => setOpenQ(openQ === q.id ? null : q.id)}>⚙️</button>
                    </div>
                  </div>
                  <QuestionInput q={q} value={value} onChange={(v) => dispatch({ type: "SET_CURRENT", questionId: q.id, value: v })} />
                  {openQ === q.id && (
                    <QuestionConfig
                      q={q}
                      onChange={(patch) => dispatch({ type: "UPDATE_QUESTION", areaId, subId: sub.id, questionId: q.id, patch })}
                      onDelete={() => { dispatch({ type: "DELETE_QUESTION", areaId, subId: sub.id, questionId: q.id }); setOpenQ(null); }}
                    />
                  )}
                </div>
              );
            })}
            <button className="add-btn small" onClick={() => addQuestion(sub.id)}>+ Pregunta en {sub.name}</button>
          </section>
        );
      })}

      <button className="add-btn" onClick={addSub}>+ Agregar subcategoría</button>
    </div>
  );
}

function QuestionConfig({
  q,
  onChange,
  onDelete,
}: {
  q: Question;
  onChange: (patch: Partial<Question>) => void;
  onDelete: () => void;
}) {
  return (
    <div className="q-config">
      <label className="field">
        <span>Texto</span>
        <input value={q.text} onChange={(e) => onChange({ text: e.target.value })} />
      </label>

      <label className="field">
        <span>Tipo</span>
        <select value={q.kind} onChange={(e) => onChange({ kind: e.target.value as QuestionKind })}>
          <option value="numeric">Numérico (con anchors)</option>
          <option value="scale">Escala 1–10</option>
          <option value="boolean">Sí / No</option>
        </select>
      </label>

      {q.kind === "numeric" && (
        <>
          <div className="grid2">
            <label className="field">
              <span>Peor (0%)</span>
              <input type="number" value={q.anchorZero ?? 0} onChange={(e) => onChange({ anchorZero: Number(e.target.value) })} />
            </label>
            <label className="field">
              <span>Mejor (100%)</span>
              <input type="number" value={q.anchorHundred ?? 100} onChange={(e) => onChange({ anchorHundred: Number(e.target.value) })} />
            </label>
            <label className="field">
              <span>Unidad</span>
              <input value={q.unit ?? ""} placeholder="€, kg, hrs…" onChange={(e) => onChange({ unit: e.target.value })} />
            </label>
          </div>
          <p className="hint">
            Para métricas invertidas (menos es mejor), poné el valor más alto en <b>Peor</b>. Ej. peso: Peor 90, Mejor 70.
          </p>
        </>
      )}

      {q.kind === "scale" && (
        <div className="grid2">
          <label className="field">
            <span>Mín</span>
            <input type="number" value={q.scaleMin ?? 1} onChange={(e) => onChange({ scaleMin: Number(e.target.value) })} />
          </label>
          <label className="field">
            <span>Máx</span>
            <input type="number" value={q.scaleMax ?? 10} onChange={(e) => onChange({ scaleMax: Number(e.target.value) })} />
          </label>
        </div>
      )}

      <label className="field">
        <span>Peso dentro de la subcategoría: {q.weight}</span>
        <input type="range" min={0.25} max={3} step={0.05} value={q.weight} onChange={(e) => onChange({ weight: Number(e.target.value) })} />
      </label>

      <div className="grid2">
        <label className="field">
          <span>Meta (Next Goal)</span>
          <input type="number" value={q.target ?? ""} placeholder="opcional"
            onChange={(e) => onChange({ target: e.target.value === "" ? undefined : Number(e.target.value) })} />
        </label>
        <label className="field">
          <span>Fecha objetivo</span>
          <input type="date" value={q.deadline ?? ""} onChange={(e) => onChange({ deadline: e.target.value || undefined })} />
        </label>
      </div>

      <button className="danger small" onClick={onDelete}>Eliminar pregunta</button>
    </div>
  );
}
