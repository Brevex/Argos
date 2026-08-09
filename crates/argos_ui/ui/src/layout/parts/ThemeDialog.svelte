<script lang="ts">
  /**
   * The theme picker, behind the gear.
   *
   * Choosing a theme rewrites custom properties on the document root. Nothing
   * is remounted and no state is lost, so this can be opened and used in the
   * middle of a running scan without interrupting it.
   */
  import { loadTheme, themeIds } from '../../themes';
  import type { ThemeModule } from '../../themes/contract';

  let {
    active,
    onChoose,
    onClose,
  }: { active: string; onChoose: (id: string) => void; onClose: () => void } = $props();

  let choices = $state<ThemeModule[]>([]);

  $effect(() => {
    void Promise.all(themeIds().map(loadTheme)).then((loaded) => {
      choices = loaded;
    });
  });
</script>

<svelte:window onkeydown={(event) => event.key === 'Escape' && onClose()} />

<div
  class="scrim"
  role="presentation"
  onclick={(event) => event.target === event.currentTarget && onClose()}
>
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Theme">
    <header>
      <h2>Theme</h2>
      <button class="close" onclick={onClose} aria-label="Close">
        <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M3 3l6 6M9 3l-6 6" /></svg>
      </button>
    </header>

    <ul>
      {#each choices as choice (choice.id)}
        <li>
          <button
            class:current={choice.id === active}
            aria-pressed={choice.id === active}
            onclick={() => onChoose(choice.id)}
          >
            <span class="swatch" style:background={choice.tokens['--accent']}></span>
            <span class="text">
              <span class="name">{choice.name}</span>
              <span class="what">{choice.description}</span>
            </span>
            {#if choice.id === active}
              <svg class="tick" viewBox="0 0 14 14" aria-hidden="true">
                <path d="M2.5 7.4l3 3 6-6.4" />
              </svg>
            {/if}
          </button>
        </li>
      {/each}
    </ul>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: var(--scrim);
    backdrop-filter: blur(2px);
    -webkit-backdrop-filter: blur(2px);
  }

  .dialog {
    width: min(28rem, calc(100vw - 3rem));
    background: var(--pane);
    backdrop-filter: var(--pane-blur);
    -webkit-backdrop-filter: var(--pane-blur);
    border: 1px solid var(--pane-border);
    border-radius: var(--pane-radius);
    box-shadow: var(--pane-shadow);
    padding: 1rem;
  }

  header {
    display: flex;
    align-items: center;
    margin-bottom: 0.75rem;
  }

  h2 {
    margin: 0;
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--text);
  }

  .close {
    margin-left: auto;
    display: grid;
    place-items: center;
    width: 1.625rem;
    height: 1.625rem;
    border: 0;
    border-radius: var(--radius);
    background: none;
    color: var(--text-dim);
    cursor: pointer;
  }

  .close:hover {
    background: var(--row-hover);
    color: var(--text);
  }

  .close svg {
    width: 0.69rem;
    height: 0.69rem;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.2;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }

  li button {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    width: 100%;
    padding: 0.69rem 0.81rem;
    text-align: left;
    background: var(--inset);
    border: 1px solid transparent;
    border-radius: var(--radius);
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }

  li button:hover {
    background: var(--row-hover);
  }

  li button.current {
    background: var(--row-selected);
    border-color: var(--row-selected-border);
  }

  .swatch {
    width: 1.375rem;
    height: 1.375rem;
    flex: none;
    border-radius: 50%;
    border: 1px solid var(--pane-border);
  }

  .text {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    min-width: 0;
  }

  .name {
    font-size: 0.84rem;
  }

  .what {
    font-size: 0.72rem;
    color: var(--text-faint);
  }

  .tick {
    margin-left: auto;
    width: 0.94rem;
    height: 0.94rem;
    flex: none;
    fill: none;
    stroke: var(--accent-strong);
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
