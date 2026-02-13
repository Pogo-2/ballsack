import { useCallback, useState } from "react";
import { useTauriEvent, StatsPayload } from "../hooks/useTauriEvents";

export default function StatsPanel() {
  const [stats, setStats] = useState<StatsPayload>({
    rttMs: 0,
    loss: 0,
    jitterMs: 0,
  });

  useTauriEvent<StatsPayload>(
    "stats-update",
    useCallback((payload) => {
      setStats(payload);
    }, [])
  );

  return (
    <div style={styles.card}>
      <h2 style={styles.heading}>Call Quality</h2>
      <div style={styles.grid}>
        <StatItem label="RTT" value={`${stats.rttMs} ms`} />
        <StatItem
          label="Packet Loss"
          value={`${(stats.loss * 100).toFixed(1)}%`}
        />
        <StatItem label="Jitter" value={`${stats.jitterMs.toFixed(1)} ms`} />
      </div>
    </div>
  );
}

function StatItem({ label, value }: { label: string; value: string }) {
  return (
    <div style={styles.stat}>
      <span style={styles.statLabel}>{label}</span>
      <span style={styles.statValue}>{value}</span>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  card: {
    background: "#1a1a1a",
    borderRadius: "12px",
    padding: "24px",
    border: "1px solid #2a2a2a",
  },
  heading: {
    fontSize: "16px",
    fontWeight: 600,
    marginBottom: "12px",
    color: "#fff",
  },
  grid: {
    display: "grid",
    gridTemplateColumns: "repeat(3, 1fr)",
    gap: "12px",
  },
  stat: {
    display: "flex",
    flexDirection: "column" as const,
    gap: "4px",
    padding: "12px",
    background: "#0f0f0f",
    borderRadius: "8px",
  },
  statLabel: {
    fontSize: "11px",
    color: "#888",
    textTransform: "uppercase" as const,
    letterSpacing: "0.5px",
  },
  statValue: {
    fontSize: "20px",
    fontWeight: 700,
    color: "#fff",
    fontFamily: "monospace",
  },
};
