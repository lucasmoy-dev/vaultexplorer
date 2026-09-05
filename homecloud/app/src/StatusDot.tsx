import type { FolderState } from "./api";

/** The whole status vocabulary of the app: one dot, five meanings. */
export function StatusDot({ state }: { state: FolderState }) {
  const tone =
    state.kind === "upToDate"
      ? "ok"
      : state.kind === "syncing"
        ? "busy"
        : state.kind === "problem"
          ? "bad"
          : "idle";
  return <span className={`dot dot-${tone}`} aria-hidden />;
}

export function stateLabel(state: FolderState): string {
  switch (state.kind) {
    case "upToDate":
      return "Al día";
    case "syncing":
      return `Sincronizando ${state.percent}%`;
    case "paused":
      return "En pausa";
    case "disconnected":
      return "Sin conexión";
    case "problem":
      return state.detail;
  }
}
