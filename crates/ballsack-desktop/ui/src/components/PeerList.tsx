import { useCallback, useEffect, useState } from "react";
import { getPeers, PeerEntry } from "../lib/invoke";
import {
  useTauriEvent,
  PeerJoinedPayload,
  PeerLeftPayload,
} from "../hooks/useTauriEvents";

export default function PeerList() {
  const [peers, setPeers] = useState<PeerEntry[]>([]);

  // Fetch initial peer list
  useEffect(() => {
    getPeers().then(setPeers).catch(console.error);
  }, []);

  // Listen for peer-joined events
  useTauriEvent<PeerJoinedPayload>(
    "peer-joined",
    useCallback((payload) => {
      setPeers((prev) => {
        if (prev.some((p) => p.peerId === payload.peerId)) return prev;
        return [...prev, { peerId: payload.peerId, displayName: payload.displayName }];
      });
    }, [])
  );

  // Listen for peer-left events
  useTauriEvent<PeerLeftPayload>(
    "peer-left",
    useCallback((payload) => {
      setPeers((prev) => prev.filter((p) => p.peerId !== payload.peerId));
    }, [])
  );

  return (
    <div style={styles.card}>
      <h2 style={styles.heading}>
        Participants ({peers.length})
      </h2>
      {peers.length === 0 ? (
        <p style={styles.empty}>No other participants yet.</p>
      ) : (
        <ul style={styles.list}>
          {peers.map((peer) => (
            <li key={peer.peerId} style={styles.item}>
              <span style={styles.avatar}>
                {peer.displayName.charAt(0).toUpperCase()}
              </span>
              <span>{peer.displayName}</span>
              <span style={styles.id}>#{peer.peerId}</span>
            </li>
          ))}
        </ul>
      )}
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
  empty: {
    fontSize: "13px",
    color: "#666",
  },
  list: {
    listStyle: "none",
    display: "flex",
    flexDirection: "column" as const,
    gap: "8px",
  },
  item: {
    display: "flex",
    alignItems: "center",
    gap: "10px",
    fontSize: "14px",
    padding: "8px 12px",
    background: "#0f0f0f",
    borderRadius: "8px",
  },
  avatar: {
    width: "28px",
    height: "28px",
    borderRadius: "50%",
    background: "#2563eb",
    color: "#fff",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    fontSize: "13px",
    fontWeight: 700,
    flexShrink: 0,
  },
  id: {
    marginLeft: "auto",
    fontSize: "11px",
    color: "#555",
    fontFamily: "monospace",
  },
};
