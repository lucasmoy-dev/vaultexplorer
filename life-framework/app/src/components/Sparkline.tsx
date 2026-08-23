// Tiny inline sparkline of 0..100 values.
interface Props {
  values: number[];
  color?: string;
  width?: number;
  height?: number;
}

export function Sparkline({ values, color = "#8b8b9a", width = 64, height = 20 }: Props) {
  if (values.length < 2) return <span className="spark-empty">—</span>;
  const step = width / (values.length - 1);
  const pts = values
    .map((v, i) => `${(i * step).toFixed(1)},${(height - (Math.max(0, Math.min(100, v)) / 100) * height).toFixed(1)}`)
    .join(" ");
  return (
    <svg className="spark" width={width} height={height} viewBox={`0 0 ${width} ${height}`}>
      <polyline points={pts} fill="none" stroke={color} strokeWidth={1.5} />
    </svg>
  );
}
