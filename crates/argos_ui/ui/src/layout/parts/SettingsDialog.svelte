<script lang="ts">
  /**
   * Settings, behind the gear.
   *
   * One section at a time, chosen from the rail. The window itself is one
   * screen with one button, and a settings panel that unrolled every option at
   * once would be the busiest thing in the application — so each section holds
   * a few controls and the others stay out of the way until asked for.
   *
   * Appearance is a section of its own rather than a block at the end of the
   * list: a theme is not a thing a scan does, and putting it beside the stages
   * would invite it to be read as one.
   *
   * These are preferences — what the *next* scan is asked for. Anything that
   * runs now and produces files (acquiring a disk, exporting artifacts,
   * searching a session again) belongs beside the thing it acts on, not here.
   *
   * Nothing in this file judges a recovery. A switch chooses what to ask the
   * engine for; what a stage does and what it finds are the engine's, and it
   * refuses a combination it cannot run rather than being pre-judged here
   * (`A-SHELL-NO-DOMAIN`).
   */
  import { session } from '../../lib/session.svelte';
  // Aliased: `active` is already the id of the chosen theme, a prop of this
  // component. This is the theme itself, for its artwork.
  import { active as inForce } from '../../themes/active.svelte';
  import { settings } from '../../lib/settings.svelte';
  import { loadTheme, themeIds } from '../../themes';
  import type { ThemeModule } from '../../themes/contract';

  let {
    active,
    onChoose,
    onClose,
  }: { active: string; onChoose: (id: string) => void; onClose: () => void } = $props();

  /** The sections, in the order the rail lists them. */
  const SECTIONS = [
    { id: 'recovery', name: 'Recovery' },
    { id: 'output', name: 'Output' },
    { id: 'appearance', name: 'Appearance' },
  ] as const;

  type SectionId = (typeof SECTIONS)[number]['id'];

  let open = $state<SectionId>('recovery');
  let choices = $state<ThemeModule[]>([]);

  $effect(() => {
    void Promise.all(themeIds().map(loadTheme)).then((loaded) => {
      choices = loaded;
    });
  });

  /**
   * The stages, in the order a scan runs them, each said in one line.
   *
   * The line describes what turning it *on* does, because that is the choice
   * being made.
   */
  const STAGES = [
    {
      key: 'filesystem' as const,
      name: 'Filesystem records',
      about: 'Recovers files whose records survived, with their original names and dates.',
    },
    {
      key: 'carving' as const,
      name: 'Surface carving',
      about: 'Reads the whole disk, finding images whose records are gone.',
    },
    {
      key: 'reassembly' as const,
      name: 'Fragment reassembly',
      about: 'Rebuilds images the disk stored in pieces. Needs surface carving.',
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
    <!--
      A window of the same system, not a card that happens to float: the frame,
      the title strip and the way out are the ones the main window uses, drawn
      from the same tokens and with the theme's own close glyph. A panel with
      its own idea of a border and its own X would be a second visual language
      inside one application.
    -->
    <header>
      <h2>Settings</h2>
      <div class="group">
        <button class="window close" onclick={onClose} aria-label="Close">
          <svg viewBox="0 0 12 12" aria-hidden="true">{@html inForce.icon('close')}</svg>
        </button>
      </div>
    </header>

    <div class="body">
      <nav aria-label="Settings sections">
        {#each SECTIONS as entry (entry.id)}
          <button
            class:current={open === entry.id}
            aria-current={open === entry.id}
            onclick={() => (open = entry.id)}
          >
            {entry.name}
          </button>
        {/each}
      </nav>

      <div class="panel">
        {#if open === 'recovery'}
          <div class="lead">
            <p>What the scan looks for. Each one takes time; turning it off saves that time.</p>
            {#if settings.customized}
              <button class="reset" onclick={() => settings.reset()} disabled={locked}>
                Restore defaults
              </button>
            {/if}
          </div>

          {#if locked}
            <!-- Said rather than hidden: a control that silently did nothing is
                 worse than one that explains when it will take effect. -->
            <p class="note">A scan is running. Changes apply to the next one.</p>
          {/if}

          <ul>
            {#each STAGES as stage (stage.key)}
              <li>
                <label class:off={!settings[stage.key]}>
                  <input
                    type="checkbox"
                    checked={settings[stage.key]}
                    disabled={locked}
                    onchange={(event) => settings.setStage(stage.key, event.currentTarget.checked)}
                  />
                  <span class="text">
                    <span class="name">{stage.name}</span>
                    <span class="about">{stage.about}</span>
                  </span>
                </label>

                <!-- Only while the stage it bounds is on: a limit on something
                     that will not run is a control with nothing to do. -->
                {#if stage.key === 'reassembly' && settings.reassembly}
                  <div class="nested">
                    <label class="inline">
                      <span class="name">Give up after</span>
                      <input
                        class="number"
                        type="number"
                        min="0"
                        step="1"
                        placeholder="120"
                        disabled={locked}
                        value={settings.reassemblyBudget ?? ''}
                        onchange={(event) =>
                          settings.setNumber('reassemblyBudget', entered(event.currentTarget.value))}
                      />
                      <span class="unit">minutes</span>
                    </label>
                    <p class="about">
                      This is the one stage that stops at a ceiling rather than at the end. Empty
                      keeps the two-hour limit; 0 searches every candidate however long it takes.
                    </p>
                  </div>
                {/if}
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
                <span class="text">
                  <span class="name">Labelling</span>
                  <span class="about">
                    Marks each image a photograph or an app asset. Never changes what is recovered.
                  </span>
                </span>
              </label>
            </li>
          </ul>
        {:else if open === 'output'}
          <div class="lead">
            <p>What reaches the folder. Nothing here changes what the scan finds.</p>
          </div>

          {#if locked}
            <p class="note">A scan is running. Changes apply to the next one.</p>
          {/if}

          <ul>
            <li>
              <label class:off={!settings.previews}>
                <input
                  type="checkbox"
                  checked={settings.previews}
                  disabled={locked}
                  onchange={(event) => settings.setFlag('previews', event.currentTarget.checked)}
                />
                <span class="text">
                  <span class="name">Thumbnails</span>
                  <span class="about">
                    Renders the small pictures the results gallery shows. Without them the gallery
                    has nothing to draw.
                  </span>
                </span>
              </label>
            </li>

            <li>
              <div class="field">
                <label class="inline">
                  <span class="name">Keep pictures at least</span>
                  <input
                    class="number"
                    type="number"
                    min="0"
                    step="1"
                    placeholder="300"
                    disabled={locked}
                    value={settings.minLongSide ?? ''}
                    onchange={(event) =>
                      settings.setNumber('minLongSide', entered(event.currentTarget.value))}
                  />
                  <span class="unit">px on the long side</span>
                </label>
                <p class="about">
                  A used disk holds far more icons and cache entries than photographs, and they are
                  small. Anything under this is still examined, hashed and recorded — it just does
                  not fill the folder. 0 writes everything.
                </p>
              </div>
            </li>

            <li>
              <div class="field">
                <label class="inline">
                  <span class="name">Use</span>
                  <input
                    class="number"
                    type="number"
                    min="1"
                    step="1"
                    placeholder="auto"
                    disabled={locked}
                    value={settings.jobs ?? ''}
                    onchange={(event) =>
                      settings.setNumber('jobs', entered(event.currentTarget.value))}
                  />
                  <span class="unit">worker threads</span>
                </label>
                <p class="about">
                  Left empty, the scan uses what the machine has. Fewer leaves the machine usable
                  for something else while it runs.
                </p>
              </div>
            </li>
          </ul>
        {:else}
          <div class="lead">
            <p>How the window looks. Changing this mid-scan interrupts nothing.</p>
          </div>

          <ul class="themes">
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
        {/if}
      </div>
    </div>
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

  /* Sized against the window rather than in fixed measures: the window can be
     small, and a panel that outgrew it would be a panel with no way out. */
  .dialog {
    display: flex;
    flex-direction: column;
    width: min(44rem, calc(100vw - 3rem));
    max-height: calc(100vh - 3rem);
    background: var(--pane);
    backdrop-filter: var(--pane-blur);
    -webkit-backdrop-filter: var(--pane-blur);
    border: 1px solid var(--window-border);
    border-radius: var(--window-radius);
    box-shadow: var(--window-shadow);
    padding: 0;
    overflow: hidden;
  }

  /* The title strip, on the terms the main window's is on. */
  header {
    display: flex;
    align-items: center;
    flex: none;
    padding: 0.42rem 0.42rem 0.42rem 1.25rem;
    border-bottom: 1px solid var(--titlebar-border);
    background: var(--titlebar);
  }

  h2 {
    margin: 0;
    font-size: 0.9rem;
    font-weight: 500;
    color: var(--titlebar-text);
  }

  /* The same strip the window buttons sit in, holding the one button a panel
     needs. */
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
    width: 3.4rem;
    height: 1.7rem;
    padding: 0;
    background: none;
    border: 0;
    color: var(--winbtn-text);
    cursor: pointer;
  }

  .window svg {
    width: 0.86rem;
    height: 0.86rem;
    fill: none;
    stroke: currentColor;
    stroke-width: var(--winbtn-stroke);
  }

  .window.close:hover {
    background: var(--winbtn-close-hover);
    color: var(--winbtn-close-hover-text);
  }

  .body {
    display: flex;
    flex: 1;
    min-height: 0;
    gap: 1.4rem;
    align-items: stretch;
    padding: 1.1rem 1.25rem 1.25rem;
  }

  /* The rail names three places and should do nothing else. A hairline carries
     the separation so the buttons themselves can stay quiet. */
  nav {
    display: flex;
    flex: none;
    width: 8rem;
    flex-direction: column;
    gap: 0.1rem;
    padding-right: 1.1rem;
    border-right: 1px solid var(--inset-border);
  }

  nav button {
    padding: 0.46rem 0.55rem;
    text-align: left;
    font: inherit;
    font-size: 0.8rem;
    color: var(--text-dim);
    background: none;
    border: 1px solid transparent;
    border-radius: var(--radius);
    cursor: pointer;
  }

  nav button:hover {
    background: var(--row-hover);
    color: var(--text);
  }

  nav button.current {
    color: var(--text);
    background: var(--row-selected);
    border-color: var(--row-selected-border);
  }

  .panel {
    flex: 1;
    min-width: 0;
    min-height: min(21rem, 40vh);
    overflow-y: auto;
  }

  .lead {
    display: flex;
    align-items: baseline;
    gap: 0.8rem;
    margin-bottom: 0.9rem;
  }

  .lead p {
    margin: 0;
    font-size: 0.76rem;
    line-height: 1.45;
    color: var(--text-dim);
  }

  .reset {
    margin-left: auto;
    flex: none;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-strong);
    font: inherit;
    font-size: 0.74rem;
    white-space: nowrap;
    cursor: pointer;
  }

  .reset:disabled {
    color: var(--text-faint);
    cursor: default;
  }

  .note {
    margin: 0 0 0.7rem;
    font-size: 0.74rem;
    color: var(--text-dim);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  /* Flat rows on a hairline, not a stack of filled boxes. Seven of those was
     the panel's whole problem: every one drew an edge, and the edges added up
     to more of the screen than the words did. */
  li + li {
    border-top: 1px solid var(--inset-border);
  }

  li > label,
  .field {
    display: flex;
    align-items: flex-start;
    gap: 0.85rem;
    padding: 0.75rem 0.5rem;
    color: var(--text);
  }

  li > label {
    cursor: pointer;
    border-radius: var(--radius);
  }

  li > label:hover {
    background: var(--row-hover);
  }

  .field {
    flex-direction: column;
    gap: 0.35rem;
  }

  /* A stage that will not run is dimmed rather than removed: what a scan is
     not going to do is as much a part of the account as what it will. */
  li > label.off .text {
    opacity: 0.5;
  }

  /* A switch rather than a tick box.
     Still a real checkbox — focusable, keyboard-operable, announced as one —
     with `appearance: none` turning the control itself into the track and its
     `::after` into the thumb. One element, so nothing can drift out of step
     with what the input actually holds.

     Every colour and both corners come from the theme: a pill is right for one
     generation of interface and wrong for another, and a terminal's toggle is
     square. The measurements are the layout's and are the same everywhere. */
  input[type='checkbox'] {
    appearance: none;
    -webkit-appearance: none;
    position: relative;
    flex: none;
    margin: 0.08rem 0 0;
    width: 2.2rem;
    height: 1.2rem;
    border-radius: var(--switch-radius);
    background: var(--switch-track);
    border: 1px solid var(--switch-border);
    cursor: pointer;
    transition:
      background 130ms ease,
      border-color 130ms ease;
  }

  input[type='checkbox']::after {
    content: '';
    position: absolute;
    top: 50%;
    left: 0.13rem;
    transform: translateY(-50%);
    width: 0.86rem;
    height: 0.86rem;
    border-radius: var(--switch-thumb-radius);
    background: var(--switch-thumb);
    transition: left 130ms ease;
  }

  input[type='checkbox']:checked {
    background: var(--switch-track-on);
    border-color: var(--switch-border-on);
  }

  input[type='checkbox']:checked::after {
    left: calc(100% - 0.99rem);
  }

  input[type='checkbox']:disabled {
    cursor: default;
    opacity: 0.5;
  }

  input[type='checkbox']:focus-visible {
    outline: 2px solid var(--accent-strong);
    outline-offset: 2px;
  }

  .text {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 0;
  }

  .name {
    font-size: 0.85rem;
  }

  .about {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.45;
    color: var(--text-dim);
  }

  /* Indented under the stage it bounds, carried by space alone — a rule here
     would be a third edge in a row that already has enough. */
  .nested {
    padding: 0 0.5rem 0.8rem 3.55rem;
  }

  .inline {
    display: flex;
    align-items: baseline;
    gap: 0.45rem;
    padding: 0;
    background: none;
    border: 0;
  }

  .unit {
    font-size: 0.75rem;
    color: var(--text-faint);
  }

  .number {
    width: 4.6rem;
    padding: 0.24rem 0.45rem;
    background: var(--inset);
    border: 1px solid var(--inset-border);
    border-radius: var(--radius);
    color: var(--text);
    font: inherit;
    font-size: 0.8rem;
  }

  .nested .about {
    margin-top: 0.35rem;
  }

  .themes li {
    border-top: 0;
  }

  .themes li button.theme {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    width: 100%;
    margin-bottom: 0.4rem;
    padding: 0.66rem 0.75rem;
    text-align: left;
    background: var(--inset);
    border: 1px solid transparent;
    border-radius: var(--radius);
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }

  .themes li button.theme:hover {
    background: var(--row-hover);
  }

  .themes li button.theme.current {
    background: var(--row-selected);
    border-color: var(--row-selected-border);
  }

  .swatch {
    width: 1.31rem;
    height: 1.31rem;
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
