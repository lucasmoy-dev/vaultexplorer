import { useEffect, useRef, useState } from "react";
import jsQR from "jsqr";
import { CloseIcon } from "./Icons";

/**
 * Reads a pairing code off another device's screen with this one's camera.
 *
 * The desktop drew a QR and told the user to scan it from the phone; going the
 * other way meant typing a hundred characters into a keyboard. A laptop has a
 * camera, so it can do what it was asking the phone to do.
 *
 * Decoding happens here rather than in the browser's own barcode detector,
 * which WebKitGTK — the engine behind this window on Linux — does not
 * implement.
 */
export function QrScanner({
  onScanned,
  onClose,
}: {
  onScanned: (code: string) => void;
  onClose: () => void;
}) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    let stream: MediaStream | null = null;
    let frame = 0;
    let stopped = false;

    // Reused across frames: allocating a canvas per frame is what turns a scan
    // into a slideshow.
    const canvas = document.createElement("canvas");
    const context = canvas.getContext("2d", { willReadFrequently: true });

    function read() {
      if (stopped) return;
      const video = videoRef.current;
      if (video && context && video.readyState === video.HAVE_ENOUGH_DATA) {
        canvas.width = video.videoWidth;
        canvas.height = video.videoHeight;
        context.drawImage(video, 0, 0, canvas.width, canvas.height);
        const image = context.getImageData(0, 0, canvas.width, canvas.height);
        const found = jsQR(image.data, image.width, image.height, {
          inversionAttempts: "dontInvert",
        });
        if (found?.data) {
          stopped = true;
          onScanned(found.data.trim());
          return;
        }
      }
      frame = requestAnimationFrame(read);
    }

    navigator.mediaDevices
      ?.getUserMedia({ video: { facingMode: "environment" } })
      .then((granted) => {
        if (stopped) {
          granted.getTracks().forEach((track) => track.stop());
          return;
        }
        stream = granted;
        if (videoRef.current) {
          videoRef.current.srcObject = granted;
          void videoRef.current.play();
        }
        frame = requestAnimationFrame(read);
      })
      .catch((error: unknown) => {
        // A camera that will not open is worth one plain sentence, not a
        // DOMException: pasting the code still works and is right there.
        setProblem(explainCameraFailure(error));
      });

    return () => {
      stopped = true;
      cancelAnimationFrame(frame);
      stream?.getTracks().forEach((track) => track.stop());
    };
  }, [onScanned]);

  return (
    <div className="scanner">
      {problem ? (
        <p className="problem">{problem}</p>
      ) : (
        <>
          <video ref={videoRef} className="scanner-video" muted playsInline />
          <p className="hint">Apunta a la pantalla del otro dispositivo.</p>
        </>
      )}
      <button className="btn btn-small" onClick={onClose} type="button">
        <CloseIcon />
        Cerrar la cámara
      </button>
    </div>
  );
}

function explainCameraFailure(error: unknown): string {
  const name = typeof error === "object" && error !== null && "name" in error ? String(error.name) : "";
  if (name === "NotAllowedError") {
    return "No se pudo usar la cámara: el permiso está denegado. Pega el código en su lugar.";
  }
  if (name === "NotFoundError" || name === "OverconstrainedError") {
    return "Este ordenador no tiene ninguna cámara disponible. Pega el código en su lugar.";
  }
  if (name === "NotReadableError") {
    return "La cámara está ocupada por otro programa. Ciérralo, o pega el código.";
  }
  return `No se pudo abrir la cámara (${name || "motivo desconocido"}). Pega el código en su lugar.`;
}
