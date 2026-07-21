import { useEffect, useRef } from "react";

// macOS-style "no" shake on a wrong password: retriggers the `.shake` CSS
// animation on `error` going non-empty. Toggling the class via a ref
// (remove, force reflow, re-add) restarts the same animation reliably --
// unlike keying the element to force a remount, this doesn't blow away
// the password input's focus/native selection state.
export function useShakeOnError<T extends HTMLElement>(error: string) {
  const ref = useRef<T>(null);
  useEffect(() => {
    if (!error) return;
    const el = ref.current;
    if (!el) return;
    el.classList.remove("shake");
    void el.offsetWidth;
    el.classList.add("shake");
  }, [error]);
  return ref;
}
