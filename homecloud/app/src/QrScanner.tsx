import { useCallback, useEffect, useRef, useState } from "react";
import jsQR from "jsqr";
import { api } from "./api";
import { CloseIcon } from "./Icons";

/**
 * Reads a pairing code off another device's screen with this one's camera.
 *
 * Decoding happens here rather than in the browser's own barcode detector,
 * which WebKitGTK — the engine behind this window on Linux — does not
 * implement. It happens in a worker rather than on this thread, because it
 * used to share a thread with the video: every decode froze the preview, the
 * picture crawled, and aiming at a small square on another screen while the
 * image stutters is most of why scanning failed and the code got typed by
 * hand instead.
 */

/**
 * The camera is asked for a specific, generous size and nothing else.
 *
 * Asking for `facingMode: "environment"` was wrong on a laptop, which has no
 * such camera: the engine kept renegotiating, the picture blacked out every
 * couple of seconds, and what came back was too soft for a QR to survive.
 */
const WANTED: MediaTrackConstraints = {
  width: { ideal: 1280 },
  height: { ideal: 720 },
};

/**
 * The QR is decoded from a square crop of the middle of the frame, scaled to
 * this. A full 1280x720 frame is four times the pixels for jsQR to walk on
 * every tick, and the extra area is the room around the code, never the code.
 */
const DECODE_SIZE = 480;

/**
 * How much of the frame's short side that crop covers. The tight one is where
 * a code held up to the camera lands; the wide one catches a code further away
 * or off to one side, which otherwise never decodes no matter how long the
 * user waits. They alternate, so both are tried about ten times a second.
 */
const CROPS = [0.7, 1];

export function QrScanner({
  onScanned,
  onClose,
}: {
  onScanned: (code: string) => void;
  onClose: () => void;
}) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [cameras, setCameras] = useState<MediaDeviceInfo[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);

  const report = useCallback((detail: string) => void api.reportCameraProblem(detail), []);

  useEffect(() => {
    let stream: MediaStream | null = null;
    let timer = 0;
    let stopped = false;
    let attempt = 0;

    // One canvas for the whole session: allocating per frame is what turns a
    // scan into a slideshow.
    const canvas = document.createElement("canvas");
    canvas.width = DECODE_SIZE;
    canvas.height = DECODE_SIZE;
    const context = canvas.getContext("2d", { willReadFrequently: true });

    // A worker keeps the decoding off the thread that paints the preview. If
    // this window cannot make one, the scanner still works — just on one
    // thread, as it always used to.
    let worker: Worker | null = null;
    try {
      worker = new Worker(new URL("./qrWorker.ts", import.meta.url));
    } catch (error) {
      report(`worker unavailable: ${describe(error)}`);
    }

    function found(code: string) {
      stopped = true;
      onScanned(code);
    }

    /** Copies the middle of the current frame into the canvas. */
    function grab(): ImageData | null {
      const video = videoRef.current;
      if (!video || !context || video.readyState < video.HAVE_CURRENT_DATA) return null;
      const shortest = Math.min(video.videoWidth, video.videoHeight);
      if (shortest <= 0) return null;
      const side = shortest * CROPS[attempt % CROPS.length];
      attempt += 1;
      context.drawImage(
        video,
        (video.videoWidth - side) / 2,
        (video.videoHeight - side) / 2,
        side,
        side,
        0,
        0,
        DECODE_SIZE,
        DECODE_SIZE,
      );
      return context.getImageData(0, 0, DECODE_SIZE, DECODE_SIZE);
    }

    function next(delay = 40) {
      if (stopped) return;
      timer = window.setTimeout(read, delay);
    }

    function read() {
      if (stopped) return;
      const image = grab();
      if (!image) {
        next(80);
        return;
      }
      if (worker) {
        // One frame in flight at a time: the reply is what asks for the next,
        // so a slow decode drops frames instead of queueing them up.
        worker.postMessage(
          {
            data: image.data.buffer,
            width: image.width,
            height: image.height,
            // A QR shown on a dark-mode screen comes back inverted, and
            // refusing to try costs a scan that would have worked.
            inverted: attempt % 2 === 0,
          },
          [image.data.buffer],
        );
        return;
      }
      const decoded = jsQR(image.data, image.width, image.height, {
        inversionAttempts: "attemptBoth",
      });
      if (decoded?.data) {
        found(decoded.data.trim());
        return;
      }
      next(60);
    }

    if (worker) {
      worker.onmessage = (event: MessageEvent<string | null>) => {
        if (stopped) return;
        if (event.data) found(event.data);
        else next();
      };
      worker.onerror = (event) => {
        report(`worker failed: ${event.message}`);
        worker?.terminate();
        worker = null;
        next();
      };
    }

    if (!navigator.mediaDevices?.getUserMedia) {
      report(
        `mediaDevices missing (secureContext=${window.isSecureContext}, origin=${window.location.origin})`,
      );
      setProblem(
        "Esta ventana no puede abrir la cámara: el sistema no se la ofrece a la aplicación. " +
          "Cancela y pega el código.",
      );
      return () => {
        stopped = true;
        worker?.terminate();
      };
    }

    const video: MediaTrackConstraints = chosen ? { ...WANTED, deviceId: { exact: chosen } } : WANTED;

    navigator.mediaDevices
      .getUserMedia({ video })
      .then(async (granted) => {
        if (stopped) {
          granted.getTracks().forEach((track) => track.stop());
          return;
        }
        stream = granted;
        const track = granted.getVideoTracks()[0];
        const settings = track?.getSettings();
        report(`opened ${track?.label || "sin nombre"} at ${settings?.width}x${settings?.height}`);

        // Cameras that can be told to keep focusing are told to: a webcam
        // parked at infinity never resolves a code held in front of it.
        await track
          ?.applyConstraints({
            advanced: [{ focusMode: "continuous" } as MediaTrackConstraintSet],
          })
          .catch(() => undefined);

        if (videoRef.current) {
          videoRef.current.srcObject = granted;
          await videoRef.current.play().catch(() => undefined);
        }

        // Labels are blank until permission is granted, so the list of cameras
        // is only worth building now — and only shown when there is a choice.
        const devices = await navigator.mediaDevices.enumerateDevices().catch(() => []);
        if (!stopped) setCameras(devices.filter((d) => d.kind === "videoinput"));

        next(0);
      })
      .catch((error: unknown) => {
        report(describe(error));
        setProblem(explainCameraFailure(error));
      });

    return () => {
      stopped = true;
      window.clearTimeout(timer);
      worker?.terminate();
      stream?.getTracks().forEach((track) => track.stop());
    };
  }, [onScanned, report, chosen]);

  return (
    <div className="scanner">
      {problem ? (
        <p className="problem">{problem}</p>
      ) : (
        <>
          <div className="scanner-frame">
            <video ref={videoRef} className="scanner-video" muted playsInline />
            <div className="scanner-target" aria-hidden />
          </div>
          <p className="hint">Encuadra el QR dentro del recuadro.</p>
          {cameras.length > 1 && (
            <select
              className="scanner-pick"
              value={chosen ?? cameras[0]?.deviceId ?? ""}
              onChange={(e) => setChosen(e.target.value)}
            >
              {cameras.map((camera, index) => (
                <option key={camera.deviceId} value={camera.deviceId}>
                  {camera.label || `Cámara ${index + 1}`}
                </option>
              ))}
            </select>
          )}
        </>
      )}
      <button className="btn btn-small" onClick={onClose} type="button">
        <CloseIcon />
        Cancelar
      </button>
    </div>
  );
}

/** The whole of what went wrong, for the log rather than the screen. */
function describe(error: unknown): string {
  if (typeof error === "object" && error !== null) {
    const named = error as { name?: string; message?: string };
    return `${named.name ?? "Error"}: ${named.message ?? ""} (secureContext=${window.isSecureContext})`;
  }
  return String(error);
}

function explainCameraFailure(error: unknown): string {
  const name = typeof error === "object" && error !== null && "name" in error ? String(error.name) : "";
  if (name === "NotAllowedError") {
    return "No se pudo usar la cámara: el permiso está denegado. Cancela y pega el código.";
  }
  if (name === "NotFoundError" || name === "OverconstrainedError") {
    return "Este ordenador no tiene ninguna cámara disponible. Cancela y pega el código.";
  }
  if (name === "NotReadableError") {
    return "La cámara está ocupada por otro programa. Ciérralo, o cancela y pega el código.";
  }
  return `No se pudo abrir la cámara (${name || "motivo desconocido"}). Cancela y pega el código.`;
}
