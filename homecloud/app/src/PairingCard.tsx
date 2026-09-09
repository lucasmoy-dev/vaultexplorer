import { useEffect, useState } from "react";
import { copyText } from "./clipboard";
import QRCode from "qrcode";
import { CheckIcon, CopyIcon } from "./Icons";

/**
 * The screen that hands a folder to another device.
 *
 * The QR and the text are the same code; which one is easier depends entirely
 * on whether the other device has a camera.
 *
 * Whoever redeems the code is let in without anyone confirming again, so the
 * card says so plainly: an open door is only acceptable when you can see that
 * it is open.
 */
export function PairingCard({ code, label }: { code: string; label: string }) {
  const [qr, setQr] = useState<string | null>(null);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");

  useEffect(() => {
    // Drawn as large as the sheet allows and with the lightest error
    // correction: this is read off a lit screen a hand's width away, where
    // spare correction buys nothing and only packs the squares tighter. The
    // wider the squares, the sooner the other device's camera locks on.
    QRCode.toDataURL(code, { margin: 2, width: 320, errorCorrectionLevel: "L" })
      .then(setQr)
      .catch(() => setQr(null));
  }, [code]);

  // A copy button that quietly does nothing is worse than no copy button: the
  // user walks away believing they have the code.
  async function copy() {
    setCopyState((await copyText(code)) ? "copied" : "failed");
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
        {copyState === "copied" ? <CheckIcon /> : <CopyIcon />}
        {copyState === "copied" ? "Copiado" : copyState === "failed" ? "No se pudo copiar" : "Copiar código"}
      </button>
      {copyState === "failed" && (
        <p className="pairing-note">
          Selecciona el código de arriba y cópialo a mano, o escanea el QR.
        </p>
      )}
      <p className="pairing-window">
        Quien lo use entrará solo, sin que tengas que aceptar nada más.
      </p>
      <p className="pairing-note">
        Cualquiera con este código puede entrar en «{label}». No lo publiques.
      </p>
    </div>
  );
}
