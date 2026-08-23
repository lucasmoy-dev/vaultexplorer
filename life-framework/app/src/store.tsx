// ---------------------------------------------------------------------------
// Central app state: one DB document behind a reducer, auto-persisted to the
// backend (debounced). Components read via useDb() and mutate via useStore().
// ---------------------------------------------------------------------------
import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { Area, DB, Question, Snapshot, Subcategory } from "./types";
import { SCHEMA_VERSION } from "./types";
import { seedDb, uid } from "./seed";
import * as api from "./api";
import { todayISO } from "./planner";

type Action =
  | { type: "REPLACE"; db: DB }
  | { type: "SET_CURRENT"; questionId: string; value: number }
  | { type: "ADD_AREA"; area: Area }
  | { type: "UPDATE_AREA"; areaId: string; patch: Partial<Area> }
  | { type: "DELETE_AREA"; areaId: string }
  | { type: "REORDER_AREA"; areaId: string; dir: -1 | 1 }
  | { type: "ADD_SUB"; areaId: string; sub: Subcategory }
  | { type: "UPDATE_SUB"; areaId: string; subId: string; patch: Partial<Subcategory> }
  | { type: "DELETE_SUB"; areaId: string; subId: string }
  | { type: "ADD_QUESTION"; areaId: string; subId: string; question: Question }
  | { type: "UPDATE_QUESTION"; areaId: string; subId: string; questionId: string; patch: Partial<Question> }
  | { type: "DELETE_QUESTION"; areaId: string; subId: string; questionId: string }
  | { type: "CHECK_IN"; note?: string }
  | { type: "UPDATE_SETTINGS"; patch: Partial<DB["settings"]> }
  | { type: "RESET_SEED" };

function mapArea(db: DB, areaId: string, fn: (a: Area) => Area): DB {
  return { ...db, areas: db.areas.map((a) => (a.id === areaId ? fn(a) : a)) };
}

function mapSub(db: DB, areaId: string, subId: string, fn: (s: Subcategory) => Subcategory): DB {
  return mapArea(db, areaId, (a) => ({
    ...a,
    subs: a.subs.map((s) => (s.id === subId ? fn(s) : s)),
  }));
}

function reducer(db: DB, action: Action): DB {
  switch (action.type) {
    case "REPLACE":
      return action.db;

    case "SET_CURRENT":
      return { ...db, current: { ...db.current, [action.questionId]: action.value } };

    case "ADD_AREA":
      return { ...db, areas: [...db.areas, action.area] };

    case "UPDATE_AREA":
      return mapArea(db, action.areaId, (a) => ({ ...a, ...action.patch }));

    case "DELETE_AREA": {
      const area = db.areas.find((a) => a.id === action.areaId);
      const current = { ...db.current };
      area?.subs.forEach((s) => s.questions.forEach((q) => delete current[q.id]));
      return { ...db, areas: db.areas.filter((a) => a.id !== action.areaId), current };
    }

    case "REORDER_AREA": {
      const sorted = [...db.areas].sort((a, b) => a.order - b.order);
      const i = sorted.findIndex((a) => a.id === action.areaId);
      const j = i + action.dir;
      if (i < 0 || j < 0 || j >= sorted.length) return db;
      [sorted[i], sorted[j]] = [sorted[j], sorted[i]];
      const orderById = new Map(sorted.map((a, idx) => [a.id, idx]));
      return { ...db, areas: db.areas.map((a) => ({ ...a, order: orderById.get(a.id)! })) };
    }

    case "ADD_SUB":
      return mapArea(db, action.areaId, (a) => ({ ...a, subs: [...a.subs, action.sub] }));

    case "UPDATE_SUB":
      return mapSub(db, action.areaId, action.subId, (s) => ({ ...s, ...action.patch }));

    case "DELETE_SUB": {
      const area = db.areas.find((a) => a.id === action.areaId);
      const sub = area?.subs.find((s) => s.id === action.subId);
      const current = { ...db.current };
      sub?.questions.forEach((q) => delete current[q.id]);
      return mapArea({ ...db, current }, action.areaId, (a) => ({
        ...a,
        subs: a.subs.filter((s) => s.id !== action.subId),
      }));
    }

    case "ADD_QUESTION":
      return mapSub(db, action.areaId, action.subId, (s) => ({
        ...s,
        questions: [...s.questions, action.question],
      }));

    case "UPDATE_QUESTION":
      return mapSub(db, action.areaId, action.subId, (s) => ({
        ...s,
        questions: s.questions.map((q) => (q.id === action.questionId ? { ...q, ...action.patch } : q)),
      }));

    case "DELETE_QUESTION": {
      const current = { ...db.current };
      delete current[action.questionId];
      return mapSub({ ...db, current }, action.areaId, action.subId, (s) => ({
        ...s,
        questions: s.questions.filter((q) => q.id !== action.questionId),
      }));
    }

    case "CHECK_IN": {
      const snap: Snapshot = {
        id: uid("snap"),
        dateISO: todayISO(),
        answers: { ...db.current },
        note: action.note,
      };
      // Replace a same-day snapshot rather than piling duplicates.
      const rest = db.snapshots.filter((s) => s.dateISO !== snap.dateISO);
      return { ...db, snapshots: [...rest, snap].sort((a, b) => a.dateISO.localeCompare(b.dateISO)) };
    }

    case "UPDATE_SETTINGS":
      return { ...db, settings: { ...db.settings, ...action.patch } };

    case "RESET_SEED":
      return seedDb();

    default:
      return db;
  }
}

/** Coerce a loaded document into the current schema shape. */
function hydrate(raw: string | null): DB {
  if (!raw) return seedDb();
  try {
    const parsed = JSON.parse(raw) as Partial<DB>;
    if (!parsed || !Array.isArray(parsed.areas)) return seedDb();
    // Migrate legacy flat areas ({ questions: [...] }) into a single
    // "General" subcategory so older exports keep loading.
    const areas = (parsed.areas as unknown[]).map((raw) => {
      const a = raw as Area & { questions?: Question[] };
      if (Array.isArray(a.subs)) return a;
      const legacy = Array.isArray(a.questions) ? a.questions : [];
      return {
        ...a,
        subs: [{ id: uid("sub"), name: "General", weight: 1, questions: legacy }],
      } as Area;
    });
    return {
      schemaVersion: parsed.schemaVersion ?? SCHEMA_VERSION,
      settings: { ...seedDb().settings, ...(parsed.settings ?? {}) },
      areas,
      current: parsed.current ?? {},
      snapshots: parsed.snapshots ?? [],
    };
  } catch {
    return seedDb();
  }
}

interface StoreCtx {
  db: DB;
  dispatch: (a: Action) => void;
  /** Replace the whole DB (used by import). */
  replaceDb: (db: DB) => void;
}

const Ctx = createContext<StoreCtx | null>(null);

export function StoreProvider({ children }: { children: ReactNode }) {
  const [db, dispatch] = useReducer(reducer, null, seedDb);
  const [ready, setReady] = useState(false);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Load once on mount.
  useEffect(() => {
    let alive = true;
    api.loadDb().then((raw) => {
      if (!alive) return;
      dispatch({ type: "REPLACE", db: hydrate(raw) });
      setReady(true);
    });
    return () => {
      alive = false;
    };
  }, []);

  // Persist (debounced) after the initial load has completed.
  useEffect(() => {
    if (!ready) return;
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      api.saveDb(JSON.stringify(db)).catch(() => {});
    }, 400);
    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
    };
  }, [db, ready]);

  // Apply theme to the root element.
  useEffect(() => {
    document.documentElement.dataset.theme = db.settings.theme;
  }, [db.settings.theme]);

  const value = useMemo<StoreCtx>(
    () => ({ db, dispatch, replaceDb: (next) => dispatch({ type: "REPLACE", db: next }) }),
    [db],
  );

  if (!ready) return null;
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useStore(): StoreCtx {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useStore must be used within StoreProvider");
  return ctx;
}

export const useDb = () => useStore().db;
