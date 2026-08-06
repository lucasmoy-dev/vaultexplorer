import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

export type MenuItem =
  | { type: "separator" }
  | {
      type: "submenu";
      label: string;
      disabled?: boolean;
      items?: MenuItem[];
      // For a submenu whose contents are expensive to compute (e.g. "Open
      // With…" enumerating every app registered for a file's MIME type) --
      // fetched once on first hover instead of up front for every right-
      // click, so opening the menu itself never has to wait on it. Mutually
      // exclusive with `items`; `MenuRows` shows a disabled "Loading…" row
      // until this resolves, then caches the result for the rest of this
      // menu's lifetime.
      loadItems?: () => Promise<MenuItem[]>;
    }
  | {
      type?: "item";
      label: string;
      shortcut?: string;
      danger?: boolean;
      disabled?: boolean;
      swatch?: string;
      onClick: () => void;
    };

export type MenuState = {
  x: number;
  y: number;
  items: MenuItem[];
  // When set, `y` is where the menu's *bottom* edge should land (it grows
  // upward from there) instead of the usual top-left anchor -- for a
  // trigger button near the bottom of the screen, where opening downward
  // (the default) immediately overflows and gets clamped back up to
  // wherever it lands, not necessarily near the button at all.
  anchorBottom?: boolean;
} | null;

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

const LOADING_ITEMS: MenuItem[] = [{ label: "Loading…", disabled: true, onClick: () => {} }];

function MenuRows({ items, onClose }: { items: MenuItem[]; onClose: () => void }) {
  const [openSub, setOpenSub] = useState<number | null>(null);
  const wrapperRefs = useRef<Record<number, HTMLDivElement | null>>({});
  // Cache for `loadItems` submenus -- fetched once per index the first
  // time it's opened, kept for the rest of this menu instance's lifetime
  // so re-hovering the same submenu doesn't refire the fetch.
  const [loadedItems, setLoadedItems] = useState<Record<number, MenuItem[]>>({});
  const loadingRef = useRef<Set<number>>(new Set());
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
          const openSubmenu = () => {
            if (item.disabled) return;
            cancelClose();
            setOpenSub(i);
            if (item.loadItems && !loadedItems[i] && !loadingRef.current.has(i)) {
              loadingRef.current.add(i);
              item.loadItems().then((resolved) => {
                loadingRef.current.delete(i);
                setLoadedItems((prev) => ({ ...prev, [i]: resolved }));
              });
            }
          };
          return (
            <div
              key={i}
              className="context-item-wrapper"
              ref={(el) => {
                wrapperRefs.current[i] = el;
              }}
              onMouseEnter={openSubmenu}
              onMouseLeave={() => scheduleClose(i)}
            >
              <button
                className="context-item"
                disabled={item.disabled}
                // Hover alone (above) never fires on touch -- tapping the
                // row is the only way a submenu opens there, so toggle on
                // click too. Harmless for mouse users: it just means a
                // click does the same thing hover already did.
                onClick={() => (openSub === i ? setOpenSub(null) : openSubmenu())}
              >
                <span className="context-label-group">
                  <span className="context-label">{item.label}</span>
                </span>
                <span className="context-shortcut">›</span>
              </button>
              {openSub === i && wrapperRefs.current[i] && (
                <SubmenuFlyout
                  items={item.loadItems ? loadedItems[i] ?? LOADING_ITEMS : item.items ?? []}
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

// Native <select> popups render with the platform's raw (WebKitGTK: plain
// white) list style that CSS can't reach into on Linux -- this is a themed
// stand-in for settings-style pickers: a button showing the current value,
// opening the same dark `.context-menu` popup the app already uses for
// right-click menus (checkmark on the selected row, same as the view-mode
// picker in App.tsx).
export function Dropdown<T extends string>({
  value,
  options,
  onChange,
  className,
  disabled,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  className?: string;
  disabled?: boolean;
}) {
  const [menu, setMenu] = useState<MenuState>(null);
  const current = options.find((o) => o.value === value);
  return (
    <>
      <button
        type="button"
        disabled={disabled}
        className={`settings-select dropdown-trigger ${className ?? ""}`}
        onClick={(e) => {
          // A second click on the trigger while its own menu is already
          // open closes it -- without this, the window-level "click
          // outside" listener in ContextMenu (deferred a tick so it can't
          // self-close on the *opening* click) still doesn't fire until a
          // tick later, so this same click would otherwise just reopen it
          // via `setMenu` immediately after that listener's close.
          if (menu) {
            setMenu(null);
            return;
          }
          const r = e.currentTarget.getBoundingClientRect();
          setMenu({
            x: r.left,
            y: r.bottom + 4,
            items: options.map((o) => ({
              label: o.value === value ? `✓ ${o.label}` : o.label,
              onClick: () => onChange(o.value),
            })),
          });
        }}
      >
        <span>{current?.label ?? value}</span>
        <span className="dropdown-caret" aria-hidden="true">
          ▾
        </span>
      </button>
      <ContextMenu state={menu} onClose={() => setMenu(null)} />
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
    let top = state.anchorBottom ? state.y - menu.height : state.y;
    if (left + menu.width + pad > window.innerWidth) left = window.innerWidth - menu.width - pad;
    if (top + menu.height + pad > window.innerHeight) top = window.innerHeight - menu.height - pad;
    setPos({ left: Math.max(pad, left), top: Math.max(pad, top) });
  }, [state]);

  useEffect(() => {
    if (!state) return;
    // Target-based, not propagation-based: a submenu flyout is portaled to
    // <body> as a *sibling* of the root menu, not a DOM descendant, so
    // `ref.current.contains(e.target)` would miss it -- but both share the
    // `context-menu` class, so `closest` catches either.
    //
    // Listening for `click`, not `mousedown`, is the other half of this:
    // Android WebView's touch-to-mouse synthesis for a tap fires an "extra"
    // mousedown with `target` set to something generic (`document`/`body`,
    // not the actually-tapped element) rather than the real target -- so
    // even the `closest` check above can't save a mousedown-based listener
    // from it. Empirically, that extra mousedown has no matching phantom
    // `click` alongside it, so anchoring on `click` instead sidesteps the
    // whole class of bug rather than chasing each of its symptoms (this
    // replaced an earlier rAF-deferred `mousedown` listener that only fixed
    // the *opening* tap, not later ones -- tapping "More" to open its own
    // submenu was closing the whole menu instead, on every tap).
    // Capture phase + stopPropagation, not the plain bubble listener this
    // used to be: a bubble-phase listener on `window` only runs *after*
    // whatever's directly under the tap has already handled its own click
    // (React dispatches from the target outward), so dismissing the menu
    // this way still let the same tap select/open the file underneath it
    // (confirmed live: tapping outside the View Options menu to close it
    // also activated whatever grid entry happened to be there). Capturing
    // on `window` fires before the event ever reaches that entry, and
    // stopping it there means the entry's own handler never runs at all.
    const closeIfOutside = (e: Event) => {
      const target = e.target as Element | null;
      if (target?.closest(".context-menu")) return;
      e.stopPropagation();
      onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    // Still deferred one tick: the *opening* tap itself (e.g. the long-press
    // that first summoned this menu) lands squarely outside `.context-menu`
    // by definition, so the target-check alone can't save it from a same-
    // tick phantom click.
    const attach = requestAnimationFrame(() => {
      window.addEventListener("click", closeIfOutside, true);
      window.addEventListener("resize", onClose);
    });
    window.addEventListener("keydown", onKey);
    return () => {
      cancelAnimationFrame(attach);
      window.removeEventListener("click", closeIfOutside, true);
      window.removeEventListener("resize", onClose);
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
