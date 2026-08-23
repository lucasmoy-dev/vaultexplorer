// Editor for a single question's raw value, shaped by its kind.
import type { Question } from "../types";
import { normalize } from "../scoring";
import { scoreColor } from "../format";

interface Props {
  q: Question;
  value: number;
  onChange: (value: number) => void;
}

export function QuestionInput({ q, value, onChange }: Props) {
  const score = normalize(q, value);

  if (q.kind === "boolean") {
    const yes = value >= 0.5;
    return (
      <div className="qinput">
        <div className="toggle-row">
          <button className={"toggle" + (!yes ? " on" : "")} onClick={() => onChange(0)}>
            No
          </button>
          <button className={"toggle" + (yes ? " on" : "")} onClick={() => onChange(1)}>
            Sí
          </button>
        </div>
        <ScoreBar score={score} />
      </div>
    );
  }

  if (q.kind === "scale") {
    const min = q.scaleMin ?? 1;
    const max = q.scaleMax ?? 10;
    return (
      <div className="qinput">
        <div className="scale-row">
          <input
            type="range"
            min={min}
            max={max}
            step={1}
            value={value}
            onChange={(e) => onChange(Number(e.target.value))}
          />
          <span className="scale-val">{value}</span>
        </div>
        <ScoreBar score={score} />
      </div>
    );
  }

  // numeric
  return (
    <div className="qinput">
      <div className="num-row">
        <input
          type="number"
          inputMode="decimal"
          value={Number.isNaN(value) ? "" : value}
          onChange={(e) => onChange(e.target.value === "" ? 0 : Number(e.target.value))}
        />
        {q.unit && <span className="unit">{q.unit}</span>}
      </div>
      <div className="anchors">
        Peor {q.anchorZero ?? 0} · Mejor {q.anchorHundred ?? 100}
      </div>
      <ScoreBar score={score} />
    </div>
  );
}

function ScoreBar({ score }: { score: number }) {
  return (
    <div className="scorebar">
      <div className="scorebar-fill" style={{ width: `${score}%`, background: scoreColor(score) }} />
      <span className="scorebar-num">{Math.round(score)}%</span>
    </div>
  );
}
