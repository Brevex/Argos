import { Show } from 'solid-js';
import CircularProgress from './CircularProgress';
import MetricCard from './MetricCard';
import {
  ClockIcon,
  DataIcon,
  ImageIcon,
  PlayIcon,
  SearchIcon,
  StopIcon,
} from './icons';
import type { DeviceInfo, ProgressEvent } from '../lib/bridge';
import type { SessionPhase } from '../lib/recovery';
import {
  formatBytes,
  formatCount,
  formatDuration,
} from '../lib/format';

interface StatusPanelProps {
  phase: SessionPhase;
  progress: ProgressEvent | null;
  device: DeviceInfo | null;
  bytesRecovered: number;
  elapsedMs: number;
  canStart: boolean;
  onStart: () => void;
  onCancel: () => void;
  onReset: () => void;
}

const PHASE_LABEL: Record<SessionPhase, string> = {
  idle: 'Idle',
  starting: 'Starting',
  running: 'Analyzing',
  cancelling: 'Cancelling',
  completed: 'Completed',
  failed: 'Failed',
};

const RATIO_THRESHOLD = 0.005;
const ELAPSED_THRESHOLD_MS = 1500;

export default function StatusPanel(props: StatusPanelProps) {
  const totalBytes = () => props.device?.size_bytes ?? 0;

  const ratio = (): number | null => {
    if (props.phase === 'completed') return 1;
    if (props.phase === 'idle') return 0;
    if (props.phase === 'starting') return null;
    const total = totalBytes();
    const scanned = props.progress?.bytes_scanned ?? 0;
    if (total <= 0) return null;
    return scanned / total;
  };

  const estimatedTotalMs = (): number | null => {
    const r = ratio();
    if (r === null || r < RATIO_THRESHOLD) return null;
    if (props.elapsedMs < ELAPSED_THRESHOLD_MS) return null;
    return props.elapsedMs / r;
  };

  const remainingMs = (): number | null => {
    const total = estimatedTotalMs();
    if (total === null) return null;
    return Math.max(0, total - props.elapsedMs);
  };

  const formatOptional = (value: number | null): string =>
    value === null ? '—' : formatDuration(value);

  const isRunning = () => props.phase === 'running';
  const isBusy = () =>
    props.phase === 'starting' ||
    props.phase === 'running' ||
    props.phase === 'cancelling';

  const primaryLabel = (): string => {
    if (props.phase === 'starting') return 'Starting…';
    if (props.phase === 'running') return 'Cancel';
    if (props.phase === 'cancelling') return 'Cancelling…';
    if (props.phase === 'completed' || props.phase === 'failed') {
      return 'Start new session';
    }
    return 'Start recovery';
  };

  const handlePrimary = () => {
    if (isRunning()) props.onCancel();
    else props.onStart();
  };

  return (
    <>
      <header class="section-title">
        <h2>Recovery Status</h2>
      </header>

      <div class="status-hero">
        <CircularProgress ratio={ratio()} size={120} stroke={8} />
        <div class="status-hero-text">
          <span class="hero-label">{PHASE_LABEL[props.phase]}</span>
          <span class="hero-device">{props.device?.path ?? '—'}</span>
          <div class="hero-times">
            <div class="hero-time">
              <span class="hero-time-label">Elapsed</span>
              <span class="hero-time-value">
                {formatDuration(props.elapsedMs)}
              </span>
            </div>
            <div class="hero-time">
              <span class="hero-time-label">Estimated</span>
              <span class="hero-time-value">
                {formatOptional(estimatedTotalMs())}
              </span>
            </div>
          </div>
        </div>
      </div>

      <div class="metric-grid">
        <MetricCard
          icon={<ImageIcon />}
          tone="green"
          value={formatCount(props.progress?.artifacts_recovered ?? 0)}
          label="Images recovered"
        />
        <MetricCard
          icon={<ClockIcon />}
          tone="purple"
          value={formatOptional(remainingMs())}
          label="Time remaining"
        />
        <MetricCard
          icon={<SearchIcon />}
          tone="cyan"
          value={formatCount(props.progress?.candidates_found ?? 0)}
          label="Files analyzed"
        />
        <MetricCard
          icon={<DataIcon />}
          tone="orange"
          value={formatBytes(props.bytesRecovered)}
          label="Data found"
        />
      </div>

      <div class="status-actions">
        <button
          type="button"
          class={`btn primary status-action-primary ${isRunning() ? 'danger' : ''}`}
          onClick={handlePrimary}
          disabled={
            (props.phase === 'idle' && !props.canStart) ||
            props.phase === 'starting' ||
            props.phase === 'cancelling'
          }
        >
          <span class="btn-icon">
            {isRunning() ? <StopIcon /> : <PlayIcon />}
          </span>
          {primaryLabel()}
        </button>
        <Show when={!isBusy() && (props.phase === 'completed' || props.phase === 'failed')}>
          <button
            type="button"
            class="btn square"
            onClick={props.onReset}
            aria-label="Clear results"
          >
            <StopIcon />
          </button>
        </Show>
      </div>
    </>
  );
}
