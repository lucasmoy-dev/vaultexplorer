export type Tab = "home" | "checkin" | "goals" | "history" | "settings";

const TABS: { id: Tab; icon: string; label: string }[] = [
  { id: "home", icon: "📊", label: "Hoy" },
  { id: "checkin", icon: "✅", label: "Check-in" },
  { id: "goals", icon: "🎯", label: "Metas" },
  { id: "history", icon: "📈", label: "Historial" },
  { id: "settings", icon: "⚙️", label: "Ajustes" },
];

export function BottomNav({ tab, onChange }: { tab: Tab; onChange: (t: Tab) => void }) {
  return (
    <nav className="bottomnav">
      {TABS.map((t) => (
        <button
          key={t.id}
          className={"nav-item" + (tab === t.id ? " active" : "")}
          onClick={() => onChange(t.id)}
        >
          <span className="nav-icon">{t.icon}</span>
          <span className="nav-label">{t.label}</span>
        </button>
      ))}
    </nav>
  );
}
