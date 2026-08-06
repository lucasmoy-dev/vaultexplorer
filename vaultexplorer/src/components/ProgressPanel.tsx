import { useState } from "react";
import { ProgressOp } from "../types";

// VS Code-style "Actions" indicator that lives in the footer/status bar: a
// small spinner + count chip, click to expand a popover listing each running
// task with its progress bar, percentage, and a cancel X.
export function ProgressPanel({
  ops,
  onCancel,
  mobile,
}: {
  ops: ProgressOp[];
  onCancel: (op: ProgressOp) => void;
  // Mobile has no footer to sit in -- the status bar this normally lives
  // in isn't part of that layout, so a running task was invisible there.
  // Floats above the content instead, clear of the bottom safe area.
  mobile?: boolean;
}) {
  const [open, setOpen] = useState(false);
  if (ops.length === 0) return null;

  const chipLabel = ops.length === 1 ? ops[0].label : `${ops.length} tasks`;

  return (
    <div className={`actions-indicator${mobile ? " floating" : ""}`}>
      {open && (
        <div className="actions-popover" onMouseDown={(e) => e.stopPropagation()}>
          <div className="actions-popover-head">
            {ops.length} {ops.length === 1 ? "task running" : "tasks running"}
          </div>
          {ops.map((op) => {
            const pct = op.total > 0 ? Math.min(100, Math.round((op.done / op.total) * 100)) : 0;
            const indeterminate = op.total <= 1 && op.done === 0;
            return (
              <div className={`actions-row ${op.status === "cancelled" ? "cancelling" : ""}`} key={op.id}>
                <div className="actions-row-top">
                  <span className="actions-row-label" title={op.label}>
                    {op.status === "cancelled" ? "Cancelling…" : op.label}
                  </span>
                  {!indeterminate && <span className="actions-pct">{pct}%</span>}
                  {op.cancelId != null && op.status !== "cancelled" && (
                    <button className="actions-cancel" aria-label="Cancel" title="Cancel" onClick={() => onCancel(op)}>
                      ✕
                    </button>
                  )}
                </div>
                <div className={`actions-bar ${indeterminate ? "indet" : ""}`}>
                  <div className="actions-fill" style={indeterminate ? undefined : { width: `${pct}%` }} />
                </div>
              </div>
            );
          })}
        </div>
      )}
      <button className="actions-chip" onClick={() => setOpen((o) => !o)} title="Show running tasks">
        <span className="actions-spinner" />
        <span className="actions-chip-label">{chipLabel}</span>
      </button>
    </div>
  );
}
