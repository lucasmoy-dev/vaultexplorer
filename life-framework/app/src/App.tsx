import { useState } from "react";
import { StoreProvider } from "./store";
import { BottomNav, type Tab } from "./components/BottomNav";
import { Home } from "./screens/Home";
import { AreaDetail } from "./screens/AreaDetail";
import { CheckIn } from "./screens/CheckIn";
import { Goals } from "./screens/Goals";
import { History } from "./screens/History";
import { Settings } from "./screens/Settings";

function Shell() {
  const [tab, setTab] = useState<Tab>("home");
  const [areaId, setAreaId] = useState<string | null>(null);
  const openArea = (id: string) => setAreaId(id);

  return (
    <div className="app">
      <main className="content">
        {areaId ? (
          <AreaDetail areaId={areaId} onBack={() => setAreaId(null)} />
        ) : tab === "home" ? (
          <Home onOpenArea={openArea} />
        ) : tab === "checkin" ? (
          <CheckIn onDone={() => setTab("home")} />
        ) : tab === "goals" ? (
          <Goals onOpenArea={openArea} />
        ) : tab === "history" ? (
          <History />
        ) : (
          <Settings onOpenArea={openArea} />
        )}
      </main>
      <BottomNav
        tab={tab}
        onChange={(t) => {
          setAreaId(null);
          setTab(t);
        }}
      />
    </div>
  );
}

export default function App() {
  return (
    <StoreProvider>
      <Shell />
    </StoreProvider>
  );
}
