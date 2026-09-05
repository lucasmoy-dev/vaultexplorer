import { useCallback, useEffect, useRef, useState } from "react";
import jsQR from "jsqr";
import { api } from "./api";
import { CloseIcon } from "./Icons";

/**
 * Reads a pairing code off another device's screen with this one's camera.
 *
 * Decoding happens here rather than in the browser's own barcode detector,
 * which WebKitGTK — the engine behind this window on Linux — does not
 * implement.
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
    let frame = 0;
    let stopped = false;

    // One canvas for the whole session: allocating per frame is what turns a
    // scan into a slideshow.
    const canvas = document.createElement("canvas");
    canvas.width = DECODE_SIZE;
    canvas.height = DECODE_SIZE;
    const context = canvas.getContext("2d", { willReadFrequently: true });

    function read() {
      if (stopped) return;
      const video = videoRef.current;
      if (video && context && video.readyState >= video.HAVE_CURRENT_DATA) {
        const side = Math.min(video.videoWidth, video.videoHeight);
        if (side > 0) {
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
          const image = context.getImageData(0, 0, DECODE_SIZE, DECODE_SIZE);
          // Both polarities: a QR shown on a dark-mode screen comes back
          // inverted, and refusing to try costs a scan that would have worked.
          const found = jsQR(image.data, image.width, image.height, {
            inversionAttempts: "attemptBoth",
          });
          if (found?.data) {
            stopped = true;
            onScanned(found.data.trim());
            return;
          }
        }
      }
      frame = requestAnimationFrame(read);
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

        if (videoRef.current) {
          videoRef.current.srcObject = granted;
          await videoRef.current.play().catch(() => undefined);
        }

        // Labels are blank until permission is granted, so the list of cameras
        // is only worth building now — and only shown when there is a choice.
        const devices = await navigator.mediaDevices.enumerateDevices().catch(() => []);
        if (!stopped) setCameras(devices.filter((d) => d.kind === "videoinput"));

        frame = requestAnimationFrame(read);
      })
      .catch((error: unknown) => {
        report(describe(error));
        setProblem(explainCameraFailure(error));
      });

    return () => {
      stopped = true;
      cancelAnimationFrame(frame);
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
