<script lang="ts">
  /**
   * The base layout, and the only structure this application has.
   *
   * One screen, three blocks and one button: choose a drive, choose where the
   * images go, watch what happens. There are no other views and no navigation,
   * because there is nothing else the tool does — everything a scan produces
   * lands in the destination folder, and the manifest beside it is the record.
   *
   * A theme contributes values for the tokens this markup reads and nothing
   * else: no component here is ever swapped, so switching a theme costs
   * nothing and loses nothing, including mid-scan.
   */
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';

  import type { Device } from '../lib/dto';
  import * as ipc from '../lib/ipc';
  import { session, subscribe } from '../lib/session.svelte';
  import { settings } from '../lib/settings.svelte';
  import { active, apply, remembered } from '../themes/active.svelte';

  import TitleBar from './parts/TitleBar.svelte';
  import DriveTable from './parts/DriveTable.svelte';
  import Destination from './parts/Destination.svelte';
  import Activity from './parts/Activity.svelte';
  import QuitDialog from './parts/QuitDialog.svelte';
  import SettingsDialog from './parts/SettingsDialog.svelte';

  /** How often the elapsed clock advances while a scan runs. */
  const TICK_MS = 500;

  let devices = $state<Device[]>([]);
  let source = $state('');
  let destination = $state('');
  let busy = $state(false);
  let refreshing = $state(false);
  let settingsOpen = $state(false);
  /**
   * Whether the window is asking whether to abandon a run.
   *
   * Only ever while one is under way. A window with nothing running closes on
   * the first press, because there is nothing to lose by closing it.
   */
  let quitAsking = $state(false);
  /**
   * Which job the button runs.
   *
   * Two, and only two, because they are the two things this tool does to a
   * disk: read it for images, or copy it so it never has to be read again.
   * They are a choice rather than two screens — the drive and the destination
   * are the same two questions either way.
   */
  let job = $state<'scan' | 'acquire'>('scan');
  /** Where each job writes, kept apart so switching does not lose either. */
  let imagePath = $state('');

  const target = $derived(job === 'acquire' ? imagePath : destination);
  const ready = $derived(source !== '' && target !== '' && !busy);

  /**
   * The single button: what it says, and whether it can be pressed.
   *
   * Cancel stops the search and lets the run write what the search found —
   * on a large medium, the hours of reading that came before the press. That
   * writing is itself a stage worth minutes, so the button stays live and a
   * second press gives it up too. A copy has nothing after its stop, so there
   * its button is spent.
   */
  const action = $derived.by(() => {
    if (session.stopping) {
      return session.job === 'acquire'
        ? { label: 'Stopping…', enabled: false }
        : { label: 'Skip writing', enabled: !busy };
    }
    if (session.running) return { label: 'Cancel', enabled: !busy };
    return { label: job === 'acquire' ? 'Copy disk' : 'Start scan', enabled: ready };
  });

  /**
   * Connects to the engine and lists what this machine has.
   *
   * The engine runs with the privileges this window already holds, so what is
   * listed here is what a scan will actually be able to read. There is no
   * unprivileged mode to end up in by accident.
   */
  async function connect(): Promise<void> {
    try {
      await ipc.connect();
      await refresh();
    } catch (err) {
      session.problem = String(err);
    }
  }

  /**
   * Re-reads what this machine has, for a disk attached or removed while the
   * window was open.
   *
   * A selection that no longer exists is dropped rather than carried: starting
   * a scan against a path the machine no longer publishes would fail at the
   * open, and it is better to notice here.
   */
  async function refresh(): Promise<void> {
    refreshing = true;
    try {
      // Whole disks only. A scan of one partition cannot see the partition
      // table, the space between partitions, or the residue an earlier
      // filesystem left behind, and this tool exists to read the whole medium.
      const inventory = await ipc.devices();
      devices = inventory.devices.filter((device) => device.kind === 'disk');
      if (source !== '' && !devices.some((device) => device.path === source)) {
        source = '';
      }
    } catch (err) {
      session.problem = String(err);
    } finally {
      refreshing = false;
    }
  }

  async function start(): Promise<void> {
    busy = true;
    session.problem = '';
    session.phase = 'connecting';
    try {
      // The settings panel builds this, and it opens on the engine's own
      // defaults — so a window nobody has touched runs exactly what
      // `argos scan <source> --out <destination>` runs, and one that has been
      // touched runs the same scan the equivalent flags would (`A-CLI-FIRST`).
      // Every field is a field of `ScanRequest`; nothing is decided here.
      if (job === 'acquire') {
        // Copying does not scan what it copied. Two jobs, asked for one at a
        // time, so the person choosing to read a failing disk exactly once is
        // not then made to read it again by the same button.
        const copy = await ipc.acquireStart(source, imagePath);
        session.begin(copy.source, 'acquire');
        return;
      }
      const started = await ipc.scanStart(settings.request(source, destination));
      session.begin(started.source);
    } catch (err) {
      session.problem = String(err);
      session.phase = 'failed';
    } finally {
      busy = false;
    }
  }

  async function stop(): Promise<void> {
    busy = true;
    // Set before the call, not after it: the engine stops at the next artifact
    // boundary and the run can go on for a moment afterwards, and a screen
    // that says nothing in that moment is a screen the user presses again.
    session.stopping = true;
    try {
      await ipc.scanCancel();
    } catch (err) {
      session.problem = String(err);
      session.stopping = false;
    } finally {
      busy = false;
    }
  }

  /**
   * Suspends the run, or lets it carry on.
   *
   * Nothing recovered is discarded either way and the medium stays open, so
   * this is not a smaller Cancel: it is how a scan that runs for hours gives
   * the machine back without losing the hours. Which way it goes is read from
   * the engine's own state, never from a flag this window keeps.
   */
  /**
   * Closes for real, having been told to.
   *
   * `destroy` rather than `close`: a close is a request, and this window
   * answers requests with the very dialog that produced this call.
   */
  function quit(): void {
    quitAsking = false;
    void getCurrentWindow().destroy();
  }

  async function suspend(): Promise<void> {
    busy = true;
    try {
      await (session.paused ? ipc.scanResume() : ipc.scanPause());
    } catch (err) {
      session.problem = String(err);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    void remembered().then(apply);
    // The panel opens on whatever was last chosen. Read once, here, so the
    // button and the panel cannot disagree about what the next scan will run.
    void settings.restore();

    let unlisten: (() => void) | undefined;
    void subscribe().then((stop) => {
      unlisten = stop;
    });
    void connect();

    // Every way out arrives here: the caption's X, the window menu, Alt+F4.
    // A recovery is hours of reading a disk that may not survive being read
    // again, and one stray press must not be able to end it.
    let unclose: (() => void) | undefined;
    void getCurrentWindow()
      .onCloseRequested((event) => {
        // Left unprevented, the close goes through as it always has: nothing
        // is running, so there is nothing to lose by letting the window go.
        if (!session.running) return;
        event.preventDefault();
        quitAsking = true;
      })
      .then((off) => {
        unclose = off;
      });

    const tick = setInterval(() => {
      if (session.phase === 'scanning') session.now = Date.now();
    }, TICK_MS);

    return () => {
      unlisten?.();
      unclose?.();
      clearInterval(tick);
    };
  });
</script>

<div class="window">
  <TitleBar />

  <main>
    <div class="form">
      <DriveTable
        {devices}
        {refreshing}
        selected={source}
        disabled={session.running}
        onSelect={(path) => (source = path)}
        onRefresh={() => void refresh()}
        onConfig={() => (settingsOpen = true)}
      />
      <!--
        The two things this tool does to a disk. A choice rather than two
        screens: the drive and the destination are the same two questions
        either way, and only the words on them change.
      -->
      <div class="job" role="group" aria-label="What to do with this drive">
        <button
          type="button"
          class:on={job === 'scan'}
          aria-pressed={job === 'scan'}
          disabled={session.running}
          onclick={() => (job = 'scan')}
        >
          <span class="what">Recover images</span>
          <span class="why">Reads the drive and writes back what it finds.</span>
        </button>
        <button
          type="button"
          class:on={job === 'acquire'}
          aria-pressed={job === 'acquire'}
          disabled={session.running}
          onclick={() => (job = 'acquire')}
        >
          <span class="what">Copy to an image</span>
          <span class="why">
            Reads the drive once into a file, so nothing has to read it again.
          </span>
        </button>
      </div>
      <Destination
        value={target}
        kind={job === 'acquire' ? 'image' : 'folder'}
        disabled={session.running}
        onChange={(path) => (job === 'acquire' ? (imagePath = path) : (destination = path))}
      />
    </div>

    <Activity />

    <div class="controls">
      <!--
        Only while a run is under way, and never once a stop has been asked
        for: there is nothing to suspend once the engine is winding down.
      -->
      <!--
        A copy has no stages to suspend between, so it takes the one control
        that means something for it: stop.
      -->
      {#if session.running && !session.stopping && session.job === 'scan'}
        <button class="action secondary" disabled={busy} onclick={suspend}>
          {session.paused ? 'Resume' : 'Pause'}
        </button>
      {/if}
      <button class="action" disabled={!action.enabled} onclick={session.running ? stop : start}>
        {action.label}
      </button>
    </div>
  </main>
</div>

{#if quitAsking}
  <QuitDialog job={session.job} onStay={() => (quitAsking = false)} onQuit={quit} />
{/if}

{#if settingsOpen}
  <SettingsDialog
    active={active.id}
    onChoose={(id) => void apply(id)}
    onClose={() => (settingsOpen = false)}
  />
{/if}

<style>
  /*
   * The frame. The system draws no decorations, so the edge, the corner and
   * the shadow around this application are these rules and nothing else.
   *
   * Two lines, not one: the border is the dark edge against the desktop and
   * the overlay below it is the bright line just inside — which is how a
   * window of one generation is drawn and, at a hairline width, exactly how a
   * window of another is.
   */
  /*
   * The layout's own scale.
   *
   * Every measure that dominates the window's height is derived from the
   * window's height, with a floor and a ceiling: the blocks shrink together as
   * the window does, so the content fits at the smallest size the window can
   * take and nothing ever has to scroll. `vh` rather than a media query
   * because the frame is continuous — there is no size at which the layout
   * becomes a different layout.
   *
   * These are the layout's, not a theme's. A theme decides colour, edge,
   * corner, texture and typeface; where a control sits and how big it is are
   * the same under all of them.
   */
  .window {
    --block-gap: clamp(0.5rem, 1.45vh, 1.1rem);
    --pane-pad: clamp(0.65rem, 1.6vh, 1.2rem);
    --row-height: clamp(2rem, 3.5vh, 2.7rem);
    --ring-size: clamp(4.6rem, 14.5vh, 9.6rem);
    --ring-gap: clamp(1.5rem, 7vw, 5.5rem);

    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    background-color: var(--backdrop);
    background-image: var(--backdrop-noise), var(--backdrop-glow);
    border: 1px solid var(--window-edge);
    border-radius: var(--window-radius);
    box-shadow: var(--window-shadow);
    /* The corner has to cut what is inside it, or a pane's own corner would
       show through the frame's. */
    overflow: hidden;
  }

  /* The bright line inside the dark one. Its own element because a theme with
     no shadow cannot have one appended to `none`. */
  .window::before {
    content: '';
    position: absolute;
    inset: 0;
    z-index: 3;
    pointer-events: none;
    border-radius: inherit;
    box-shadow: inset 0 0 0 1px var(--window-border);
  }

  /* The client area, inside the frame band. On a theme whose frame is a
     hairline this is the whole window and nothing shows; on one with a wide
     translucent border it is the sheet the controls sit on, and the band
     around it is the frame. */
  main {
    flex: 1;
    min-height: 0;
    /* Nothing here scrolls. The blocks have fixed heights and the activity
       block takes whatever is left over, so a warning or an error arriving
       cannot move them. */
    overflow: hidden;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--block-gap);
    margin: 0 var(--window-inset) var(--window-inset);
    padding: var(--pane-pad) clamp(0.9rem, 3vw, 2.3rem);
    background: var(--main-surface);
    border: 1px solid var(--main-border);
    border-radius: var(--main-radius);
    box-shadow: var(--main-shadow);
  }

  /* Whether choosing what to read and choosing where to write are one sheet or
     two separate controls is the theme's call, so this carries no appearance
     of its own beyond what a theme gives it. */
  .form {
    width: 100%;
    max-width: 70rem;
    /* Never shrinks. Its height is fixed by the drive table's, and a sheet that
       gave ground would clip a row rather than hide one. */
    flex: none;
    display: flex;
    flex-direction: column;
    gap: var(--block-gap);
    padding: var(--pane-pad) clamp(0.8rem, 2vw, 1.375rem);
    background: var(--scanlines), var(--form-pane);
    border: 1px solid var(--form-pane-border);
    border-radius: var(--pane-radius);
    box-shadow: var(--form-pane-shadow);
  }

  /* Two cards, equal weight: neither job is the advanced one. */
  .job {
    display: flex;
    gap: 0.5rem;
  }

  .job button {
    position: relative;
    display: flex;
    flex: 1;
    min-width: 0;
    flex-direction: column;
    gap: 0.14rem;
    padding: 0.6rem 0.7rem;
    text-align: left;
    font: inherit;
    color: var(--text);
    background: var(--inset);
    border: 1px solid var(--inset-border);
    border-radius: var(--input-radius);
    box-shadow: var(--inset-shadow);
    cursor: pointer;
  }

  /* The band of light across the top of anything that can be pressed — on the
     card that is chosen, and on that one only. On the other the fill is flat:
     the same token the drive table's own ground is drawn with, so the two can
     never drift apart, and what separates chosen from not is colour and edge
     rather than texture. */
  .job button::after {
    content: '';
    position: absolute;
    inset: 0;
    pointer-events: none;
    border-radius: inherit;
    background: none;
  }

  .job button.on::after {
    background: var(--specular);
  }

  .job button:hover:not(:disabled) {
    background: var(--row-hover);
    border-color: var(--inset-border-hover);
  }

  .job button:active:not(:disabled) {
    background: var(--button-active);
    box-shadow: var(--button-shadow-active);
  }

  .job button.on {
    background: var(--row-selected);
    border-color: var(--row-selected-border);
    box-shadow: var(--row-selected-shadow);
  }

  .job button:disabled {
    cursor: default;
    color: var(--disabled-text);
    opacity: var(--disabled-opacity);
  }

  .job .what {
    font-size: var(--type-sm);
  }

  /* Two lines' worth whether the typeface takes one or two: a theme changes
     the face, never where a control sits (`themes/contract.ts`). */
  .job .why {
    font-size: var(--type-2xs);
    line-height: 1.35;
    min-height: 2.7em;
    color: var(--text-dim);
  }

  /* The button row. `main` centres its children, so this keeps the pair
     centred as one unit rather than stacking them. */
  .controls {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }

  /*
   * Two of the three kinds of button this interface has, side by side.
   *
   * The one that runs the job wears the theme's action fill; suspending a run
   * is a smaller decision than starting or abandoning one, so it wears the
   * ordinary one — and the layout says so with the fill and the width rather
   * than leaving two identical buttons to be told apart by their words.
   */
  .action {
    position: relative;
    flex: none;
    width: 16.9rem;
    padding: 0.94rem 0;
    font-family: var(--font);
    font-size: var(--type-lg);
    color: var(--action-text);
    background: var(--action);
    text-shadow: var(--text-glow);
    border: 1px solid var(--action-border);
    border-radius: var(--button-radius);
    box-shadow: var(--action-shadow);
    cursor: pointer;
  }

  .action::after {
    content: '';
    position: absolute;
    inset: 0;
    pointer-events: none;
    border-radius: inherit;
    background: var(--specular);
  }

  .action:hover:not(:disabled) {
    background: var(--action-hover);
    color: var(--action-text-hover);
  }

  .action:active:not(:disabled) {
    background: var(--action-active);
    color: var(--action-text-hover);
    box-shadow: var(--action-shadow-active);
  }

  /* A button that cannot be pressed gives up the accent fill entirely and
     wears the ordinary surface instead. Keeping the fill and dimming the ink
     is what made the label vanish: pale grey text at half opacity over a
     saturated blue reads at a contrast of one to one, and the button still
     looks like the thing you are meant to press. */
  .action:disabled {
    color: var(--disabled-text);
    background: var(--button);
    border-color: var(--button-border);
    cursor: not-allowed;
    box-shadow: none;
  }

  /* The gloss goes with the fill. */
  .action:disabled::after {
    background: none;
  }

  .secondary {
    width: 9.4rem;
    color: var(--button-text);
    background: var(--button);
    border-color: var(--button-border);
    box-shadow: var(--button-shadow);
  }

  .secondary:hover:not(:disabled) {
    background: var(--button-hover);
    border-color: var(--button-border-hover);
    color: var(--button-text-hover);
  }

  .secondary:active:not(:disabled) {
    background: var(--button-active);
    box-shadow: var(--button-shadow-active);
    color: var(--button-text-hover);
  }
</style>
