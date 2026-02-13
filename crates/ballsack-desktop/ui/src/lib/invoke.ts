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

export interface AudioDeviceInfo {
  id: string;
  label: string;
  isDefault: boolean;
}

export async function startCall(
  roomId: number,
  displayName: string,
  inputDevice?: string | null
): Promise<StartCallResult> {
  return invoke<StartCallResult>("start_call", {
    roomId,
    displayName,
    inputDevice: inputDevice ?? null,
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

export async function setMuted(muted: boolean): Promise<boolean> {
  return invoke<boolean>("set_muted", { muted });
}

export async function listAudioDevices(): Promise<AudioDeviceInfo[]> {
  return invoke<AudioDeviceInfo[]>("list_audio_devices");
}
