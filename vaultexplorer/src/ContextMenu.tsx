import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

export type MenuItem =
  | { type: "separator" }
  | { type: "submenu"; label: string; disabled?: boolean; items: MenuItem[] }
  | {
      type?: "item";
      label: string;
      shortcut?: string;
      danger?: boolean;
      disabled?: boolean;
      swatch?: string;
      onClick: () => void;
    };

export type MenuState = { x: number; y: number; items: MenuItem[] } | null;

// Off-screen until measured -- avoids a flash at the wrong spot before the
// layout effect below repositions it (that effect runs before paint, so
// this is never actually visible).
const OFFSCREEN = { left: -9999, top: -9999 };

function SubmenuFlyout({
  items,
  onClose,
  anchor,
  onEnter,
  onLeave,
}: {
  items: MenuItem[];
  onClose: () => void;
  anchor: HTMLDivElement;
  onEnter: () => void;
  onLeave: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState(OFFSCREEN);

  useLayoutEffect(() => {
    if (!ref.current) return;
    const a = anchor.getBoundingClientRect();
    const menu = ref.current.getBoundingClientRect();
    const pad = 8;
    // Prefer opening to the right of the parent item; flip to the left
    // if there isn't room (the whole reason submenus used to vanish off
    // the edge of the window).
    let left = a.right + 2;
    if (left + menu.width + pad > window.innerWidth) left = a.left - menu.width - 2;
    left = Math.max(pad, left);
    let top = a.top - 6;
    if (top + menu.height + pad > window.innerHeight) top = window.innerHeight - menu.height - pad;
    top = Math.max(pad, top);
    setPos({ left, top });
  }, [anchor]);

  // Portaled to <body> -- the root menu has `backdrop-filter` for its blur
  // effect, which (per spec) makes it a containing block for `position:
  // fixed` descendants. Left nested in the DOM, this submenu's fixed
  // coordinates would resolve against that blurred ancestor box instead
  // of the real viewport, landing it far off wherever `getBoundingClientRect`
  // (always viewport-relative) told it to go. Escaping to `body` avoids
  // that containing-block trap entirely.
  return createPortal(
    <div
      ref={ref}
      className="context-menu context-submenu"
      style={{ left: pos.left, top: pos.top }}
      onMouseDown={(e) => e.stopPropagation()}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
    >
      <MenuRows items={items} onClose={onClose} />
    </div>,
    document.body
  );
}

function MenuRows({ items, onClose }: { items: MenuItem[]; onClose: () => void }) {
  const [openSub, setOpenSub] = useState<number | null>(null);
  const wrapperRefs = useRef<Record<number, HTMLDivElement | null>>({});
  // Portaling the submenu to <body> (see above) means it's no longer a DOM
  // descendant of the row that opens it, so a native mouseleave fires the
  // instant the pointer crosses from the row into the (now-sibling-in-the-
  // DOM) submenu. A short close delay, cancelled if either the row or the
  // submenu itself is re-entered, is the standard fix (same pattern every
  // hover-menu library uses).
  const closeTimer = useRef<number | null>(null);
  const cancelClose = () => {
    if (closeTimer.current !== null) {
      window.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
  };
  const scheduleClose = (i: number) => {
    cancelClose();
    closeTimer.current = window.setTimeout(() => {
      setOpenSub((s) => (s === i ? null : s));
    }, 200);
  };

  return (
    <>
      {items.map((item, i) => {
        if (item.type === "separator") return <div key={i} className="context-sep" />;

        if (item.type === "submenu") {
          return (
            <div
              key={i}
              className="context-item-wrapper"
              ref={(el) => {
                wrapperRefs.current[i] = el;
              }}
              onMouseEnter={() => {
                if (!item.disabled) {
                  cancelClose();
                  setOpenSub(i);
                }
              }}
              onMouseLeave={() => scheduleClose(i)}
            >
              <button className="context-item" disabled={item.disabled}>
                <span className="context-label-group">
                  <span className="context-label">{item.label}</span>
                </span>
                <span className="context-shortcut">›</span>
              </button>
              {openSub === i && wrapperRefs.current[i] && (
                <SubmenuFlyout
                  items={item.items}
                  onClose={onClose}
                  anchor={wrapperRefs.current[i]!}
                  onEnter={cancelClose}
                  onLeave={() => scheduleClose(i)}
                />
              )}
            </div>
          );
        }

        return (
          <button
            key={i}
            className={`context-item ${item.danger ? "danger" : ""}`}
            disabled={item.disabled}
            onClick={() => {
              onClose();
              item.onClick();
            }}
          >
            <span className="context-label-group">
              {item.swatch && (
                <span className="context-swatch" style={{ background: item.swatch }} />
              )}
              <span className="context-label">{item.label}</span>
            </span>
            {item.shortcut && <span className="context-shortcut">{item.shortcut}</span>}
          </button>
        );
      })}
    </>
  );
}

export function ContextMenu({ state, onClose }: { state: MenuState; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number }>({ left: 0, top: 0 });

  // Clamp to viewport so the menu never spills off-screen.
  useLayoutEffect(() => {
    if (!state || !ref.current) return;
    const menu = ref.current.getBoundingClientRect();
    const pad = 8;
    let left = state.x;
    let top = state.y;
    if (left + menu.width + pad > window.innerWidth) left = window.innerWidth - menu.width - pad;
    if (top + menu.height + pad > window.innerHeight) top = window.innerHeight - menu.height - pad;
    setPos({ left: Math.max(pad, left), top: Math.max(pad, top) });
  }, [state]);

  useEffect(() => {
    if (!state) return;
    const close = () => onClose();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", close);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [state, onClose]);

  if (!state) return null;

  return (
    <div
      ref={ref}
      className="context-menu"
      style={{ left: pos.left, top: pos.top }}
      onMouseDown={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.preventDefault()}
    >
      <MenuRows items={state.items} onClose={onClose} />
    </div>
  );
}
