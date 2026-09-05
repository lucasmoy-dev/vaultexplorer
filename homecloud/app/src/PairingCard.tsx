import { useEffect, useState } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import QRCode from "qrcode";

/**
 * The screen you leave open on one device while you point the other one at it.
 * The QR and the text are the same code; which one is easier depends entirely
 * on whether the other device has a camera.
 */
export function PairingCard({ code, label }: { code: string; label: string }) {
  const [qr, setQr] = useState<string | null>(null);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");

  useEffect(() => {
    QRCode.toDataURL(code, { margin: 1, width: 260, errorCorrectionLevel: "M" })
      .then(setQr)
      .catch(() => setQr(null));
  }, [code]);

  // A copy button that quietly does nothing is worse than no copy button: the
  // user walks away believing they have the code.
  async function copy() {
    try {
      await writeText(code);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
    setTimeout(() => setCopyState("idle"), 2400);
  }

  return (
    <div className="pairing">
      <p className="pairing-lead">
        Escanea esto desde el otro dispositivo, o pega el código.
      </p>
      {qr ? (
        <img className="pairing-qr" src={qr} alt={`Código QR para compartir ${label}`} />
      ) : (
        <div className="pairing-qr pairing-qr-empty">No se pudo dibujar el QR</div>
      )}
      <code className="pairing-code">{code}</code>
      <button className="btn btn-primary" onClick={copy}>
        {copyState === "copied" ? "Copiado" : copyState === "failed" ? "No se pudo copiar" : "Copiar código"}
      </button>
      {copyState === "failed" && (
        <p className="pairing-note">
          Selecciona el código de arriba y cópialo a mano.
        </p>
      )}
      <p className="pairing-note">
        Cualquiera con este código puede entrar en «{label}». No lo publiques.
      </p>
    </div>
  );
}
