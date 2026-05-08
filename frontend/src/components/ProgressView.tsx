import { Show } from 'solid-js';
import CircularProgress from './CircularProgress';
import type { DeviceInfo, ProgressEvent } from '../lib/bridge';
import type { SessionPhase } from '../lib/recovery';
import {
  formatBytes,
  formatCount,
  formatDuration,
} from '../lib/format';

interface ProgressViewProps {
  phase: SessionPhase;
  progress: ProgressEvent | null;
  device: DeviceInfo | null;
  bytesRecovered: number;
  elapsedMs: number;
  errorMessage: string | null;
}

const HEADLINE: Record<SessionPhase, string> = {
  idle: 'Aguardando seleção',
  starting: 'Iniciando dispositivo…',
  running: 'Escaneando dispositivo…',
  cancelling: 'Cancelando sessão…',
  completed: 'Sessão concluída',
  failed: 'Sessão interrompida',
};

export default function ProgressView(props: ProgressViewProps) {
  const totalBytes = () => props.device?.size_bytes ?? 0;
  const ratio = (): number | null => {
    const total = totalBytes();
    const scanned = props.progress?.bytes_scanned ?? 0;
    if (props.phase === 'idle') return 0;
    if (props.phase === 'completed') return 1;
    if (props.phase === 'starting') return null;
    if (total <= 0) return null;
    return scanned / total;
  };

  const subtitle = () => {
    if (props.phase === 'idle') return 'Selecione um dispositivo para começar.';
    if (props.device) return props.device.path;
    return '—';
  };

  return (
    <>
      <header class="section-title">
        <h2>Progresso da recuperação</h2>
      </header>

      <Show when={props.errorMessage}>
        <div class="error-banner">
          <span class="error-banner-dot" />
          {props.errorMessage}
        </div>
      </Show>

      <div class="progress-content">
        <div class="progress-hero">
          <CircularProgress ratio={ratio()} />
          <div class="progress-hero-text">
            <span class="progress-hero-title">{HEADLINE[props.phase]}</span>
            <span class="progress-hero-subtitle">{subtitle()}</span>
          </div>
        </div>

        <div class="metric-grid">
        <div class="metric">
          <span class="metric-value">
            {formatBytes(props.progress?.bytes_scanned ?? 0)}
          </span>
          <span class="metric-label">Escaneados</span>
        </div>
        <div class="metric">
          <span class="metric-value">{formatBytes(props.bytesRecovered)}</span>
          <span class="metric-label">Recuperados</span>
        </div>
        <div class="metric">
          <span class="metric-value">
            {formatCount(props.progress?.artifacts_recovered ?? 0)}
          </span>
          <span class="metric-label">Arquivos</span>
        </div>
        <div class="metric">
          <span class="metric-value mono">
            {formatDuration(props.elapsedMs)}
          </span>
          <span class="metric-label">Tempo decorrido</span>
        </div>
        </div>
      </div>

      <footer class="progress-footer">
        <span class="progress-footer-text">
          <Show
            when={props.phase === 'running' || props.phase === 'cancelling'}
            fallback={
              <Show
                when={props.phase === 'completed'}
                fallback="Aguardando início da sessão."
              >
                Trilha de auditoria finalizada.
              </Show>
            }
          >
            Buscando por imagens e metadados…
          </Show>
        </span>
        <span class="progress-footer-stat">
          Encontrados: {formatCount(props.progress?.candidates_found ?? 0)}
        </span>
      </footer>
    </>
  );
}
