<script lang="ts">
  /**
   * What a finished scan recovered, strongest evidence first.
   *
   * A recovery of a used disk writes hundreds of thousands of artifacts and a
   * few hundred photographs. Measured on the disk this was built for: 47,658
   * files written, 859 of them naming a camera. Without an order the
   * photographs are present and unreachable, which is the failure this view
   * exists to end.
   *
   * The order is the engine's and so is the filter. Nothing here reads a
   * standing, counts an artifact or decides what a photograph is — the names
   * arrive as strings and are shown as strings (`A-SHELL-NO-DOMAIN`). The
   * pictures are the session's `previews/` thumbnails, the one path this
   * window is granted.
   */
  import { untrack } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-dialog';

  import * as ipc from '../../lib/ipc';
  import type { Artifact } from '../../lib/dto';

  let {
    session,
    previewDir,
    onSearchAgain,
  }: {
    /** Session directory the finished scan wrote. */
    session: string;
    /** Absolute path of that session's previews directory. */
    previewDir: string;
    /**
     * Searches this session's fragmentation points again, when the window can.
     * Absent when there is no medium to read them back from.
     */
    onSearchAgain?: (() => void) | undefined;
  } = $props();

  /** Artifacts fetched per page. The engine caps this; this is well under it. */
  const PAGE = 60;

  /**
   * The filters offered, weakest last.
   *
   * The names are the engine's own and are not interpreted here. What the
   * window chooses is which one to ask for, which is a view preference.
   */
  const FILTERS = [
    { standing: 'camera-named', label: 'Naming a camera' },
    { standing: 'dated', label: 'With a capture date' },
    { standing: 'photograph-sized', label: 'Photograph-sized' },
    { standing: null, label: 'Everything recovered' },
  ] as const;

  let chosen = $state<string | null>('camera-named');
  let artifacts = $state<Artifact[]>([]);
  let total = $state(0);
  let recorded = $state(0);
  let loading = $state(false);
  let problem = $state('');

  /** Whether every artifact the filter admits has been fetched. */
  const complete = $derived(artifacts.length >= total);

  async function load(reset: boolean): Promise<void> {
    if (loading) return;
    loading = true;
    problem = '';
    try {
      const offset = reset ? 0 : artifacts.length;
      const page = await ipc.scanGallery(session, offset, PAGE, chosen);
      artifacts = reset ? page.artifacts : [...artifacts, ...page.artifacts];
      total = page.total;
      recorded = page.recorded;
    } catch (err) {
      problem = String(err);
    } finally {
      loading = false;
    }
  }

  function choose(standing: string | null): void {
    chosen = standing;
    void load(true);
  }

  /** What the last export produced, until another one replaces it. */
  let exported = $state('');
  let exporting = $state(false);

  /**
   * Copies out exactly the set this view is showing.
   *
   * The filter is the selection. A person who has narrowed the list to the
   * pictures naming a camera has already said which ones they want, and asking
   * them again in a second dialog is how the two come to disagree.
   *
   * Every copy is verified against the digest the scan recorded; one that no
   * longer reproduces it is reported and not written, which is the whole point
   * of exporting through the engine rather than dragging a folder.
   */
  async function exportShown(): Promise<void> {
    const start = await ipc.invokerHome().catch(() => '');
    const to = await open({
      directory: true,
      multiple: false,
      title: 'Copy the recovered pictures into',
      defaultPath: start === '' ? undefined : start,
    });
    if (typeof to !== 'string') return;

    exporting = true;
    exported = '';
    problem = '';
    try {
      const result = await ipc.exportCopy(session, to, chosen);
      const parts = [`Copied ${result.copied} ${result.copied === 1 ? 'picture' : 'pictures'}`];
      // Reported, never folded into the count: an artifact that failed its
      // digest is the one thing an examiner has to be told about by name.
      if (result.tampered.length > 0) {
        parts.push(`${result.tampered.length} refused — the bytes no longer match the manifest`);
      }
      if (result.missing.length > 0) {
        parts.push(`${result.missing.length} missing from the session folder`);
      }
      exported = parts.join(' · ');
    } catch (err) {
      problem = String(err);
    } finally {
      exporting = false;
    }
  }

  /** The thumbnail for an artifact, or empty when it has none. */
  function thumbnail(artifact: Artifact): string {
    if (!artifact.preview || previewDir === '') return '';
    // `preview` is recorded relative to the session, as `previews/<hash>.jpg`;
    // the granted directory is the previews folder itself.
    const file = artifact.preview.split('/').pop() ?? '';
    return file === '' ? '' : convertFileSrc(`${previewDir}/${file}`);
  }

  // The session is the only thing that should start a fetch. `load` reads
  // `loading` on its way in, and a reactive read there makes the fetch its own
  // trigger: the flag goes up, the effect re-runs, the flag comes down when the
  // reply lands and the effect runs again — a loop that re-fetches the gallery
  // for as long as the window is open. Untracked, the effect depends on the
  // session and nothing else.
  $effect(() => {
    const from = session;
    if (from !== '') untrack(() => void load(true));
  });
</script>

<section class="gallery">
  <header>
    <div class="filters" role="group" aria-label="Which artifacts to show">
      {#each FILTERS as filter (filter.label)}
        <button
          type="button"
          class:on={chosen === filter.standing}
          aria-pressed={chosen === filter.standing}
          disabled={loading}
          onclick={() => choose(filter.standing)}
        >
          {filter.label}
        </button>
      {/each}
    </div>
    <p class="count">
      {#if recorded > 0}
        {total.toLocaleString()} of {recorded.toLocaleString()} recorded
      {/if}
    </p>
    {#if onSearchAgain}
      <!--
        The scan's hours went into locating the fragmentation points, and the
        manifest kept them. Trying a longer budget from those is minutes, not
        another overnight run — so this is a button rather than a rerun.
      -->
      <button class="export" type="button" disabled={loading} onclick={onSearchAgain}>
        Search fragments again…
      </button>
    {/if}
    <button
      class="export"
      type="button"
      disabled={exporting || loading || total === 0}
      onclick={() => void exportShown()}
    >
      {exporting ? 'Copying…' : 'Copy these out…'}
    </button>
  </header>

  {#if problem !== ''}
    <p class="problem">{problem}</p>
  {/if}
  {#if exported !== ''}
    <p class="exported">{exported}</p>
  {/if}

  <div class="grid">
    {#each artifacts as artifact (artifact.sha256)}
      {@const src = thumbnail(artifact)}
      <figure>
        {#if src !== ''}
          <img {src} alt="" loading="lazy" />
        {:else}
          <div class="nopreview" aria-hidden="true"></div>
        {/if}
        <figcaption>
          <span class="name">{artifact.name ?? '(not written)'}</span>
          {#if artifact.width && artifact.height}
            <span class="dim">{artifact.width}&times;{artifact.height}</span>
          {/if}
          {#if artifact.camera}
            <span class="camera">{artifact.camera}</span>
          {/if}
          {#if artifact.taken}
            <span class="taken">{artifact.taken}</span>
          {/if}
          {#if artifact.sameSizeNeighbours}
            <span class="cache"
              >among {artifact.sameSizeNeighbours} of one size</span
            >
          {/if}
        </figcaption>
      </figure>
    {/each}
  </div>

  {#if artifacts.length === 0 && !loading && problem === ''}
    <p class="empty">
      Nothing in this session carries that evidence. A weaker filter shows more;
      everything recovered is in the output folder either way.
    </p>
  {/if}

  {#if !complete}
    <button class="more" type="button" disabled={loading} onclick={() => load(false)}>
      {loading ? 'Loading…' : 'Show more'}
    </button>
  {/if}
</section>

<style>
  .gallery {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    /* Takes the larger share of what is left once a run has ended: the figures
       above have stopped moving by then, and this is what the person is
       looking at. The grid inside scrolls, so it never pushes the window. */
    flex: 2 1 0;
    min-height: 0;
  }

  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 1rem;
    flex-wrap: wrap;
  }

  .filters {
    display: flex;
    gap: 0.25rem;
    flex-wrap: wrap;
  }

  /* The ordinary button, in a row where one of them is held down: the same
     object as Refresh, Browse and Config, and the chosen one wears the same
     selection the drive table's chosen row wears. */
  .filters button,
  .export,
  .more {
    position: relative;
    padding: 0.3rem 0.7rem;
    font: inherit;
    font-size: var(--type-xs);
    color: var(--button-text);
    background: var(--button);
    border: 1px solid var(--button-border);
    border-radius: var(--button-radius);
    box-shadow: var(--button-shadow);
    text-shadow: var(--text-glow);
    cursor: pointer;
  }

  .filters button::after,
  .export::after,
  .more::after {
    content: '';
    position: absolute;
    inset: 0;
    pointer-events: none;
    border-radius: inherit;
    background: var(--specular);
  }

  .filters button:hover:not(:disabled),
  .export:hover:not(:disabled),
  .more:hover:not(:disabled) {
    background: var(--button-hover);
    border-color: var(--button-border-hover);
    color: var(--button-text-hover);
  }

  .filters button:active:not(:disabled),
  .export:active:not(:disabled),
  .more:active:not(:disabled) {
    background: var(--button-active);
    box-shadow: var(--button-shadow-active);
    color: var(--button-text-hover);
  }

  .filters button:disabled,
  .export:disabled,
  .more:disabled {
    color: var(--disabled-text);
    opacity: var(--disabled-opacity);
    cursor: default;
  }

  .filters button:focus-visible,
  .export:focus-visible,
  .more:focus-visible {
    outline: var(--focus-outline);
    outline-offset: var(--focus-offset);
  }

  .filters button.on {
    background: var(--row-selected);
    border-color: var(--row-selected-border);
    box-shadow: var(--row-selected-shadow);
    color: var(--text);
  }

  .count,
  .empty,
  .problem,
  .exported {
    margin: 0;
    max-width: var(--measure);
    color: var(--text-dim);
    font-size: var(--type-xs);
    line-height: 1.5;
    text-shadow: var(--text-glow);
  }

  .problem {
    color: var(--danger);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(9rem, 1fr));
    gap: 0.6rem;
    overflow-y: auto;
    min-height: 0;
    align-content: start;
  }

  figure {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    background: var(--inset);
    border: 1px solid var(--inset-border);
    border-radius: var(--input-radius);
    box-shadow: var(--inset-shadow);
    padding: 0.35rem;
    overflow: hidden;
  }

  img,
  .nopreview {
    width: 100%;
    aspect-ratio: 4 / 3;
    object-fit: contain;
    background: var(--track);
    border-radius: var(--input-radius);
    display: block;
  }

  figcaption {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    font-size: var(--type-2xs);
    color: var(--text-faint);
    text-shadow: var(--text-glow);
    min-width: 0;
  }

  figcaption span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .name {
    color: var(--text);
  }

  .camera,
  .taken {
    color: var(--text-dim);
  }

  .cache {
    color: var(--warn);
  }

  .more {
    align-self: center;
    padding: 0.4rem 1.1rem;
    font-size: var(--type-sm);
  }
</style>
