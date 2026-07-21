import { ReactNode, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "./api";

// macOS traffic-light window controls. In the two-pane layout these live at
// the top of the sidebar (like Finder), extracted here so both the sidebar
// and the legacy full-width TitleBar (boot screen) can use them.

const appWindow = getCurrentWindow();

export function TrafficLights() {
  const [hovering, setHovering] = useState(false);
  return (
    <div
      className="traffic-lights"
      onMouseEnter={() => setHovering(true)}
      onMouseLeave={() => setHovering(false)}
    >
      <button className="tl tl-close" aria-label="Close" onClick={() => appWindow.close()}>
        {hovering && (
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <path d="M3 3l6 6M9 3l-6 6" stroke="#4d0000" strokeWidth="2" strokeLinecap="round" />
          </svg>
        )}
      </button>
      <button className="tl tl-min" aria-label="Minimize" onClick={() => appWindow.minimize()}>
        {hovering && (
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <path d="M3 6h6" stroke="#5c3d00" strokeWidth="2" strokeLinecap="round" />
          </svg>
        )}
      </button>
      <button className="tl tl-max" aria-label="Maximize" onClick={() => appWindow.toggleMaximize()}>
        {hovering && (
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <path d="M4 4v4h4zM8 8V4H4z" fill="#0a3d00" />
          </svg>
        )}
      </button>
    </div>
  );
}

export function TitleBar({ children }: { children?: ReactNode }) {
  const [mobile, setMobile] = useState(false);
  useEffect(() => {
    api.isMobilePlatform().then(setMobile).catch(() => {});
  }, []);

  return (
    <div className="titlebar" data-tauri-drag-region={mobile ? undefined : true}>
      {!mobile && <TrafficLights />}
      <div className="titlebar-content" data-tauri-drag-region={mobile ? undefined : true}>
        {children}
      </div>
    </div>
  );
}
