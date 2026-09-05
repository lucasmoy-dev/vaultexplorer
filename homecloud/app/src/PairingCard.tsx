import { useEffect, useState } from "react";
import { copyText } from "./clipboard";
import QRCode from "qrcode";
import { api, type PairingWindow } from "./api";
import { CheckIcon, CopyIcon } from "./Icons";

/**
 * The screen that hands a folder to another device.
 *
 * The QR and the text are the same code; which one is easier depends entirely
 * on whether the other device has a camera.
 *
 * While this is on screen the folder is accepting whoever redeems the code, so
 * the countdown is part of the card rather than a detail somewhere else: an
 * open door is only acceptable when you can see it is open.
 */
export function PairingCard({ code, label }: { code: string; label: string }) {
  const [qr, setQr] = useState<string | null>(null);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const [window_, setWindow] = useState<PairingWindow | null>(null);

  useEffect(() => {
    QRCode.toDataURL(code, { margin: 1, width: 260, errorCorrectionLevel: "M" })
      .then(setQr)
      .catch(() => setQr(null));
  }, [code]);

  useEffect(() => {
    let live = true;
    const tick = () => {
      void api.pairingWindow().then((w) => {
        if (live) setWindow(w);
      });
    };
    tick();
    const timer = setInterval(tick, 1000);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, []);

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
      {window_ && (
        <p className="pairing-window">
          Entrará solo, sin que tengas que aceptar nada más, durante{" "}
          <strong>{formatCountdown(window_.secondsLeft)}</strong>.
        </p>
      )}
      <p className="pairing-note">
        Cualquiera con este código puede entrar en «{label}». No lo publiques.
      </p>
    </div>
  );
}

function formatCountdown(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (minutes === 0) return `${rest} s`;
  return `${minutes}:${String(rest).padStart(2, "0")} min`;
}
