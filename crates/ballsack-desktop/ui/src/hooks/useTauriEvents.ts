/**
 * React hooks for listening to Tauri events from the Rust backend.
 */
import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

export interface PeerJoinedPayload {
  peerId: number;
  displayName: string;
}

export interface PeerLeftPayload {
  peerId: number;
}

export interface StatsPayload {
  rttMs: number;
  loss: number;
  jitterMs: number;
}

/**
 * Listen to a Tauri event. Automatically unsubscribes on unmount.
 */
export function useTauriEvent<T>(
  event: string,
  handler: (payload: T) => void
) {
  useEffect(() => {
    const unlisten = listen<T>(event, (e) => handler(e.payload));
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [event, handler]);
}
