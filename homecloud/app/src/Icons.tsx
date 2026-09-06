/**
 * The icon set, drawn inline.
 *
 * Inline rather than an icon font or a package because there are a dozen of
 * them and they all inherit `currentColor`, which is what makes them work
 * unchanged on a primary button, a quiet button and a dark window.
 *
 * Every icon here sits next to a word. None of them is the only thing carrying
 * a meaning, so they are hidden from screen readers rather than labelled twice.
 */

function Svg({ children, size = 17 }: { children: React.ReactNode; size?: number }) {
  return (
    <svg
      className="icon"
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {children}
    </svg>
  );
}

export const ShareIcon = () => (
  <Svg>
    <path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4l1.8 2.2h7.2A1.5 1.5 0 0 1 19 9.7v7.8a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 3 17.5z" />
    <path d="M12 15.5v-4.6M12 10.9l-1.8 1.8M12 10.9l1.8 1.8" />
  </Svg>
);

export const JoinIcon = () => (
  <Svg>
    <path d="M10.5 13.5a4 4 0 0 0 5.7 0l2.6-2.6a4 4 0 1 0-5.7-5.7L11.8 6.5" />
    <path d="M13.5 10.5a4 4 0 0 0-5.7 0l-2.6 2.6a4 4 0 1 0 5.7 5.7l1.3-1.3" />
  </Svg>
);

export const QrIcon = () => (
  <Svg>
    <rect x="3.5" y="3.5" width="6.5" height="6.5" rx="1.2" />
    <rect x="14" y="3.5" width="6.5" height="6.5" rx="1.2" />
    <rect x="3.5" y="14" width="6.5" height="6.5" rx="1.2" />
    <path d="M14 14h3v3h-3zM20.5 14v3M17.5 20.5h3M14 20.5h.01" />
  </Svg>
);

export const CopyIcon = () => (
  <Svg>
    <rect x="9" y="9" width="11" height="11" rx="2" />
    <path d="M5 15a2 2 0 0 1-1-1.7V6a2 2 0 0 1 2-2h7.3A2 2 0 0 1 15 5" />
  </Svg>
);

export const CheckIcon = () => (
  <Svg>
    <path d="M4.5 12.8l4.6 4.6L19.5 7" />
  </Svg>
);

export const FolderIcon = () => (
  <Svg>
    <path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4l1.8 2.2h7.2A1.5 1.5 0 0 1 19 9.7v7.8a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 3 17.5z" />
  </Svg>
);

export const PencilIcon = () => (
  <Svg>
    <path d="M4 20l4.3-1.1L19.1 8.1a2 2 0 0 0 0-2.8l-.4-.4a2 2 0 0 0-2.8 0L5.1 15.7z" />
    <path d="M14.8 6.6l2.6 2.6" />
  </Svg>
);

export const PauseIcon = () => (
  <Svg>
    <path d="M9.5 5.5v13M14.5 5.5v13" />
  </Svg>
);

export const PlayIcon = () => (
  <Svg>
    <path d="M8 5.4l10 6.6-10 6.6z" />
  </Svg>
);

export const TrashIcon = () => (
  <Svg>
    <path d="M4.5 7h15M9.5 7V5.2A1.2 1.2 0 0 1 10.7 4h2.6a1.2 1.2 0 0 1 1.2 1.2V7" />
    <path d="M6.5 7l.8 11.4A1.6 1.6 0 0 0 8.9 20h6.2a1.6 1.6 0 0 0 1.6-1.6L17.5 7" />
  </Svg>
);

export const PlusIcon = () => (
  <Svg>
    <path d="M12 5.5v13M5.5 12h13" />
  </Svg>
);

export const CloseIcon = () => (
  <Svg>
    <path d="M6.5 6.5l11 11M17.5 6.5l-11 11" />
  </Svg>
);

export const BroomIcon = () => (
  <Svg>
    <path d="M14.5 4.5l5 5" />
    <path d="M16.8 7.2L9.6 14.4" />
    <path d="M9.6 14.4l-4.4 1.2a1 1 0 0 0-.6 1.5l2.3 3.1a1 1 0 0 0 1.6 0l2.7-3.6z" />
  </Svg>
);

export const RefreshIcon = () => (
  <Svg>
    <path d="M20 12a8 8 0 1 1-2.34-5.66" />
    <path d="M20 4.5V10h-5.5" />
  </Svg>
);

export const ClockIcon = () => (
  <Svg>
    <circle cx="12" cy="12" r="8.2" />
    <path d="M12 7.4V12l3 1.8" />
  </Svg>
);

export const GearIcon = () => (
  <Svg size={18}>
    <circle cx="12" cy="12" r="3.1" />
    <path d="M18.9 14.4a1.6 1.6 0 0 0 .3 1.8l.1.1a1.9 1.9 0 1 1-2.7 2.7l-.1-.1a1.6 1.6 0 0 0-1.8-.3 1.6 1.6 0 0 0-1 1.5v.2a1.9 1.9 0 1 1-3.8 0v-.1a1.6 1.6 0 0 0-1-1.5 1.6 1.6 0 0 0-1.8.3l-.1.1a1.9 1.9 0 1 1-2.7-2.7l.1-.1a1.6 1.6 0 0 0 .3-1.8 1.6 1.6 0 0 0-1.5-1h-.2a1.9 1.9 0 1 1 0-3.8h.1a1.6 1.6 0 0 0 1.5-1 1.6 1.6 0 0 0-.3-1.8l-.1-.1a1.9 1.9 0 1 1 2.7-2.7l.1.1a1.6 1.6 0 0 0 1.8.3h.1a1.6 1.6 0 0 0 1-1.5v-.2a1.9 1.9 0 1 1 3.8 0v.1a1.6 1.6 0 0 0 1 1.5 1.6 1.6 0 0 0 1.8-.3l.1-.1a1.9 1.9 0 1 1 2.7 2.7l-.1.1a1.6 1.6 0 0 0-.3 1.8v.1a1.6 1.6 0 0 0 1.5 1h.2a1.9 1.9 0 1 1 0 3.8h-.1a1.6 1.6 0 0 0-1.5 1z" />
  </Svg>
);
