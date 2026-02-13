import { useEffect, useState } from "react";
import {
  startCall,
  endCall,
  setMuted,
  listAudioDevices,
  type AudioDeviceInfo,
} from "../lib/invoke";

interface Props {
  inCall: boolean;
  onCallStateChange: (inCall: boolean) => void;
}

export default function CallControls({ inCall, onCallStateChange }: Props) {
  const [roomId, setRoomId] = useState("1");
  const [displayName, setDisplayName] = useState("User");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [muted, setMutedState] = useState(false);

  // Audio device picker — "" means "let the OS pick the default"
  const [devices, setDevices] = useState<AudioDeviceInfo[]>([]);
  const [selectedDevice, setSelectedDevice] = useState<string>("");

  // Fetch devices on mount
  useEffect(() => {
    listAudioDevices()
      .then((devs) => {
        setDevices(devs);
        // Default to "" (system default) so the user doesn't have to touch it
      })
      .catch(() => {});
  }, []);

  const handleRefreshDevices = async () => {
    try {
      const devs = await listAudioDevices();
      setDevices(devs);
    } catch (_) {}
  };

  const handleJoin = async () => {
    setError(null);
    setLoading(true);
    try {
      const inputDevice = selectedDevice || null;
      await startCall(parseInt(roomId, 10), displayName, inputDevice);
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
    setMutedState(false);
    onCallStateChange(false);
  };

  const handleToggleMute = async () => {
    try {
      const newMuted = await setMuted(!muted);
      setMutedState(newMuted);
    } catch (_) {}
  };

  if (inCall) {
    return (
      <div style={styles.card}>
        <div style={styles.inCallRow}>
          <span style={styles.statusDot} />
          <span>
            In call &mdash; Room {roomId} as <strong>{displayName}</strong>
          </span>
          <button
            style={muted ? styles.muteBtn_active : styles.muteBtn}
            onClick={handleToggleMute}
          >
            {muted ? "Unmute" : "Mute"}
          </button>
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
        <label style={styles.label}>
          Microphone
          <div style={styles.deviceRow}>
            <select
              style={styles.select}
              value={selectedDevice}
              onChange={(e) => setSelectedDevice(e.target.value)}
            >
              <option value="">System Default</option>
              {devices.map((d) => (
                <option key={d.id} value={d.id}>
                  {d.label}
                  {d.isDefault ? " (default)" : ""}
                </option>
              ))}
            </select>
            <button
              style={styles.refreshBtn}
              onClick={handleRefreshDevices}
              title="Refresh device list"
            >
              &#x21bb;
            </button>
          </div>
          {devices.length <= 1 && (
            <span style={styles.hint}>
              Only {devices.length} mic detected.{" "}
              Enable more in Windows Sound Settings &gt; Recording.
            </span>
          )}
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
  select: {
    background: "#0f0f0f",
    border: "1px solid #333",
    borderRadius: "6px",
    padding: "8px 12px",
    color: "#fff",
    fontSize: "14px",
    flex: 1,
    minWidth: "180px",
    appearance: "auto" as const,
  },
  deviceRow: {
    display: "flex",
    gap: "6px",
    alignItems: "center",
  },
  refreshBtn: {
    background: "#374151",
    color: "#fff",
    border: "none",
    borderRadius: "6px",
    padding: "6px 10px",
    fontSize: "16px",
    cursor: "pointer",
    lineHeight: 1,
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
  muteBtn: {
    marginLeft: "auto",
    background: "#374151",
    color: "#fff",
    border: "none",
    borderRadius: "6px",
    padding: "6px 16px",
    fontSize: "13px",
    fontWeight: 600,
    cursor: "pointer",
  },
  muteBtn_active: {
    marginLeft: "auto",
    background: "#f59e0b",
    color: "#000",
    border: "none",
    borderRadius: "6px",
    padding: "6px 16px",
    fontSize: "13px",
    fontWeight: 600,
    cursor: "pointer",
  },
  leaveBtn: {
    background: "#dc2626",
    color: "#fff",
    border: "none",
    borderRadius: "6px",
    padding: "6px 16px",
    fontSize: "13px",
    fontWeight: 600,
    cursor: "pointer",
  },
  hint: {
    fontSize: "11px",
    color: "#666",
    marginTop: "2px",
  },
  error: {
    marginTop: "12px",
    color: "#f87171",
    fontSize: "13px",
  },
};
