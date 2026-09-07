import { useEffect, useState } from "react";

/**
 * A message that appears for a moment and goes away on its own.
 *
 * Module-level rather than passed down as a prop: the thing worth confirming
 * — "the link is on your clipboard" — usually happens in a component nested
 * a few sheets deep, and threading a callback through every layer just to
 * show one line of text would be more code than the feature.
 */

type Listener = (message: string) => void;
let listener: Listener | null = null;

export function showToast(message: string) {
  listener?.(message);
}

/** Mounted once, near the root, so it sits above every sheet. */
export function ToastHost() {
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    listener = (next) => setMessage(next);
    return () => {
      listener = null;
    };
  }, []);

  useEffect(() => {
    if (!message) return;
    const timer = setTimeout(() => setMessage(null), 2200);
    return () => clearTimeout(timer);
  }, [message]);

  if (!message) return null;
  return (
    <div className="toast" role="status">
      {message}
    </div>
  );
}
