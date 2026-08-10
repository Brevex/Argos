<script lang="ts">
  /**
   * The window's own title bar: the name on the left, the window buttons on
   * the right, and a drag region across everything between.
   *
   * Its geometry is fixed here and not by the theme. A theme decides whether
   * the three buttons are a glass strip or three bare glyphs; it does not
   * decide where they are or how big they are, because a control that moves
   * when the colours change is a control the user has to find again.
   */
  import { getCurrentWindow } from '@tauri-apps/api/window';

  import { active } from '../../themes/active.svelte';

  const window = getCurrentWindow();
</script>

<header data-tauri-drag-region>
  <span class="name" data-tauri-drag-region>Argos</span>

  <!-- Not a drag region: a drag region takes the press before a button sees
       it, and these three are the buttons a window most needs to work. -->
  <div class="group">
    <button class="window" onclick={() => window.minimize()} aria-label="Minimise">
      <svg viewBox="0 0 12 12" aria-hidden="true">{@html active.icon('minimise')}</svg>
    </button>
    <button class="window" onclick={() => window.toggleMaximize()} aria-label="Maximise">
      <svg viewBox="0 0 12 12" aria-hidden="true">{@html active.icon('maximise')}</svg>
    </button>
    <button class="window close" onclick={() => window.close()} aria-label="Close">
      <svg viewBox="0 0 12 12" aria-hidden="true">{@html active.icon('close')}</svg>
    </button>
  </div>
</header>

<style>
  header {
    display: flex;
    align-items: center;
    height: 2.9rem;
    padding: 0 0.75rem 0 1.6rem;
    flex: none;
    user-select: none;
    background: var(--titlebar);
    border-bottom: 1px solid var(--titlebar-border);
  }

  .name {
    font-size: 1.05rem;
    font-weight: 500;
    color: var(--titlebar-text);
    letter-spacing: 0.01em;
    text-shadow: var(--text-glow);
  }

  /* One strip, divided into three. With a transparent fill and transparent
     dividers this is three bare glyphs and nothing else — but always these
     three, always this size, always here. */
  .group {
    display: flex;
    align-items: stretch;
    margin-left: auto;
    background: var(--winbtn-group);
    border: 1px solid var(--winbtn-group-border);
    border-radius: var(--winbtn-group-radius);
    overflow: hidden;
  }

  .window {
    display: grid;
    place-items: center;
    width: 2.6rem;
    height: 1.7rem;
    padding: 0;
    background: none;
    border: 0;
    color: var(--winbtn-text);
    cursor: pointer;
  }

  .window + .window {
    border-left: 1px solid var(--winbtn-divider);
  }

  .window.close {
    width: 3.4rem;
  }

  .window svg {
    width: 0.86rem;
    height: 0.86rem;
    fill: none;
    stroke: currentColor;
    stroke-width: var(--winbtn-stroke);
  }

  .window:hover {
    background: var(--winbtn-hover);
  }

  .window.close:hover {
    background: var(--winbtn-close-hover);
    color: var(--winbtn-close-hover-text);
  }
</style>
