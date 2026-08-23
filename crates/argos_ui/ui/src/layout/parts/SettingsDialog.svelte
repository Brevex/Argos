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
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import { loadTheme, themeIds } from '../../themes';
  import type { ThemeModule } from '../../themes/contract';
  import NumberField from './NumberField.svelte';

  /**
   * Names the photograph whose tables are lent.
   *
   * A file rather than a switch, because the technique cannot work without one:
   * there is nothing to fall back on if no sibling is named.
   */
  async function chooseReference(): Promise<void> {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      title: 'A photograph from the same camera as what is missing',
      filters: [{ name: 'Photographs', extensions: ['jpg', 'jpeg', 'JPG', 'JPEG'] }],
    });
    if (typeof picked === 'string') settings.setReference(picked);
  }

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
                      <NumberField
                        value={settings.reassemblyBudget}
                        placeholder="120"
                        disabled={locked}
                        label="Give up reassembling after, in minutes"
                        onChange={(value) => settings.setNumber('reassemblyBudget', value)}
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

            <li>
              <div class="field">
                <span class="name">Fragments with no header</span>
                <p class="about">
                  A fragment whose beginning is gone cannot be decoded on its own. Name a
                  photograph from the same camera and its tables are lent to those fragments.
                </p>
                <p class="about">
                  What comes out is <em>pixels, not files</em> — it lands in a
                  <code>grafted</code> folder, apart from what was recovered, and never in the
                  manifest.
                </p>
                <div class="picker">
                  <button class="choose" disabled={locked} onclick={chooseReference}>
                    {settings.reference ? 'Change photograph' : 'Choose a photograph'}
                  </button>
                  {#if settings.reference}
                    <button class="clear" disabled={locked} onclick={() => settings.setReference(null)}>
                      Off
                    </button>
                  {/if}
                </div>
                {#if settings.reference}
                  <p class="chosen" title={settings.reference}>{settings.reference}</p>
                {:else}
                  <p class="chosen off">Off. Nothing extra is searched for.</p>
                {/if}
              </div>
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
                  <NumberField
                    value={settings.minLongSide}
                    placeholder="300"
                    disabled={locked}
                    label="Keep pictures at least, in pixels on the long side"
                    onChange={(value) => settings.setNumber('minLongSide', value)}
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
                  <NumberField
                    value={settings.jobs}
                    placeholder="auto"
                    min={1}
                    disabled={locked}
                    label="Worker threads to use"
                    onChange={(value) => settings.setNumber('jobs', value)}
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
                      {@html inForce.icon('tick')}
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
  /*
   * A window, built the way the main one is: the frame is the whole element,
   * the title sits on it, and the panel is inset into it by the width of the
   * band. A theme whose windows have no band gives the inset no width, and the
   * panel then covers everything but the title — which is the same panel it
   * was before.
   *
   * Sized against the window rather than in fixed measures: the window can be
   * small, and a panel that outgrew it would be a panel with no way out.
   */
  .dialog {
    position: relative;
    display: flex;
    flex-direction: column;
    width: min(44rem, calc(100vw - 3rem));
    max-height: calc(100vh - 3rem);
    background: var(--dialog-strip);
    backdrop-filter: var(--dialog-blur);
    -webkit-backdrop-filter: var(--dialog-blur);
    border: 1px solid var(--window-edge);
    border-radius: var(--window-radius);
    box-shadow: var(--window-shadow);
    padding: 0;
    overflow: hidden;
  }

  /* The same bright line inside the dark edge that the main window has. */
  .dialog::before {
    content: '';
    position: absolute;
    inset: 0;
    z-index: 3;
    pointer-events: none;
    border-radius: inherit;
    box-shadow: inset 0 0 0 1px var(--window-border);
  }

  /*
   * The title strip, and the one piece of glass in this application that is
   * real.
   *
   * The window's own frame lies over the desktop, which no filter in a web
   * view can reach; this lies over the window, which *is* the page — so the
   * blur behind it blurs something, and on a theme whose windows are framed in
   * glass this is that glass, measured on the same profile as the frame. On a
   * theme whose window is one unbroken surface it is the panel's own fill and
   * the panel's own filter, and there is no strip to see.
   */
  header {
    display: flex;
    align-items: center;
    flex: none;
    height: var(--dialog-strip-height);
    padding: 0 var(--winbtn-offset-right) 0 1.25rem;
  }

  h2 {
    margin: 0;
    font-size: var(--type-md);
    font-weight: 600;
    color: var(--titlebar-text);
    text-shadow: var(--titlebar-text-shadow, var(--text-glow));
  }

  /* The same strip the window buttons sit in, holding the one button a panel
     needs. */
  .group {
    display: flex;
    align-items: stretch;
    gap: var(--winbtn-gap);
    margin-left: auto;
    /* The group hangs from the band's top edge on a theme whose windows do,
       and sits on the title's own line on one whose windows do not. */
    align-self: var(--winbtn-align);
    margin-top: var(--winbtn-offset-top);
    background: var(--winbtn-group);
    border: 1px solid var(--winbtn-group-border);
    border-top: 0;
    border-radius: var(--winbtn-group-radius);
    box-shadow: var(--winbtn-group-shadow);
    overflow: hidden;
  }

  .window {
    display: grid;
    place-items: center;
    width: var(--winbtn-close-width);
    height: var(--winbtn-height);
    padding: 0;
    background: var(--winbtn-close);
    border: 1px solid;
    border-color: var(--winbtn-border);
    border-radius: var(--winbtn-radius);
    color: var(--winbtn-close-text);
    cursor: pointer;
  }

  /* The glyph is the theme's drawing, and it says for itself whether it is a
     line or a filled shape with an outline — the caption glyphs of a bevelled
     desktop are the second, and forcing `fill: none` here made them the
     first. All this decides is how big it is and what colour it inherits. */
  .window svg {
    width: var(--winbtn-glyph);
    height: var(--winbtn-glyph);
    stroke-width: var(--winbtn-stroke);
  }

  .window.close:hover {
    background: var(--winbtn-close-hover);
    color: var(--winbtn-close-hover-text);
  }

  .window.close:active {
    background: var(--winbtn-close-active);
    color: var(--winbtn-close-hover-text);
  }

  .window:focus-visible {
    outline: var(--focus-outline);
    outline-offset: -2px;
  }

  /* The panel, inside the frame — the dialog's client area, edged the way the
     main window's client area is edged. */
  .body {
    display: flex;
    flex: 1;
    min-height: 0;
    gap: 1.4rem;
    align-items: stretch;
    margin: 0 var(--dialog-inset) var(--dialog-inset);
    padding: 1.1rem 1.25rem 1.25rem;
    background: var(--pane);
    border: 1px solid var(--main-border);
    border-radius: var(--main-radius);
    box-shadow: var(--main-shadow);
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
    font-size: var(--type-sm);
    color: var(--text-dim);
    background: none;
    border: 1px solid transparent;
    border-radius: var(--row-radius);
    cursor: pointer;
  }

  nav button:hover {
    background: var(--row-hover);
    color: var(--text);
  }

  nav button:active {
    background: var(--row-selected);
  }

  /* The section being shown is the selected row of a list, drawn the way the
     drive table draws one — not an outline that has to be looked for. */
  nav button.current {
    color: var(--text);
    background: var(--row-selected);
    border-color: var(--row-selected-border);
    box-shadow: var(--row-selected-shadow);
  }

  nav button:focus-visible {
    outline: var(--focus-outline);
    outline-offset: -3px;
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

  /*
   * Three levels of voice, and they never trade places: what a control is
   * called, what it does, and what a reader has to be told about the state it
   * is in. The name is the body size in the text colour; the description is a
   * step down and dimmed; a note is smaller again.
   */
  .lead p {
    margin: 0;
    max-width: var(--measure);
    font-size: var(--type-xs);
    line-height: 1.5;
    color: var(--text-dim);
  }

  /* A word rather than an object: the third kind of button this interface
     has, and the only place one appears. */
  .reset {
    margin-left: auto;
    flex: none;
    padding: 0;
    border: 0;
    background: none;
    color: var(--link);
    font: inherit;
    font-size: var(--type-xs);
    white-space: nowrap;
    text-decoration: underline;
    text-underline-offset: 0.15em;
    cursor: pointer;
  }

  .reset:hover:not(:disabled) {
    color: var(--link-hover);
  }

  .reset:disabled {
    color: var(--disabled-text);
    text-decoration: none;
    cursor: default;
  }

  .note {
    margin: 0 0 0.7rem;
    font-size: var(--type-2xs);
    color: var(--text-faint);
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
    border-radius: var(--row-radius);
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
    opacity: 0.55;
  }

  /*
   * On or off, in whichever of the three shapes the theme draws it.
   *
   * The control is a real checkbox in every one of them — focusable, operated
   * by the keyboard, announced as a checkbox — with `appearance: none` handing
   * the drawing to the theme. It keeps the same footprint whichever shape it
   * takes, so the words beside it do not move when the theme changes; a tick
   * box narrower than a switch is drawn centred in the switch's width, which
   * also leaves it a target far bigger than the box.
   */
  input[type='checkbox'] {
    appearance: none;
    -webkit-appearance: none;
    position: relative;
    flex: none;
    margin: 0.08rem 0 0;
    width: 2.2rem;
    height: 1.2rem;
    background: none;
    border: 0;
    cursor: pointer;
  }

  input[type='checkbox']:disabled {
    cursor: default;
    opacity: var(--disabled-opacity);
  }

  input[type='checkbox']:focus-visible {
    outline: var(--focus-outline);
    outline-offset: 2px;
  }

  /* --- a switch: a track with a thumb that slides across it -------------- */
  :global(html[data-checkbox='switch']) input[type='checkbox'] {
    border-radius: var(--switch-radius);
    background: var(--switch-track);
    border: 1px solid var(--switch-border);
    transition:
      background 130ms ease,
      border-color 130ms ease;
  }

  :global(html[data-checkbox='switch']) input[type='checkbox']::after {
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

  :global(html[data-checkbox='switch']) input[type='checkbox']:checked {
    background: var(--switch-track-on);
    border-color: var(--switch-border-on);
  }

  :global(html[data-checkbox='switch']) input[type='checkbox']:checked::after {
    left: calc(100% - 0.99rem);
  }

  /* --- a tick box: a sunken well with a mark cut into it ----------------- */
  :global(html[data-checkbox='checkbox']) input[type='checkbox']::before {
    content: '';
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 1.05rem;
    height: 1.05rem;
    background: var(--check-box);
    border: 1px solid var(--check-border);
    border-radius: var(--check-radius);
    box-shadow: var(--check-shadow);
  }

  :global(html[data-checkbox='checkbox']) input[type='checkbox']:checked::before {
    background: var(--check-box-checked);
    border-color: var(--check-border-checked);
  }

  :global(html[data-checkbox='checkbox']) input[type='checkbox']:checked::after {
    content: '';
    position: absolute;
    top: 50%;
    left: 50%;
    width: 0.62rem;
    height: 0.34rem;
    transform: translate(-50%, -70%) rotate(-45deg);
    border-left: 0.16rem solid var(--check-mark);
    border-bottom: 0.16rem solid var(--check-mark);
  }

  :global(html[data-checkbox='checkbox']) input[type='checkbox']:hover::before {
    border-color: var(--check-border-hover);
    box-shadow: var(--check-shadow-hover);
  }

  /* --- brackets: a cell that is lit or unlit ----------------------------- */
  :global(html[data-checkbox='bracket']) input[type='checkbox']::before {
    content: '[ ]';
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    /* An absolutely positioned box offset from the left shrinks to the room
       left of it, which is narrower than three characters: without this the
       brackets stack one above the other. */
    width: max-content;
    white-space: pre;
    font-family: var(--font);
    font-size: var(--type-md);
    line-height: 1;
    color: var(--check-border);
  }

  :global(html[data-checkbox='bracket']) input[type='checkbox']:checked::before {
    content: '[X]';
    color: var(--check-mark);
    text-shadow: var(--text-glow);
  }

  :global(html[data-checkbox='bracket']) input[type='checkbox']:hover::before {
    color: var(--check-border-checked);
  }

  .text {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 0;
  }

  .name {
    font-size: var(--type-md);
  }

  .about {
    margin: 0;
    max-width: var(--measure);
    font-size: var(--type-xs);
    line-height: 1.5;
    color: var(--text-dim);
  }

  /* Indented under the stage it bounds, carried by space alone — a rule here
     would be a third edge in a row that already has enough. */
  .nested {
    padding: 0 0.5rem 0.8rem 3.55rem;
  }

  .inline {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    padding: 0;
    background: none;
    border: 0;
  }

  .unit {
    font-size: var(--type-xs);
    color: var(--text-faint);
  }

  .nested .about {
    margin-top: 0.35rem;
  }

  /* The picker for the photograph whose tables are lent, and what it produced.
     Two ordinary buttons and a line of state under them — not a control that
     leaves a person guessing what it did. */
  .picker {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-top: 0.2rem;
  }

  .choose,
  .clear {
    position: relative;
    padding: 0.3rem 0.85rem;
    font: inherit;
    font-size: var(--type-xs);
    color: var(--button-text);
    background: var(--button);
    border: 1px solid var(--button-border);
    border-radius: var(--button-radius);
    box-shadow: var(--button-shadow);
    cursor: pointer;
  }

  .choose::after,
  .clear::after {
    content: '';
    position: absolute;
    inset: 0;
    pointer-events: none;
    border-radius: inherit;
    background: var(--specular);
  }

  .choose:hover:not(:disabled),
  .clear:hover:not(:disabled) {
    background: var(--button-hover);
    border-color: var(--button-border-hover);
    color: var(--button-text-hover);
  }

  .choose:active:not(:disabled),
  .clear:active:not(:disabled) {
    background: var(--button-active);
    box-shadow: var(--button-shadow-active);
    color: var(--button-text-hover);
  }

  .choose:disabled,
  .clear:disabled {
    color: var(--disabled-text);
    opacity: var(--disabled-opacity);
    cursor: default;
  }

  /* What the picker chose, or that it chose nothing: state, in the quietest
     voice the panel has, and never mistakable for a description. */
  .chosen {
    margin: 0.45rem 0 0;
    font-size: var(--type-2xs);
    line-height: 1.45;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }

  .chosen.off {
    color: var(--text-faint);
    direction: ltr;
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
    border: 1px solid var(--inset-border);
    border-radius: var(--row-radius);
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }

  .themes li button.theme:hover {
    background: var(--row-hover);
    border-color: var(--inset-border-hover);
  }

  .themes li button.theme:active {
    background: var(--button-active);
    box-shadow: var(--button-shadow-active);
  }

  .themes li button.theme.current {
    background: var(--row-selected);
    border-color: var(--row-selected-border);
    box-shadow: var(--row-selected-shadow);
  }

  .themes li button.theme:focus-visible {
    outline: var(--focus-outline);
    outline-offset: -3px;
  }

  .swatch {
    width: 1.31rem;
    height: 1.31rem;
    flex: none;
    border-radius: 50%;
    border: 1px solid var(--pane-border);
    box-shadow: var(--bevel-raised);
  }

  .tick {
    margin-left: auto;
    width: 0.94rem;
    height: 0.94rem;
    flex: none;
    color: var(--accent-strong);
  }
</style>
