import { useEffect, useState } from "react";
import { Drive, MachineSummary, api, formatSize } from "../api";
import { DiskGlyph, UsbDriveGlyph } from "../icons";
import "./DeviceView.css";

// "My Device" -- what the machine *is* and how full it is, as opposed to
// "My Computer", which is where its files are. Split reported directly:
// one sidebar entry was doing both jobs and neither well.
//
// Everything here is a capacity question ("how much room is left, and in
// what"), so every number is paired with the bar it belongs to rather than
// listed as a spec sheet. Live values (memory, load) re-read on a timer;
// the static ones (CPU, OS) are read once.

const REFRESH_MS = 4000;

function pct(used: number, total: number): number {
  if (total <= 0) return 0;
  return Math.min(100, Math.max(0, (used / total) * 100));
}

// Green until it's worth noticing, amber when it's getting tight, red when
// it's a problem -- the thresholds people already expect from every disk
// UI, so the colour means the same thing here as it does elsewhere.
function levelOf(percent: number): "ok" | "warn" | "full" {
  if (percent >= 90) return "full";
  if (percent >= 75) return "warn";
  return "ok";
}

function Meter({
  label,
  detail,
  used,
  total,
  hint,
}: {
  label: React.ReactNode;
  detail?: string;
  used: number;
  total: number;
  hint?: string;
}) {
  const percent = pct(used, total);
  return (
    <div className="device-meter">
      <div className="device-meter-head">
        <span className="device-meter-label">{label}</span>
        <span className="device-meter-value">
          {formatSize(used)} <span className="device-meter-of">of {formatSize(total)}</span>
        </span>
      </div>
      <div
        className={`device-bar ${levelOf(percent)}`}
        role="meter"
        aria-valuenow={Math.round(percent)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={typeof label === "string" ? label : undefined}
      >
        <div className="device-bar-fill" style={{ width: `${percent}%` }} />
      </div>
      <div className="device-meter-foot">
        <span>{detail}</span>
        <span>{Math.round(percent)}% used · {formatSize(Math.max(total - used, 0))} free</span>
      </div>
      {hint && <p className="device-hint">{hint}</p>}
    </div>
  );
}

function formatUptime(secs: number): string {
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export function DeviceView({
  onOpenDrive,
  onFreeUpSpace,
}: {
  onOpenDrive: (d: Drive) => void;
  onFreeUpSpace: () => void;
}) {
  const [info, setInfo] = useState<MachineSummary | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    let cancelled = false;
    const load = () =>
      api
        .machineSummary()
        .then((s) => {
          if (!cancelled) setInfo(s);
        })
        .catch((e) => {
          if (!cancelled) setError(String(e));
        });
    load();
    const t = setInterval(load, REFRESH_MS);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, []);

  if (error) return <p className="error device-error">{error}</p>;
  if (!info) return <div className="device-view device-loading">Reading device info…</div>;

  const ramUsed = Math.max(info.ram_total - info.ram_available, 0);
  const swapUsed = Math.max(info.swap_total - info.swap_free, 0);
  // The disks worth showing capacity for: anything actually mounted. An
  // unmounted removable is listed too (it's still a drive you own) but has
  // no usage to report.
  const disks = info.disks.filter((d) => d.mountpoint || d.removable);
  const totalCapacity = disks.reduce((sum, d) => sum + d.total, 0);
  const totalUsed = disks.reduce((sum, d) => sum + d.used, 0);

  return (
    <div className="device-view">
      <div className="device-head">
        <div>
          <h2>{info.os_name}</h2>
          <p className="device-sub">
            {info.cpu_model} · {info.cpu_cores} cores · up {formatUptime(info.uptime_secs)} · load{" "}
            {info.load1.toFixed(2)}
          </p>
        </div>
        <button className="btn-plain" onClick={onFreeUpSpace}>
          Free up space…
        </button>
      </div>

      <div className="device-grid">
        <section className="device-panel">
          <h3>Memory</h3>
          <Meter
            label="RAM"
            detail={`${info.cpu_cores} cores available`}
            used={ramUsed}
            total={info.ram_total}
            hint={
              ramUsed / Math.max(info.ram_total, 1) > 0.9
                ? "Memory is nearly full — closing a few apps will make everything feel faster."
                : undefined
            }
          />
          {info.swap_total > 0 && (
            <Meter
              label="Swap"
              detail="Overflow onto disk"
              used={swapUsed}
              total={info.swap_total}
              hint={
                swapUsed / Math.max(info.swap_total, 1) > 0.5
                  ? "Heavy swap use means the machine is out of RAM and paging to disk."
                  : undefined
              }
            />
          )}
        </section>

        <section className="device-panel">
          <h3>Storage</h3>
          {totalCapacity > 0 && (
            <Meter label="All drives" detail={`${disks.length} drives`} used={totalUsed} total={totalCapacity} />
          )}
          <div className="device-disks">
            {disks.map((d) => (
              <button
                key={d.path}
                className="device-disk"
                onClick={() => d.mountpoint && onOpenDrive(d)}
                disabled={!d.mountpoint}
                title={d.mountpoint ? `Open ${d.mountpoint}` : "Not mounted"}
              >
                <span className="device-disk-ico">
                  {d.removable ? <UsbDriveGlyph size={22} /> : <DiskGlyph size={22} />}
                </span>
                <span className="device-disk-main">
                  <span className="device-disk-name">
                    {d.label || d.model || d.name}
                    {d.fstype && <span className="device-disk-fs">{d.fstype}</span>}
                  </span>
                  {d.total > 0 ? (
                    <>
                      <span className={`device-bar small ${levelOf(pct(d.used, d.total))}`}>
                        <span className="device-bar-fill" style={{ width: `${pct(d.used, d.total)}%` }} />
                      </span>
                      <span className="device-disk-foot">
                        {formatSize(d.free)} free of {formatSize(d.total)}
                      </span>
                    </>
                  ) : (
                    <span className="device-disk-foot">{d.mountpoint ?? "Not mounted"}</span>
                  )}
                </span>
              </button>
            ))}
            {disks.length === 0 && <p className="device-hint">No drives reported.</p>}
          </div>
        </section>
      </div>
    </div>
  );
}
