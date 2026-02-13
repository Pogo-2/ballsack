import { useState } from "react";
import CallControls from "./components/CallControls";
import PeerList from "./components/PeerList";
import StatsPanel from "./components/StatsPanel";

export default function App() {
  const [inCall, setInCall] = useState(false);

  return (
    <div style={styles.container}>
      <header style={styles.header}>
        <h1 style={styles.title}>ballsack</h1>
        <span style={styles.subtitle}>E2EE Group Calls</span>
      </header>

      <main style={styles.main}>
        <CallControls inCall={inCall} onCallStateChange={setInCall} />
        {inCall && (
          <>
            <PeerList />
            <StatsPanel />
          </>
        )}
      </main>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    minHeight: "100vh",
    display: "flex",
    flexDirection: "column",
    padding: "24px",
    gap: "24px",
  },
  header: {
    display: "flex",
    alignItems: "baseline",
    gap: "12px",
  },
  title: {
    fontSize: "28px",
    fontWeight: 700,
    color: "#ffffff",
  },
  subtitle: {
    fontSize: "14px",
    color: "#888",
  },
  main: {
    display: "flex",
    flexDirection: "column",
    gap: "20px",
    flex: 1,
  },
};
