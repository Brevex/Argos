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

import type { Exported, Gallery, Inventory, ScanRequest, ScanStarted, Summary } from './dto';

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
 * Nothing is asked about privileges here. This window is already running with
 * the ones a raw device needs — it asked for them before it was drawn — and
 * the engine inherits them as an ordinary child.
 */
export function connect(): Promise<void> {
  return invoke('connect');
}

/**
 * Where a folder picker should open.
 *
 * The window runs as an administrator, so its idea of "home" is the
 * administrator's, which is not where anyone keeps anything. Empty when the
 * platform has no answer, and the picker then chooses for itself.
 */
export function invokerHome(): Promise<string> {
  return invoke('invoker_home');
}

/**
 * The view preferences this account last stored, as JSON text.
 *
 * Not `localStorage`: the window runs as an administrator, so a web view store
 * lands in the administrator's profile and belongs to the machine rather than
 * to the person looking at it. This is a file in their own home.
 */
export function preferencesRead(): Promise<string> {
  return invoke('preferences_read');
}

/** Replaces the stored view preferences. */
export function preferencesWrite(text: string): Promise<void> {
  return invoke('preferences_write', { text });
}

/** The media this machine exposes. */
export function devices(): Promise<Inventory> {
  return invoke('devices');
}

/** Starts a scan. Progress arrives on the event channel, never by asking. */
export function scanStart(request: ScanRequest): Promise<ScanStarted> {
  return invoke('scan_start', { request });
}

/**
 * Suspends the running scan at the next chunk boundary.
 *
 * The medium stays open and nothing recovered is discarded; the run carries on
 * from where it stopped. This is the same call `p` makes at the `argos` prompt.
 */
export function scanPause(): Promise<void> {
  return invoke('scan_pause');
}

/** Resumes a paused scan. */
export function scanResume(): Promise<void> {
  return invoke('scan_resume');
}

/**
 * One page of a finished session's artifacts, strongest evidence first.
 *
 * The order and the filter are the engine's. `standing` is passed through as
 * the name the engine gave it; this file does not know what the names mean and
 * must not — which artifact looks like a photograph is a recovery question
 * (`A-SHELL-NO-DOMAIN`).
 */
export function scanGallery(
  session: string,
  offset: number,
  limit: number,
  standing: string | null,
): Promise<Gallery> {
  return invoke('scan_gallery', { session, offset, limit, standing });
}

/**
 * Copies a session's artifacts into `to`, verifying every hash on the way.
 *
 * `standing` is the filter the gallery is showing, passed through as the name
 * the engine gave it. An artifact whose bytes no longer reproduce the digest
 * the scan recorded comes back in `tampered` and is not copied.
 */
export function exportCopy(
  session: string,
  to: string,
  standing: string | null,
): Promise<Exported> {
  return invoke('export_copy', { session, to, standing });
}

/** Stops the running scan, keeping everything recovered so far. */
export function scanCancel(): Promise<void> {
  return invoke('scan_cancel');
}

/** Subscribes to engine notifications. */
export function onEngineMessage(handle: (message: EngineMessage) => void): Promise<UnlistenFn> {
  return listen<EngineMessage>(ENGINE_EVENT, (event) => handle(event.payload));
}
