import { Show } from 'solid-js';
import { AlertIcon, CheckIcon, ShieldIcon } from './icons';
import type { SessionPhase } from '../lib/recovery';

interface IntegrityFooterProps {
  phase: SessionPhase;
}

export default function IntegrityFooter(props: IntegrityFooterProps) {
  const failed = () => props.phase === 'failed';

  return (
    <section class="integrity-footer" aria-label="Integrity status">
      <div class="integrity-cell">
        <span class="integrity-icon shield" aria-hidden="true">
          <ShieldIcon />
        </span>
        <div class="integrity-text">
          <strong>Read-only</strong>
          <span>
            Your data is safe. No changes are made to the source device.
          </span>
        </div>
      </div>
      <div class="integrity-cell right">
        <span class={`integrity-status ${failed() ? 'error' : 'ok'}`}>
          <Show when={!failed()} fallback={<AlertIcon />}>
            <CheckIcon />
          </Show>
          <span>
            Integrity check: {failed() ? 'Failed' : 'OK'}
          </span>
        </span>
      </div>
    </section>
  );
}
