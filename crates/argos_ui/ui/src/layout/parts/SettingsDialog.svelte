<script lang="ts">
  /**
   * Settings, behind the gear: what the next scan runs, and what it looks like.
   *
   * The recovery switches are the same options `argos scan` takes as flags, and
   * they open on the same defaults, so the button runs what the command line
   * runs until someone decides otherwise (`A-CLI-FIRST`). Nothing here judges a
   * recovery — a switch chooses what to ask the engine for, and the engine
   * decides everything about what that means (`A-SHELL-NO-DOMAIN`).
   *
   * Choosing a theme rewrites custom properties on the document root. Nothing
   * is remounted and no state is lost, so this can be opened and used in the
   * middle of a running scan without interrupting it.
   */
  import { session } from '../../lib/session.svelte';
  import { MEASURED_COST, settings } from '../../lib/settings.svelte';
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

  /**
   * The stages, in the order a scan runs them.
   *
   * `needs` is what a stage has nothing to work from without — reassembly works
   * on the candidates carving could not complete. The store keeps the pair
   * consistent; this only says so on screen.
   */
  const stages = [
    {
      key: 'filesystem' as const,
      name: 'Filesystem records',
      about: 'Recovers files whose metadata survived, with their names and dates.',
    },
    {
      key: 'carving' as const,
      name: 'Surface carving',
      about: 'Reads the whole medium looking for images whose metadata is gone.',
    },
    {
      key: 'reassembly' as const,
      name: 'Fragment reassembly',
      about: 'Rebuilds images the medium stored in pieces. Needs surface carving.',
    },
  ];

  /** A run is under way, so these apply to the next one rather than to it. */
  const locked = $derived(session.running);

  /** Reads a number field, treating an empty box as "take the default". */
  function entered(value: string): number | null {
    if (value.trim() === '') return null;
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
</script>

<svelte:window onkeydown={(event) => event.key === 'Escape' && onClose()} />

<div
  class="scrim"
  role="presentation"
  onclick={(event) => event.target === event.currentTarget && onClose()}
>
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Settings">
    <header>
      <h2>Settings</h2>
      <button class="close" onclick={onClose} aria-label="Close">
        <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M3 3l6 6M9 3l-6 6" /></svg>
      </button>
    </header>

    <section>
      <div class="legend">
        <h3>Recovery</h3>
        {#if settings.customized}
          <button class="reset" onclick={() => settings.reset()} disabled={locked}>
            Restore defaults
          </button>
        {/if}
      </div>

      {#if locked}
        <!-- Said rather than hidden: a control that silently did nothing is
             worse than one that explains when it will. -->
        <p class="note">A scan is running. Changes apply to the next one.</p>
      {/if}

      <ul class="switches">
        {#each stages as stage (stage.key)}
          <li>
            <label class:off={!settings[stage.key]}>
              <input
                type="checkbox"
                checked={settings[stage.key]}
                disabled={locked}
                onchange={(event) => settings.setStage(stage.key, event.currentTarget.checked)}
              />
              <span class="body">
                <span class="name">
                  {stage.name}
                  <span class="cost">{MEASURED_COST[stage.key]}</span>
                </span>
                <span class="about">{stage.about}</span>
              </span>
            </label>
          </li>
        {/each}

        <li>
          <label class:off={!settings.triage}>
            <input
              type="checkbox"
              checked={settings.triage}
              disabled={locked}
              onchange={(event) => settings.setFlag('triage', event.currentTarget.checked)}
            />
            <span class="body">
              <span class="name">
                Labelling
                <span class="cost">{MEASURED_COST.triage}</span>
              </span>
              <span class="about">
                Marks each image photograph or synthetic asset. Never changes what is recovered.
              </span>
            </span>
          </label>
        </li>

        <li>
          <label class:off={!settings.previews}>
            <input
              type="checkbox"
              checked={settings.previews}
              disabled={locked}
              onchange={(event) => settings.setFlag('previews', event.currentTarget.checked)}
            />
            <span class="body">
              <span class="name">Thumbnails</span>
              <span class="about">
                {settings.previews
                  ? 'Renders the pictures the results gallery draws.'
                  : 'Off: the results gallery will have no pictures to show.'}
              </span>
            </span>
          </label>
        </li>
      </ul>

      <div class="numbers">
        <label>
          <span class="name">Smallest picture kept</span>
          <span class="field">
            <input
              type="number"
              min="0"
              step="1"
              placeholder="300"
              disabled={locked}
              value={settings.minLongSide ?? ''}
              onchange={(event) =>
                settings.setNumber('minLongSide', entered(event.currentTarget.value))}
            />
            <span class="unit">px</span>
          </span>
          <span class="about">
            Long side, in pixels. Anything smaller is still examined, hashed and recorded — it just
            does not fill the folder. 0 writes everything.
          </span>
        </label>

        <label>
          <span class="name">Worker threads</span>
          <span class="field">
            <input
              type="number"
              min="1"
              step="1"
              placeholder="auto"
              disabled={locked}
              value={settings.jobs ?? ''}
              onchange={(event) => settings.setNumber('jobs', entered(event.currentTarget.value))}
            />
          </span>
          <span class="about">Left empty, the scan uses what the machine has.</span>
        </label>
      </div>
    </section>

    <section>
      <div class="legend"><h3>Appearance</h3></div>
      <ul>
        {#each choices as choice (choice.id)}
          <li>
            <button
              class="theme"
              class:current={choice.id === active}
              aria-pressed={choice.id === active}
              onclick={() => onChoose(choice.id)}
            >
              <span class="swatch" style:background={choice.tokens['--accent']}></span>
              <span class="name">{choice.name}</span>
              {#if choice.id === active}
                <svg class="tick" viewBox="0 0 14 14" aria-hidden="true">
                  <path d="M2.5 7.4l3 3 6-6.4" />
                </svg>
              {/if}
            </button>
          </li>
        {/each}
      </ul>
    </section>
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
    width: min(30rem, calc(100vw - 3rem));
    /* The panel grew a section; on a short window it scrolls rather than
       running off the bottom where the close button cannot be reached. */
    max-height: calc(100vh - 4rem);
    overflow-y: auto;
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

  section + section {
    margin-top: 1.15rem;
  }

  .legend {
    display: flex;
    align-items: baseline;
    margin-bottom: 0.5rem;
  }

  h3 {
    margin: 0;
    font-size: 0.72rem;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-faint);
  }

  .reset {
    margin-left: auto;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-strong);
    font: inherit;
    font-size: 0.72rem;
    cursor: pointer;
  }

  .reset:disabled {
    color: var(--text-faint);
    cursor: default;
  }

  .note {
    margin: 0 0 0.5rem;
    font-size: 0.72rem;
    color: var(--text-dim);
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

  .switches label {
    display: flex;
    align-items: flex-start;
    gap: 0.65rem;
    padding: 0.6rem 0.81rem;
    background: var(--inset);
    border: 1px solid transparent;
    border-radius: var(--radius);
    color: var(--text);
    cursor: pointer;
  }

  .switches label:hover {
    background: var(--row-hover);
  }

  /* A stage that will not run is dimmed rather than removed: what a scan is
     not going to do is as much a part of the account as what it will. */
  .switches label.off .body {
    opacity: 0.55;
  }

  .switches input,
  .numbers input {
    accent-color: var(--accent-strong);
  }

  .switches input {
    margin: 0.15rem 0 0;
    flex: none;
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 0.12rem;
    min-width: 0;
  }

  .name {
    font-size: 0.84rem;
  }

  .cost {
    margin-left: 0.45rem;
    font-size: 0.7rem;
    color: var(--text-faint);
  }

  .about {
    font-size: 0.71rem;
    line-height: 1.35;
    color: var(--text-dim);
  }

  .numbers {
    display: flex;
    gap: 0.6rem;
    margin-top: 0.5rem;
  }

  .numbers label {
    display: flex;
    flex: 1;
    min-width: 0;
    flex-direction: column;
    gap: 0.2rem;
    padding: 0.6rem 0.81rem;
    background: var(--inset);
    border: 1px solid transparent;
    border-radius: var(--radius);
    color: var(--text);
  }

  .field {
    display: flex;
    align-items: baseline;
    gap: 0.3rem;
  }

  .numbers input {
    width: 100%;
    min-width: 0;
    padding: 0.25rem 0.4rem;
    background: var(--pane);
    border: 1px solid var(--inset-border);
    border-radius: var(--radius);
    color: var(--text);
    font: inherit;
    font-size: 0.8rem;
  }

  .unit {
    font-size: 0.72rem;
    color: var(--text-faint);
  }

  li button.theme {
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

  li button.theme:hover {
    background: var(--row-hover);
  }

  li button.theme.current {
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
