import { useEffect, useRef, useState } from "react";
import { api } from "../api";

// Debounced auto-save shared by the plain-text and markdown editors --
// keystrokes update local state immediately, but the actual write waits
// for a short pause so every keystroke doesn't hit disk (or, for a vault
// file, re-encrypt) individually.
export function useAutoSaveText(
  fullPath: string,
  inVault: boolean
): {
  content: string | null;
  error: string;
  saving: boolean;
  setContent: (value: string) => void;
  externalRevision: number;
} {
  const [content, setContentState] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  // Bumped only when content changes from *outside* this pane's own
  // typing (the focus-triggered re-read below) -- MarkdownEditorPane's
  // preview mode needs to tell "the user is typing, don't touch the DOM"
  // apart from "the file changed externally, do re-render" and `content`
  // alone can't distinguish those two cases.
  const [externalRevision, setExternalRevision] = useState(0);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    setContentState(null);
    setError("");
    const read = inVault ? api.vaultReadText(fullPath) : api.fsReadText(fullPath);
    read.then(setContentState).catch((e) => setError(String(e)));
    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
    };
  }, [fullPath, inVault]);

  // Re-read from disk whenever the window regains focus -- the common way
  // this file actually changes out from under the open preview is the
  // user switching to an external editor (Sublime, etc.), saving there,
  // then switching back. Without this, the pane just keeps showing
  // whatever it loaded at open time, which reads as "my external edit
  // didn't really save" even though it did. Skipped while a local edit's
  // debounced save is still pending, so this can't clobber the user's own
  // in-app typing.
  useEffect(() => {
    function onFocus() {
      if (saveTimer.current) return;
      const read = inVault ? api.vaultReadText(fullPath) : api.fsReadText(fullPath);
      read
        .then((text) => {
          setContentState(text);
          setExternalRevision((v) => v + 1);
        })
        .catch(() => {});
    }
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [fullPath, inVault]);

  function setContent(value: string) {
    setContentState(value);
    if (saveTimer.current) clearTimeout(saveTimer.current);
    setSaving(true);
    saveTimer.current = setTimeout(() => {
      const write = inVault ? api.vaultWriteText(fullPath, value) : api.fsWriteText(fullPath, value);
      write
        .then(() => setSaving(false))
        .catch((e) => {
          setError(String(e));
          setSaving(false);
        });
    }, 500);
  }

  return { content, error, saving, setContent, externalRevision };
}
