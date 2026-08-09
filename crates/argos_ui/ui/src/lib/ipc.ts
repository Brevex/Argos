/**
 * Typed access to the shell's commands and events.
 *
 * Every function here is a call into Rust that becomes a JSON-RPC call into
 * the engine. Nothing in this file decides anything about a recovery: it moves
 * a request out and a record back.
 *
 * The types come from `dto.ts`, which is **generated** from the Rust
 * definitions in `argos_ipc`. Do not hand-edit that file — one definition of
 * the wire format, two languages.
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import type { Inventory, ScanRequest, ScanStarted, Summary } from './dto';

/** The Tauri event every engine notification arrives on. */
const ENGINE_EVENT = 'argos://engine';

/** One notification from the engine, as it appears on the event channel. */
export type EngineMessage =
  | { method: 'stageBegan'; params: { stage: string; unit: string; total: number } }
  | { method: 'progress'; params: { stage: string; unit: string; done: number; total: number } }
  | { method: 'stageDone'; params: { stage: string; findings: number } }
  | { method: 'stored'; params: { artifacts: number; bytes: number } }
  | { method: 'state'; params: { state: string } }
  | { method: 'unreadable'; params: { regions: number; bytes: number } }
  | { method: 'warning'; params: { text: string } }
  | { method: 'finished'; params: Summary };

/**
 * Connects to the engine process.
 *
 * `elevated` asks the operating system for the privileges a raw device needs.
 * Scanning an image file does not need them and should not ask.
 */
export function connect(elevated: boolean): Promise<void> {
  return invoke('connect', { elevated });
}

/** The media this machine exposes. */
export function devices(): Promise<Inventory> {
  return invoke('devices');
}

/** Starts a scan. Progress arrives on the event channel, never by asking. */
export function scanStart(request: ScanRequest): Promise<ScanStarted> {
  return invoke('scan_start', { request });
}

/** Stops the running scan, keeping everything recovered so far. */
export function scanCancel(): Promise<void> {
  return invoke('scan_cancel');
}

/** Subscribes to engine notifications. */
export function onEngineMessage(handle: (message: EngineMessage) => void): Promise<UnlistenFn> {
  return listen<EngineMessage>(ENGINE_EVENT, (event) => handle(event.payload));
}
