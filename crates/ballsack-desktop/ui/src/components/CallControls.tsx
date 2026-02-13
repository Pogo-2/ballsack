import { useState } from "react";
import { startCall, endCall } from "../lib/invoke";

interface Props {
  inCall: boolean;
  onCallStateChange: (inCall: boolean) => void;
}

export default function CallControls({ inCall, onCallStateChange }: Props) {
  const [roomId, setRoomId] = useState("1");
  const [displayName, setDisplayName] = useState("User");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const handleJoin = async () => {
    setError(null);
    setLoading(true);
    try {
      await startCall(parseInt(roomId, 10), displayName);
      onCallStateChange(true);
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const handleLeave = async () => {
    try {
      await endCall();
    } catch (_) {}
    onCallStateChange(false);
  };

  if (inCall) {
    return (
      <div style={styles.card}>
        <div style={styles.inCallRow}>
          <span style={styles.statusDot} />
          <span>
            In call &mdash; Room {roomId} as <strong>{displayName}</strong>
          </span>
          <button style={styles.leaveBtn} onClick={handleLeave}>
            Leave
          </button>
        </div>
      </div>
    );
  }

  return (
    <div style={styles.card}>
      <h2 style={styles.heading}>Join a Room</h2>
      <div style={styles.form}>
        <label style={styles.label}>
          Room ID
          <input
            style={styles.input}
            type="number"
            value={roomId}
            onChange={(e) => setRoomId(e.target.value)}
          />
        </label>
        <label style={styles.label}>
          Display Name
          <input
            style={styles.input}
            type="text"
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
          />
        </label>
        <button
          style={styles.joinBtn}
          onClick={handleJoin}
          disabled={loading}
        >
          {loading ? "Connecting..." : "Join"}
        </button>
      </div>
      {error && <p style={styles.error}>{error}</p>}
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
    fontSize: "18px",
    fontWeight: 600,
    marginBottom: "16px",
    color: "#fff",
  },
  form: {
    display: "flex",
    gap: "12px",
    alignItems: "flex-end",
    flexWrap: "wrap" as const,
  },
  label: {
    display: "flex",
    flexDirection: "column" as const,
    gap: "4px",
    fontSize: "13px",
    color: "#aaa",
  },
  input: {
    background: "#0f0f0f",
    border: "1px solid #333",
    borderRadius: "6px",
    padding: "8px 12px",
    color: "#fff",
    fontSize: "14px",
    width: "160px",
  },
  joinBtn: {
    background: "#2563eb",
    color: "#fff",
    border: "none",
    borderRadius: "6px",
    padding: "8px 24px",
    fontSize: "14px",
    fontWeight: 600,
    cursor: "pointer",
    height: "36px",
  },
  inCallRow: {
    display: "flex",
    alignItems: "center",
    gap: "10px",
    fontSize: "14px",
  },
  statusDot: {
    width: "10px",
    height: "10px",
    borderRadius: "50%",
    background: "#22c55e",
    flexShrink: 0,
  },
  leaveBtn: {
    marginLeft: "auto",
    background: "#dc2626",
    color: "#fff",
    border: "none",
    borderRadius: "6px",
    padding: "6px 16px",
    fontSize: "13px",
    fontWeight: 600,
    cursor: "pointer",
  },
  error: {
    marginTop: "12px",
    color: "#f87171",
    fontSize: "13px",
  },
};
