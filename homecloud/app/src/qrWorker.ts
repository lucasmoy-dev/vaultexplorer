/// <reference lib="webworker" />
import jsQR from "jsqr";

/**
 * Where the camera frames are actually read.
 *
 * jsQR walking a 480x480 frame takes tens of milliseconds, and it used to do
 * that on the same thread that paints the video — so the preview ran at a
 * handful of frames a second, which is what made aiming at a QR feel like
 * aiming through treacle, and aiming is most of the job. Off here, the
 * preview stays smooth however long a frame takes to decode.
 */

interface Frame {
  data: ArrayBuffer;
  width: number;
  height: number;
  /** Alternated by the caller: trying both polarities every frame halves the rate. */
  inverted: boolean;
}

self.onmessage = (event: MessageEvent<Frame>) => {
  const { data, width, height, inverted } = event.data;
  const found = jsQR(new Uint8ClampedArray(data), width, height, {
    inversionAttempts: inverted ? "onlyInvert" : "dontInvert",
  });
  (self as unknown as Worker).postMessage(found?.data ? found.data.trim() : null);
};
