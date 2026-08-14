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
  import { open } from '@tauri-apps/plugin-dialog';

  import type { Device } from '../lib/dto';
  import * as ipc from '../lib/ipc';
  import { session, subscribe } from '../lib/session.svelte';
  import { settings } from '../lib/settings.svelte';
  import { active, apply, remembered } from '../themes/active.svelte';

  import TitleBar from './parts/TitleBar.svelte';
  import DriveTable from './parts/DriveTable.svelte';
  import Destination from './parts/Destination.svelte';
  import Activity from './parts/Activity.svelte';
  import Gallery from './parts/Gallery.svelte';
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
  /** Session directory of the run that finished, and its previews folder. */
  let finished = $state<{ session: string; previewDir: string } | null>(null);
  /** The same, for the run in progress: promoted once the engine says done. */
  let pending: { session: string; previewDir: string } | null = null;

  const target = $derived(job === 'acquire' ? imagePath : destination);
  const ready = $derived(source !== '' && target !== '' && !busy);

  /**
   * The single button: what it says, and whether it can be pressed.
   *
   * Once a stop has been asked for there is nothing left to press: the engine
   * has been told, and it stops between two artifacts rather than instantly.
   * Saying so is the difference between a button that is working and a button
   * that looks ignored.
   */
  const action = $derived(
    session.stopping
      ? { label: 'Stopping…', enabled: false }
      : session.running
        ? { label: 'Cancel', enabled: !busy }
        : { label: job === 'acquire' ? 'Copy disk' : 'Start scan', enabled: ready },
  );

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
        finished = null;
        session.begin(copy.source, 'acquire');
        return;
      }
      const started = await ipc.scanStart(settings.request(source, destination));
      finished = null;
      session.begin(started.source);
      // Kept from the moment the engine names them: the results view needs the
      // session directory and its previews folder, and asking for them again
      // after the run would be a second source for one fact.
      pending = { session: started.out, previewDir: started.previewDir };
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

  /**
   * Searches the finished session's fragmentation points again.
   *
   * The sweep and the validation pass are what those points cost to find, and
   * the manifest kept them — so this reads the medium for the extents it
   * reports and nothing else. Worth doing with a longer budget, which is why
   * the setting for it is in the panel.
   */
  async function searchAgain(): Promise<void> {
    if (finished === null) return;
    const start = await ipc.invokerHome().catch(() => '');
    const out = await open({
      directory: true,
      multiple: false,
      title: 'Write the newly reassembled images into',
      defaultPath: start === '' ? undefined : start,
    });
    if (typeof out !== 'string') return;

    busy = true;
    session.problem = '';
    session.phase = 'connecting';
    try {
      const request = { ...settings.request(source, out), resumeFrom: finished.session };
      const started = await ipc.scanStart(request);
      finished = null;
      session.begin(started.source);
      pending = { session: started.out, previewDir: started.previewDir };
    } catch (err) {
      session.problem = String(err);
      session.phase = 'failed';
    } finally {
      busy = false;
    }
  }

  // A run that ends — finished or stopped early — has a session directory
  // worth showing. Both write a manifest, so both have results to read.
  $effect(() => {
    const ended =
      session.phase === 'done' || session.phase === 'cancelled' || session.phase === 'failed';
    if (ended && pending !== null) {
      finished = pending;
      pending = null;
    }
  });

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

    const tick = setInterval(() => {
      if (session.phase === 'scanning') session.now = Date.now();
    }, TICK_MS);

    return () => {
      unlisten?.();
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

    {#if finished !== null}
      <Gallery
        session={finished.session}
        previewDir={finished.previewDir}
        onSearchAgain={source === '' ? undefined : () => void searchAgain()}
      />
    {/if}

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

{#if settingsOpen}
  <SettingsDialog
    active={active.id}
    onChoose={(id) => void apply(id)}
    onClose={() => (settingsOpen = false)}
  />
{/if}

<style>
  /* The frame. The system draws no decorations, so the edge, the corner and
     the shadow around this application are this rule and nothing else. */
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
    background: var(--backdrop);
    background-image: var(--backdrop-glow);
    border: 1px solid var(--window-border);
    border-radius: var(--window-radius);
    box-shadow: var(--window-shadow);
    /* The corner has to cut what is inside it, or a pane's own corner would
       show through the frame's. */
    overflow: hidden;
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
    border-radius: var(--radius);
    cursor: pointer;
  }

  .job button:hover:not(:disabled) {
    background: var(--row-hover);
  }

  .job button.on {
    background: var(--row-selected);
    border-color: var(--row-selected-border);
  }

  .job button:disabled {
    cursor: default;
    opacity: 0.6;
  }

  .job .what {
    font-size: 0.82rem;
  }

  /* Two lines' worth whether the typeface takes one or two: a theme changes
     the face, never where a control sits (`themes/contract.ts`). */
  .job .why {
    font-size: 0.7rem;
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

  /* Subordinate to the main button, and narrower: suspending a run is a
     smaller decision than starting or abandoning one, and the layout says so
     rather than leaving two equal buttons to be told apart by their words. */
  .secondary {
    width: 9.4rem;
    color: var(--text);
    background: transparent;
    border-color: var(--pane-border);
    box-shadow: none;
  }

  .action {
    flex: none;
    width: 16.9rem;
    padding: 0.94rem 0;
    font-family: var(--font);
    font-size: 0.97rem;
    color: var(--action-text);
    background: var(--action);
    text-shadow: var(--text-glow);
    border: 1px solid var(--action-border);
    border-radius: var(--radius);
    box-shadow: var(--action-shadow);
    cursor: pointer;
  }

  .action:hover:not(:disabled) {
    filter: brightness(1.08);
  }

  .action:disabled {
    opacity: 0.45;
    cursor: not-allowed;
    box-shadow: none;
  }

</style>
