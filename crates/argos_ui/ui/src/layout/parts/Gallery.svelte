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
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-dialog';

  import * as ipc from '../../lib/ipc';
  import type { Artifact } from '../../lib/dto';

  let {
    session,
    previewDir,
  }: {
    /** Session directory the finished scan wrote. */
    session: string;
    /** Absolute path of that session's previews directory. */
    previewDir: string;
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

  $effect(() => {
    if (session !== '') void load(true);
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

  .filters button {
    padding: 0.3rem 0.7rem;
    border: 1px solid var(--inset-border);
    border-radius: 0.4rem;
    background: var(--inset);
    color: var(--text-dim);
    font: inherit;
    font-size: 0.85em;
    cursor: pointer;
    text-shadow: var(--text-glow);
  }

  .filters button:hover:not(:disabled) {
    background: var(--row-hover);
    color: var(--text);
  }

  .filters button.on {
    background: var(--row-selected);
    border-color: var(--row-selected-border);
    color: var(--text);
  }

  .filters button:disabled {
    cursor: default;
    opacity: 0.6;
  }

  .count,
  .empty,
  .problem {
    margin: 0;
    color: var(--text-dim);
    font-size: 0.85em;
    text-shadow: var(--text-glow);
  }

  /* What the last export produced, kept until another one replaces it: a copy
     that refused an artifact has to stay on screen long enough to be read. */
  .exported {
    margin: 0;
    color: var(--text-dim);
    font-size: 0.85em;
    text-shadow: var(--text-glow);
  }

  .export {
    padding: 0.3rem 0.7rem;
    font: inherit;
    font-size: 0.78rem;
    color: var(--text);
    background: var(--inset);
    border: 1px solid var(--inset-border);
    border-radius: var(--radius);
    cursor: pointer;
  }

  .export:hover:not(:disabled) {
    background: var(--row-hover);
  }

  .export:disabled {
    color: var(--text-faint);
    cursor: default;
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
    border-radius: 0.4rem;
    padding: 0.35rem;
    overflow: hidden;
  }

  img,
  .nopreview {
    width: 100%;
    aspect-ratio: 4 / 3;
    object-fit: contain;
    background: var(--track);
    border-radius: 0.25rem;
    display: block;
  }

  figcaption {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    font-size: 0.72em;
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
    border: 1px solid var(--action-border);
    border-radius: 0.4rem;
    background: var(--action);
    color: var(--action-text);
    font: inherit;
    cursor: pointer;
  }

  .more:disabled {
    cursor: default;
    opacity: 0.7;
  }
</style>
