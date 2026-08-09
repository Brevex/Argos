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
  import { DEFAULT_THEME, loadTheme } from '../themes';

  import TitleBar from './parts/TitleBar.svelte';
  import DriveTable from './parts/DriveTable.svelte';
  import Destination from './parts/Destination.svelte';
  import Activity from './parts/Activity.svelte';
  import ThemeDialog from './parts/ThemeDialog.svelte';

  /** Where the theme choice is remembered. A view preference, nothing more. */
  const THEME_KEY = 'argos.theme';

  /** How often the elapsed clock advances while a scan runs. */
  const TICK_MS = 500;

  let devices = $state<Device[]>([]);
  let source = $state('');
  let destination = $state('');
  let busy = $state(false);
  let refreshing = $state(false);
  let themeOpen = $state(false);
  let theme = $state(DEFAULT_THEME);

  const ready = $derived(source !== '' && destination !== '' && !busy);

  /** The single button: what it says, and whether it can be pressed. */
  const action = $derived(
    session.running ? { label: 'Cancel', enabled: !busy } : { label: 'Start scan', enabled: ready },
  );

  async function applyTheme(id: string): Promise<void> {
    const module = await loadTheme(id);
    const root = document.documentElement;
    for (const [token, value] of Object.entries(module.tokens)) {
      root.style.setProperty(token, value);
    }
    root.style.colorScheme = module.scheme;
    theme = module.id;
    try {
      localStorage.setItem(THEME_KEY, module.id);
    } catch {
      // A window that cannot persist a preference still has to render.
    }
  }

  /**
   * Connects to the engine and lists what this machine has.
   *
   * Not elevated: enumeration needs no privileges anywhere, and asking for
   * them before the user has chosen anything would be asking for more than the
   * work in front of them needs.
   */
  async function connect(): Promise<void> {
    try {
      await ipc.connect(false);
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
        // Icons, sprites and UI chrome are the bulk of what a system disk
        // gives back, and they are not what anyone is looking for. They are
        // still examined, hashed and recorded in the manifest with their
        // extents — what changes is that their bytes do not fill the
        // destination folder. This is the same run as
        // `argos scan … --exclude-assets`.
        excludeAssets: true,
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
    try {
      await ipc.scanCancel();
    } catch (err) {
      session.problem = String(err);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    let stored = DEFAULT_THEME;
    try {
      stored = localStorage.getItem(THEME_KEY) ?? DEFAULT_THEME;
    } catch {
      // Ignored: the default is a working theme.
    }
    void applyTheme(stored);

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
  <TitleBar onSettings={() => (themeOpen = true)} />

  <main>
    <div class="pane">
      <DriveTable
        {devices}
        {refreshing}
        selected={source}
        disabled={session.running}
        onSelect={(path) => (source = path)}
        onRefresh={() => void refresh()}
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
    active={theme}
    onChoose={(id) => void applyTheme(id)}
    onClose={() => (themeOpen = false)}
  />
{/if}

<style>
  .window {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--backdrop);
    background-image: var(--backdrop-glow);
  }

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
    padding: 0.4rem 2rem 1.1rem;
  }

  /* Blocks one and two share one sheet, as they do in the design: choosing
     what to read and choosing where to write are one decision. */
  .pane {
    width: 100%;
    max-width: 70rem;
    /* Never shrinks. Its height is fixed by the drive table's, and a pane that
       gave ground would clip a row rather than hide one. */
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 1.1rem;
    padding: 1.2rem 1.375rem;
    background: var(--pane);
    backdrop-filter: var(--pane-blur);
    -webkit-backdrop-filter: var(--pane-blur);
    border: 1px solid var(--pane-border);
    border-radius: var(--pane-radius);
    box-shadow: var(--pane-shadow);
  }

  .action {
    flex: none;
    width: 16.9rem;
    padding: 0.94rem 0;
    font-family: var(--font);
    font-size: 0.97rem;
    color: var(--action-text);
    background: var(--action);
    border: 1px solid var(--row-selected-border);
    border-radius: var(--radius);
    box-shadow: 0 0.375rem 1.125rem -0.5rem var(--accent-glow);
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
