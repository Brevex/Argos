import { Show } from 'solid-js';
import type { SessionPhase } from '../lib/recovery';

interface BottomBannerProps {
  phase: SessionPhase;
  canStart: boolean;
  onStart: () => void;
  onCancel: () => void;
  onReset: () => void;
}

const TIPS: Record<SessionPhase, { tone: 'info' | 'warn' | 'ok' | 'error'; title: string; body: string }> = {
  idle: {
    tone: 'info',
    title: 'Pronto para iniciar.',
    body: 'Selecione um dispositivo e a pasta de destino para começar a recuperação.',
  },
  starting: {
    tone: 'info',
    title: 'Iniciando sessão.',
    body: 'Validando escopo e abrindo o dispositivo em modo somente leitura.',
  },
  running: {
    tone: 'info',
    title: 'Dica:',
    body:
      'A recuperação pode levar de alguns minutos a várias horas, dependendo do tamanho do dispositivo. Você pode continuar usando o computador normalmente durante o processo.',
  },
  cancelling: {
    tone: 'warn',
    title: 'Cancelando.',
    body: 'Encerrando a sessão e finalizando a trilha de auditoria.',
  },
  completed: {
    tone: 'ok',
    title: 'Sessão concluída.',
    body: 'Os artefatos validados foram gravados na pasta de destino, com hash registrado no audit log.',
  },
  failed: {
    tone: 'error',
    title: 'Sessão interrompida.',
    body: 'Verifique o erro acima e tente novamente. Nada foi modificado no dispositivo de origem.',
  },
};

export default function BottomBanner(props: BottomBannerProps) {
  const tip = () => TIPS[props.phase];

  return (
    <section class={`bottom-banner ${tip().tone}`}>
      <span class="bottom-banner-icon" aria-hidden="true">
        i
      </span>
      <div class="bottom-banner-text">
        <strong>{tip().title}</strong>
        <span>{tip().body}</span>
      </div>
      <div class="bottom-banner-actions">
        <Show when={props.phase === 'idle'}>
          <button
            type="button"
            class="btn primary"
            onClick={props.onStart}
            disabled={!props.canStart}
          >
            Iniciar recuperação
          </button>
        </Show>
        <Show when={props.phase === 'running' || props.phase === 'starting'}>
          <button
            type="button"
            class="btn danger"
            onClick={props.onCancel}
            disabled={props.phase === 'starting'}
          >
            Cancelar
          </button>
        </Show>
        <Show when={props.phase === 'cancelling'}>
          <button type="button" class="btn danger" disabled>
            Cancelando…
          </button>
        </Show>
        <Show when={props.phase === 'completed' || props.phase === 'failed'}>
          <button type="button" class="btn primary" onClick={props.onReset}>
            Nova sessão
          </button>
        </Show>
      </div>
    </section>
  );
}
