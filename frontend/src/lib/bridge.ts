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

export const startRecovery = (
  source: string,
  output: string,
): Promise<{ session_id: number }> =>
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
  io: 'Falha ao acessar o dispositivo. Em Linux, abrir /dev/* sem ser root exige as capabilities cap_dac_read_search, cap_sys_rawio e cap_fowner — aplique setcap cap_dac_read_search,cap_sys_rawio,cap_fowner+ep no binário, ou rode com sudo.',
  allocation: 'Memória insuficiente para alinhar buffers do dispositivo.',
  unsupported: 'Plataforma não suportada para esta operação.',
  pattern_build: 'Falha ao construir padrões de busca.',
  validation: 'Os bytes recuperados falharam na validação estrutural.',
  audit_serialization: 'Falha ao serializar a trilha de auditoria.',
  denied: 'Caminho fora do escopo permitido ou sessão inválida.',
};

export const friendlyError = (value: unknown): string => {
  if (isBridgeError(value)) return ERROR_MESSAGES[value.kind];
  return 'Erro inesperado.';
};
