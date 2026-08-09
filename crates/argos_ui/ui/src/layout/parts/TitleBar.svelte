<script lang="ts">
  /**
   * The window's own title bar.
   *
   * The system decorations are off, so this draws them: the mark on the left,
   * the theme control and the window buttons on the right, and a drag region
   * across everything between.
   */
  import { getCurrentWindow } from '@tauri-apps/api/window';

  let { onSettings }: { onSettings: () => void } = $props();

  const window = getCurrentWindow();
</script>

<header data-tauri-drag-region>
  <div class="mark" data-tauri-drag-region>
    <span class="glyph" aria-hidden="true"></span>
    <span class="name" data-tauri-drag-region>Argos</span>
  </div>

  <div class="controls">
    <button class="gear" onclick={onSettings} aria-label="Theme" title="Theme">
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <path
          d="M8 10.4a2.4 2.4 0 1 0 0-4.8 2.4 2.4 0 0 0 0 4.8Z"
          fill="none"
          stroke="currentColor"
          stroke-width="1.2"
        />
        <path
          d="M13.1 9.6a5.6 5.6 0 0 0 0-3.2l1.3-1-1.4-2.4-1.6.6a5.6 5.6 0 0 0-2.7-1.6L8.4.6H5.6l-.3 1.8a5.6 5.6 0 0 0-2.7 1.6l-1.6-.6L-.4 5.8l1.3 1a5.6 5.6 0 0 0 0 3.2l-1.3 1 1.4 2.4 1.6-.6a5.6 5.6 0 0 0 2.7 1.6l.3 1.8h2.8l.3-1.8a5.6 5.6 0 0 0 2.7-1.6l1.6.6 1.4-2.4-1.3-1Z"
          fill="none"
          stroke="currentColor"
          stroke-width="1.2"
          stroke-linejoin="round"
          transform="translate(0.8 0.4) scale(0.9)"
        />
      </svg>
    </button>

    <button class="window" onclick={() => window.minimize()} aria-label="Minimise">
      <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M2 6h8" /></svg>
    </button>
    <button class="window" onclick={() => window.toggleMaximize()} aria-label="Maximise">
      <svg viewBox="0 0 12 12" aria-hidden="true"><rect x="2.5" y="2.5" width="7" height="7" /></svg>
    </button>
    <button class="window close" onclick={() => window.close()} aria-label="Close">
      <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M3 3l6 6M9 3l-6 6" /></svg>
    </button>
  </div>
</header>

<style>
  header {
    display: flex;
    align-items: center;
    height: 2.875rem;
    padding-left: 1.125rem;
    flex: none;
    user-select: none;
  }

  .mark {
    display: flex;
    align-items: center;
    gap: 0.625rem;
  }

  .glyph {
    width: 0.81rem;
    height: 0.81rem;
    border-radius: 0.19rem;
    background: var(--text-dim);
    opacity: 0.85;
  }

  .name {
    font-size: 0.91rem;
    font-weight: 500;
    color: var(--text);
    letter-spacing: 0.01em;
  }

  .controls {
    display: flex;
    align-items: center;
    margin-left: auto;
    height: 100%;
  }

  button {
    display: grid;
    place-items: center;
    background: none;
    border: 0;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
  }

  .gear {
    width: 2.125rem;
    height: 2.125rem;
    margin-right: 0.375rem;
    border-radius: var(--radius);
  }

  .gear svg {
    width: 0.94rem;
    height: 0.94rem;
  }

  .gear:hover {
    background: var(--row-hover);
    color: var(--text);
  }

  .window {
    width: 2.875rem;
    height: 100%;
  }

  .window svg {
    width: 0.75rem;
    height: 0.75rem;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.1;
  }

  .window:hover {
    background: var(--row-hover);
    color: var(--text);
  }

  .window.close:hover {
    background: var(--danger);
    color: var(--accent-text);
  }
</style>
