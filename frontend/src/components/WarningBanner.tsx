import { createSignal, Show } from 'solid-js';

export default function WarningBanner(props: { message: string | null }) {
  const [dismissed, setDismissed] = createSignal(false);

  return (
    <Show when={props.message && !dismissed()}>
      <div class="warning-banner">
        <span class="warning-banner-text">{props.message}</span>
        <button
          class="warning-banner-close"
          type="button"
          onClick={() => setDismissed(true)}
          aria-label="Dismiss warning"
        >
          ×
        </button>
      </div>
    </Show>
  );
}
