import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type DeviceClass = 'hdd' | 'ssd' | 'unknown';

export interface DeviceInfo {
  name: string;
  path: string;
  size_bytes: number;
  class: DeviceClass;
  removable: boolean;
  model: string | null;
}

export interface ProgressEvent {
  session_id: number;
  bytes_scanned: number;
  candidates_found: number;
  artifacts_recovered: number;
}

export interface ArtifactEvent {
  session_id: number;
  offset: number;
  length: number;
  format: string;
  score: number;
}

export type SessionCompletionStatus = 'ok' | 'cancelled' | 'failed';

export interface SessionCompletedEvent {
  session_id: number;
  status: SessionCompletionStatus;
  error: BridgeError | null;
}

export type BridgeErrorKind =
  | 'io'
  | 'allocation'
  | 'unsupported'
  | 'pattern_build'
  | 'validation'
  | 'audit_serialization'
  | 'denied';

export interface BridgeError {
  kind: BridgeErrorKind;
  detail: string;
}

export const listDevices = (): Promise<DeviceInfo[]> => invoke('list_devices');

export interface StartResponse {
  session_id: number;
  warning?: string;
}

export const startRecovery = (
  source: string,
  output: string,
): Promise<StartResponse> =>
  invoke('start_recovery', { request: { source, output } });

export const cancelRecovery = (sessionId: number): Promise<void> =>
  invoke('cancel_recovery', { request: { session_id: sessionId } });

export const onProgress = (
  handler: (event: ProgressEvent) => void,
): Promise<UnlistenFn> =>
  listen<ProgressEvent>('progress', (event) => handler(event.payload));

export const onArtifact = (
  handler: (event: ArtifactEvent) => void,
): Promise<UnlistenFn> =>
  listen<ArtifactEvent>('artifact', (event) => handler(event.payload));

export const onSessionCompleted = (
  handler: (event: SessionCompletedEvent) => void,
): Promise<UnlistenFn> =>
  listen<SessionCompletedEvent>('session_completed', (event) =>
    handler(event.payload),
  );

export const isBridgeError = (value: unknown): value is BridgeError =>
  typeof value === 'object' &&
  value !== null &&
  'kind' in value &&
  'detail' in value;

const ERROR_MESSAGES: Record<BridgeErrorKind, string> = {
  io: 'Failed to access the device. Argos needs elevated privileges to read raw block devices — run the bundled launcher (it requests admin/UAC by default), or apply the platform-specific capabilities manually.',
  allocation: 'Insufficient memory to align device buffers.',
  unsupported: 'This platform is not supported for the requested operation.',
  pattern_build: 'Failed to build the search patterns used for carving.',
  validation: 'Recovered bytes failed structural validation and were discarded.',
  audit_serialization: 'Failed to serialize the audit trail.',
  denied: 'The selected path is outside the allowed scope or the session is no longer valid.',
};

export const friendlyError = (value: unknown): string => {
  if (isBridgeError(value)) return ERROR_MESSAGES[value.kind];
  return 'Unexpected error.';
};
