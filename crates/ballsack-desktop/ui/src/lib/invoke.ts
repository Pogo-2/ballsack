/**
 * Typed wrappers around Tauri invoke().
 */
import { invoke } from "@tauri-apps/api/core";

export interface StartCallResult {
  roomId: number;
  peerId: number;
}

export interface PeerEntry {
  peerId: number;
  displayName: string;
}

export interface StatsSnapshot {
  rttMs: number;
  loss: number;
  jitterMs: number;
  bitrateIn: number;
  bitrateOut: number;
}

export async function startCall(
  roomId: number,
  displayName: string
): Promise<StartCallResult> {
  return invoke<StartCallResult>("start_call", {
    roomId,
    displayName,
  });
}

export async function endCall(): Promise<void> {
  return invoke<void>("end_call");
}

export async function getPeers(): Promise<PeerEntry[]> {
  return invoke<PeerEntry[]>("get_peers");
}

export async function getStats(): Promise<StatsSnapshot> {
  return invoke<StatsSnapshot>("get_stats");
}
