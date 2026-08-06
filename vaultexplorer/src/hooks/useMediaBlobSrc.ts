import { useEffect, useState } from "react";
import { isMobilePlatformCached } from "../api";

// Android's WebView custom-protocol bridge (wry, tracked upstream at
// tauri-apps/wry#1551) can stall for ~10-30s -- sometimes never recovering
// -- on the byte-range requests a native <audio>/<video> element issues
// against the asset:// protocol while probing duration/seekability. A
// single plain (non-range) fetch of the same URL doesn't hit that path and
// resolves immediately (same fix already applied to the image editor/
// rotate code for the unrelated canvas-tainting bug). Pulling the whole
// file into memory once and playing it back from a blob: URL sidesteps the
// element's own range requests entirely. Desktop's custom-protocol
// implementation doesn't have this bug and streams large video fine, so
// this workaround is scoped to mobile only.
export function useMediaBlobSrc(src: string): string {
  const [resolved, setResolved] = useState(src);

  useEffect(() => {
    let cancelled = false;
    let objectUrl: string | null = null;
    setResolved(src);
    (async () => {
      if (!(await isMobilePlatformCached())) return;
      try {
        const res = await fetch(src);
        if (!res.ok) return;
        const blob = await res.blob();
        if (cancelled) return;
        objectUrl = URL.createObjectURL(blob);
        setResolved(objectUrl);
      } catch {
        // Fall back to the raw asset:// src -- still better than nothing.
      }
    })();

    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [src]);

  return resolved;
}
