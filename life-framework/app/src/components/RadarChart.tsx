// Hand-drawn SVG radar (n-gon). Renders the Today polygon and, optionally,
// a transparent "Next Goal" overlay on the same axes.
import type { Area } from "../types";

export interface RadarSeries {
  scores: number[]; // 0..100, one per axis, same order as `areas`
  color: string;
  label: string;
  fillOpacity: number;
  dashed?: boolean;
}

interface Props {
  areas: Area[]; // axis order
  series: RadarSeries[];
  size?: number;
  onSelectAxis?: (areaId: string) => void;
}

const RINGS = [20, 40, 60, 80, 100];

export function RadarChart({ areas, series, size = 320, onSelectAxis }: Props) {
  const n = areas.length;
  const cx = size / 2;
  const cy = size / 2;
  const R = size / 2 - 46; // leave room for labels

  if (n < 3) {
    return (
      <div className="radar-empty">
        Agregá al menos 3 áreas para dibujar el polígono.
      </div>
    );
  }

  // Angle for axis i: start at top (-90deg), clockwise.
  const angle = (i: number) => (-Math.PI / 2) + (i * 2 * Math.PI) / n;
  // Point at radius fraction f (0..1) along axis i.
  const pt = (i: number, f: number): [number, number] => {
    const a = angle(i);
    return [cx + R * f * Math.cos(a), cy + R * f * Math.sin(a)];
  };

  const polygon = (scores: number[]) =>
    scores.map((s, i) => pt(i, clampF(s)).join(",")).join(" ");

  return (
    <svg
      className="radar"
      viewBox={`0 0 ${size} ${size}`}
      width="100%"
      role="img"
      aria-label="Radar de áreas de vida"
    >
      {/* grid rings */}
      {RINGS.map((ring) => (
        <polygon
          key={ring}
          className="radar-ring"
          points={areas.map((_, i) => pt(i, ring / 100).join(",")).join(" ")}
        />
      ))}
      {/* axes */}
      {areas.map((a, i) => {
        const [x, y] = pt(i, 1);
        return <line key={a.id} className="radar-axis" x1={cx} y1={cy} x2={x} y2={y} />;
      })}

      {/* series polygons (goal first, so today draws on top) */}
      {[...series].reverse().map((s, idx) => (
        <polygon
          key={idx}
          className={"radar-series" + (s.dashed ? " dashed" : "")}
          points={polygon(s.scores)}
          style={{
            fill: s.color,
            stroke: s.color,
            fillOpacity: s.fillOpacity,
          }}
        />
      ))}
      {/* today vertices */}
      {series[0] &&
        series[0].scores.map((sc, i) => {
          const [x, y] = pt(i, clampF(sc));
          return <circle key={i} className="radar-dot" cx={x} cy={y} r={3.5} style={{ fill: series[0].color }} />;
        })}

      {/* labels */}
      {areas.map((a, i) => {
        const [x, y] = pt(i, 1.16);
        const anchor = x < cx - 8 ? "end" : x > cx + 8 ? "start" : "middle";
        return (
          <g
            key={a.id}
            className="radar-label"
            onClick={() => onSelectAxis?.(a.id)}
            style={{ cursor: onSelectAxis ? "pointer" : "default" }}
          >
            <text x={x} y={y - 4} textAnchor={anchor} className="radar-label-icon">
              {a.icon}
            </text>
            <text x={x} y={y + 11} textAnchor={anchor} className="radar-label-name" style={{ fill: a.color }}>
              {a.name}
            </text>
          </g>
        );
      })}
    </svg>
  );
}

const clampF = (s: number) => Math.max(0, Math.min(100, s)) / 100;
