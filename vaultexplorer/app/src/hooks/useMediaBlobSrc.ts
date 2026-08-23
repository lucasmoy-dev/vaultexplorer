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
      // Only the asset:// protocol has the stall described above. The local
      // media server (see mediaserver.rs) answers byte ranges properly over
      // plain HTTP, and buffering a whole file through it would cost a
      // 10MB round trip per track -- which on a music queue is a pause
      // between every song.
      if (src.startsWith("http://127.0.0.1") || src.startsWith("http://localhost")) return;
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
