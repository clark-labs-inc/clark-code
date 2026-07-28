import { useCallback, useEffect, useState } from "react";
import "@xyflow/react/dist/style.css";
import { NewDiscoveryModal, Sidebar, Toast } from "./components/Layout";
import { Benchmarks } from "./pages/Benchmarks";
import { Capsules } from "./pages/Capsules";
import { Coverage } from "./pages/Coverage";
import { Discoveries } from "./pages/Discoveries";
import { Evidence } from "./pages/Evidence";
import { Governance } from "./pages/Governance";
import { Machines } from "./pages/Machines";
import { Overview } from "./pages/Overview";
import { Simulations } from "./pages/Simulations";
import { Sources } from "./pages/Sources";
import { SystemGraph } from "./pages/SystemGraph";

const validRoutes = new Set([
  "overview",
  "discoveries",
  "coverage",
  "graph",
  "evidence",
  "simulations",
  "machines",
  "capsules",
  "benchmarks",
  "governance",
  "sources",
]);

function routeFromHash() {
  const route = window.location.hash.replace(/^#\/?/, "");
  return validRoutes.has(route) ? route : "overview";
}

export function App() {
  const [route, setRoute] = useState(routeFromHash);
  const [newDiscoveryOpen, setNewDiscoveryOpen] = useState(false);
  const [runActive, setRunActive] = useState(true);
  const [notice, setNotice] = useState("");

  useEffect(() => {
    const onHashChange = () => setRoute(routeFromHash());
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  useEffect(() => {
    window.scrollTo({ top: 0, behavior: "instant" });
  }, [route]);

  const navigate = useCallback((nextRoute) => {
    if (!validRoutes.has(nextRoute)) return;
    window.location.hash = `/${nextRoute}`;
    setRoute(nextRoute);
  }, []);

  const toggleRun = () => {
    setRunActive((active) => {
      setNotice(active
        ? "Discovery run paused safely. Every active lease remains resumable."
        : "Discovery run resumed across 26 enrolled machines.");
      return !active;
    });
  };

  const sharedProps = {
    onNavigate: navigate,
    onNewDiscovery: () => setNewDiscoveryOpen(true),
    onNotice: setNotice,
  };

  const content = {
    overview: <Overview {...sharedProps} />,
    discoveries: <Discoveries {...sharedProps} running={runActive} onToggleRun={toggleRun} />,
    coverage: <Coverage {...sharedProps} />,
    graph: <SystemGraph {...sharedProps} />,
    evidence: <Evidence {...sharedProps} />,
    simulations: <Simulations {...sharedProps} />,
    machines: <Machines {...sharedProps} />,
    capsules: <Capsules {...sharedProps} />,
    benchmarks: <Benchmarks {...sharedProps} />,
    governance: <Governance {...sharedProps} />,
    sources: <Sources {...sharedProps} />,
  }[route];

  if (route === "overview") {
    return (
      <div className="atlas-app-shell">
        <main className="app-content">{content}</main>
        <Toast message={notice} onDismiss={() => setNotice("")} />
      </div>
    );
  }

  return (
    <div className="app-shell">
      <Sidebar active={route} onNavigate={navigate} onNewDiscovery={() => setNewDiscoveryOpen(true)} />
      <main className="app-content">{content}</main>
      <NewDiscoveryModal
        open={newDiscoveryOpen}
        onClose={() => setNewDiscoveryOpen(false)}
        onCreated={(name) => {
          setNewDiscoveryOpen(false);
          setRunActive(true);
          navigate("discoveries");
          setNotice(`${name} started with a signed read-only charter.`);
        }}
      />
      <Toast message={notice} onDismiss={() => setNotice("")} />
    </div>
  );
}
