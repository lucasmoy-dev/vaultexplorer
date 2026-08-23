// Minimal multi-series line chart over dated points, inline SVG.
export interface LineSeries {
  color: string;
  label: string;
  points: { x: number; y: number }[]; // x = time (ms), y = 0..100
}

interface Props {
  series: LineSeries[];
  height?: number;
}

export function LineChart({ series, height = 200 }: Props) {
  const all = series.flatMap((s) => s.points);
  if (all.length < 2) {
    return <div className="chart-empty">Hacé al menos 2 check-ins para ver la tendencia.</div>;
  }
  const W = 320;
  const H = height;
  const padL = 26;
  const padB = 18;
  const padT = 8;
  const padR = 8;

  const xs = all.map((p) => p.x);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const spanX = maxX - minX || 1;

  const sx = (x: number) => padL + ((x - minX) / spanX) * (W - padL - padR);
  const sy = (y: number) => padT + (1 - y / 100) * (H - padT - padB);

  return (
    <svg className="linechart" viewBox={`0 0 ${W} ${H}`} width="100%" role="img" aria-label="Tendencia">
      {[0, 25, 50, 75, 100].map((g) => (
        <g key={g}>
          <line className="lc-grid" x1={padL} y1={sy(g)} x2={W - padR} y2={sy(g)} />
          <text className="lc-ytick" x={padL - 4} y={sy(g) + 3} textAnchor="end">
            {g}
          </text>
        </g>
      ))}
      {series.map((s, i) => (
        <polyline
          key={i}
          className="lc-line"
          points={s.points
            .slice()
            .sort((a, b) => a.x - b.x)
            .map((p) => `${sx(p.x)},${sy(p.y)}`)
            .join(" ")}
          style={{ stroke: s.color }}
        />
      ))}
      {series.map((s) =>
        s.points.map((p, j) => (
          <circle key={s.label + j} className="lc-dot" cx={sx(p.x)} cy={sy(p.y)} r={2.5} style={{ fill: s.color }} />
        )),
      )}
    </svg>
  );
}
