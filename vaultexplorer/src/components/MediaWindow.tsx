import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { GalleryEntry, MediaViewer } from "./media/MediaViewer";
import { TrafficLights } from "../TitleBar";
import "./MediaWindow.css";

// MediaWindow -- a photo/video/audio file opened as its own window rather
// than as an overlay stacked on the file grid (App.tsx's `?media=1`
// routing, same convention as PlayerWindow's `?player=1`). The grid stays
// usable behind it, several files can be open at once, and each one gets
// real window chrome: macOS traffic lights, with green mapped to
// fullscreen the way every mac media app does.
//
// The viewer *inside* is the same MediaViewer the in-app overlay uses --
// this window is chrome plus a gallery cursor, not a second player.
export interface MediaWindowProps {
  gallery: GalleryEntry[];
  startIndex: number;
}

const CHROME_HIDE_MS = 2600;

export function MediaWindow({ gallery, startIndex }: MediaWindowProps): React.JSX.Element {
  const [chromeVisible, setChromeVisible] = useState(true);
  const [index, setIndex] = useState(Math.min(Math.max(startIndex, 0), Math.max(gallery.length - 1, 0)));
  const hideTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const current = gallery[index] ?? null;

  const wake = useCallback(() => {
    setChromeVisible(true);
    if (hideTimer.current) clearTimeout(hideTimer.current);
    hideTimer.current = setTimeout(() => setChromeVisible(false), CHROME_HIDE_MS);
  }, []);
  useEffect(() => {
    wake();
    return () => {
      if (hideTimer.current) clearTimeout(hideTimer.current);
    };
  }, [wake]);

  const close = useCallback(() => {
    getCurrentWebviewWindow().close().catch(() => {});
  }, []);

  const toggleFullscreen = useCallback(async () => {
    const win = getCurrentWebviewWindow();
    try {
      await win.setFullscreen(!(await win.isFullscreen()));
    } catch {
      // A window manager that refuses fullscreen shouldn't break playback.
    }
  }, []);

  // The window title is the file, so the OS window switcher shows which
  // photo/track this window is -- the whole point of it being a window.
  useEffect(() => {
    if (current) getCurrentWebviewWindow().setTitle(current.name).catch(() => {});
  }, [current?.name]);

  return (
    <div className={`media-window${chromeVisible ? "" : " chrome-hidden"}`} onMouseMove={wake} onTouchStart={wake}>
      <div className="media-window-titlebar" data-tauri-drag-region>
        <TrafficLights onMaximize={toggleFullscreen} />
        <div className="media-window-title" data-tauri-drag-region>
          {current?.name ?? ""}
        </div>
        <div className="media-window-spacer" />
      </div>
      <MediaViewer
        gallery={gallery}
        startIndex={startIndex}
        onClose={close}
        onDeleted={(deleted) => {
          // Deleting the last item leaves nothing to look at; anything
          // else is handled inside the viewer's own gallery.
          if (gallery.length <= 1 && gallery[0]?.fullPath === deleted) close();
        }}
        onFileChanged={() => {}}
        onIndexChange={setIndex}
        filmstrip
        chromeless
        mobile={false}
      />
    </div>
  );
}
