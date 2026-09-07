import { useEffect, useRef, useState } from "react";

/**
 * A small "?" that only shows its explanation when someone actually wants it.
 *
 * The alternative — every nuance sitting on screen as a paragraph — is how a
 * screen that shows one folder's sharing controls ends up looking like a
 * page of terms and conditions. Most people never need the explanation;
 * anyone who does gets the whole thing, not a shortened version.
 */
export function Help({ children }: { children: React.ReactNode }) {
  const [open, setOpen] = useState(false);
  const wrapperRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (!open) return;
    function onDocumentClick(e: MouseEvent) {
      if (wrapperRef.current && !wrapperRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDocumentClick);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onDocumentClick);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <span className="help" ref={wrapperRef}>
      <button
        type="button"
        className="help-btn"
        aria-label="Ayuda"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        ?
      </button>
      {open && (
        <div className="help-pop" role="tooltip">
          {children}
        </div>
      )}
    </span>
  );
}
