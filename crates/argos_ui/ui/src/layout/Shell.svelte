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

  import type { Device } from '../lib/dto';
  import * as ipc from '../lib/ipc';
  import { session, subscribe } from '../lib/session.svelte';
  import { active, apply, remembered } from '../themes/active.svelte';

  import TitleBar from './parts/TitleBar.svelte';
  import DriveTable from './parts/DriveTable.svelte';
  import Destination from './parts/Destination.svelte';
  import Activity from './parts/Activity.svelte';
  import ThemeDialog from './parts/ThemeDialog.svelte';

  /** How often the elapsed clock advances while a scan runs. */
  const TICK_MS = 500;

  let devices = $state<Device[]>([]);
  let source = $state('');
  let destination = $state('');
  let busy = $state(false);
  let refreshing = $state(false);
  let themeOpen = $state(false);

  const ready = $derived(source !== '' && destination !== '' && !busy);

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
        : { label: 'Start scan', enabled: ready },
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
      // The button runs exactly what `argos scan <source> --out <destination>`
      // runs, with no options of its own. A window that could ask for a
      // different recovery than the command line would be a second interface
      // to the engine rather than a view of it (`A-CLI-FIRST`).
      const started = await ipc.scanStart({
        source,
        out: destination,
        jobs: null,
        filesystem: true,
        carving: true,
        reassembly: true,
        triage: true,
        // The engine's own floor, not a number this window invented: a used
        // disk holds far more cache entries and icons than photographs, and
        // they are small. Everything under it is still examined, hashed and
        // recorded with its extents and dimensions — what changes is that it
        // does not fill the destination folder. This is the same run as
        // `argos scan … ` with no `--min-long-side` given.
        minLongSide: null,
        previews: false,
      });
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

  onMount(() => {
    void remembered().then(apply);

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
        onConfig={() => (themeOpen = true)}
      />
      <Destination
        value={destination}
        disabled={session.running}
        onChange={(path) => (destination = path)}
      />
    </div>

    <Activity />

    <button class="action" disabled={!action.enabled} onclick={session.running ? stop : start}>
      {action.label}
    </button>
  </main>
</div>

{#if themeOpen}
  <ThemeDialog
    active={active.id}
    onChoose={(id) => void apply(id)}
    onClose={() => (themeOpen = false)}
  />
{/if}

<style>
  /* The frame. The system draws no decorations, so the edge, the corner and
     the shadow around this application are this rule and nothing else. */
  .window {
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
    gap: 1.1rem;
    margin: 0 var(--window-inset) var(--window-inset);
    padding: 1.35rem 2.3rem 1.5rem;
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
    gap: 1.1rem;
    padding: 1.2rem 1.375rem;
    background: var(--scanlines), var(--form-pane);
    border: 1px solid var(--form-pane-border);
    border-radius: var(--pane-radius);
    box-shadow: var(--form-pane-shadow);
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
