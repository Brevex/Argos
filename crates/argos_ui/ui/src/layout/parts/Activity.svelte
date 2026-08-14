<script lang="ts">
  /**
   * Block three: what the engine is doing, as it does it.
   *
   * Every figure below has one source, and none of them are estimates except
   * the one labelled as one:
   *
   * - **Images recovered** and **Data recovered** are the engine's own counts
   *   of artifacts that reached the output directory. They are *not* candidate
   *   counts. A signature hit that has not passed its format's state machine
   *   is not a recovery, and this pipeline validates after it sweeps — so both
   *   sit at zero while the surface is being read, and that is the truth about
   *   the run rather than a gap to fill (`A-CONFIDENCE-HONEST`).
   *   A run told to leave synthetic assets unwritten records every one of them
   *   in the manifest, and neither figure counts those: they describe the
   *   destination folder, and how many were recorded without being written is
   *   said in words beside the two rings.
   * - **Data analyzed** is bytes read off the medium.
   * - **Elapsed** is a clock.
   * - **Remaining** is arithmetic on the read rate, shown with `≈` because it
   *   is a guess about time and nothing else.
   */
  import { bytes, count, duration } from '../../lib/format';
  import { session } from '../../lib/session.svelte';
  import Ring from './Ring.svelte';

  const status = $derived.by(() => {
    switch (session.phase) {
      case 'connecting':
        return 'Starting the recovery engine…';
      case 'scanning': {
        // A stop was asked for. The engine finishes the artifact in flight,
        // writes it and writes the manifest, and saying so is what makes the
        // wait legible instead of looking like a button that did nothing.
        if (session.stopping) {
          return session.job === 'acquire'
            ? 'Stopping — what has been copied so far stays in the image'
            : 'Stopping — finishing the image being written, then the manifest';
        }
        // The stage, named, because a run spends most of its time in passes
        // that are not the read: a screen that only ever said "Scanning" while
        // candidates were being validated for ten minutes would look stalled.
        if (session.doing === '') return 'Starting…';
        const percent = session.doneOfStage;
        return percent === null ? `${session.doing}…` : `${session.doing} — ${percent}%`;
      }
      case 'done': {
        // An acquisition recovered no images and examined no candidates; it
        // copied sectors. Saying otherwise would describe a job that did not
        // happen (`A-CONFIDENCE-HONEST`).
        if (session.job === 'acquire') {
          const copy = session.acquired;
          if (copy === null) return 'Finished — the disk was copied';
          if (copy.complete) return 'Finished — every sector was read into the image';
          // Two different facts, never merged: a run stopped by its operator
          // says nothing about the medium, and reporting it as damage would be
          // a false account of the disk (`A-CONFIDENCE-HONEST`).
          const said = [];
          if (copy.stoppedEarly) {
            said.push(
              `Stopped — ${count(copy.notAttempted)} sectors were never read, and the image ends where the copy did`,
            );
          }
          if (copy.unreadableRegions > 0) {
            said.push(
              `${count(copy.unreadableRegions)} runs of sectors could not be read and are zero-filled in the image`,
            );
          }
          return said.length > 0 ? said.join('. ') : 'Finished — the disk was copied';
        }
        const examined = 'Finished — every candidate on the medium was examined';
        // What was left unwritten is said here or nowhere. The manifest in the
        // destination folder records every one of them with its extents and
        // digest, and a viewer who is not told the count has no way to know
        // there is anything in it beyond the files they can see.
        if (session.omitted === 0) return examined;
        return `${examined}. ${count(session.omitted)} system assets were recorded in the manifest and not written`;
      }
      case 'cancelled':
        return 'Stopped early — what had been recovered was written, the rest was not examined';
      case 'failed':
        return session.job === 'acquire' ? 'The copy failed' : 'The scan failed';
      default:
        return 'Select a drive and a destination';
    }
  });

  const remaining = $derived(session.remaining);
</script>

<section>
  <p
    class="status"
    class:live={session.phase === 'scanning'}
    class:done={session.phase === 'done'}
  >
    {status}
  </p>

  <div class="rings">
    {#if session.job === 'acquire'}
      <Ring label="Copy" fraction={session.scanned} />
    {:else}
      <Ring label="Scan" fraction={session.scanned} />
      <Ring label="Recovery" fraction={session.recovered} />
    {/if}
  </div>

  <dl class="stats">
    {#if session.job === 'acquire'}
      <div>
        <dt>Sectors copied</dt>
        <dd>{count(session.acquired?.recovered ?? session.work.done)}</dd>
      </div>
      <div>
        <dt>Sectors on the disk</dt>
        <dd>{count(session.acquired?.sectors ?? session.work.total)}</dd>
      </div>
      <div>
        <dt>Unreadable runs</dt>
        <dd>{count(session.acquired?.unreadableRegions ?? 0)}</dd>
      </div>
      <div>
        <dt>Never read</dt>
        <dd>{count(session.acquired?.notAttempted ?? 0)}</dd>
      </div>
    {:else}
      <div>
        <dt>Images recovered</dt>
        <dd>{count(session.artifacts)}</dd>
      </div>
      <div>
        <dt>Data analyzed</dt>
        <dd>{bytes(session.sweep.done)}</dd>
      </div>
      <div>
        <dt>Data recovered</dt>
        <dd>{bytes(session.stored)}</dd>
      </div>
    {/if}
    <div>
      <dt>Elapsed</dt>
      <dd>{duration(session.elapsed)}</dd>
    </div>
    <div>
      <dt>Remaining</dt>
      <dd>{remaining === null ? '—' : `≈ ${duration(remaining)}`}</dd>
    </div>
  </dl>

  {#if session.warnings.length > 0 || session.problem}
    <div class="messages">
      {#if session.warnings.length > 0}
        <ul class="warnings">
          {#each session.warnings as warning, index (index)}
            <li>{warning}</li>
          {/each}
        </ul>
      {/if}

      {#if session.problem}
        <p class="problem">{session.problem}</p>
      {/if}
    </div>
  {/if}
</section>

<style>
  section {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    /* Takes every pixel the fixed blocks left, which is what keeps them still
       when a message appears: the room a warning or an error needs comes from
       here, not from the drive table. */
    flex: 1 1 auto;
    min-height: 0;
    width: 100%;
    /* A block squeezed to nothing centres its content across its own edges and
       paints over what is above it. Clipping is the difference between a tight
       window and a broken one. */
    overflow: hidden;
  }

  /* The one part of the screen that gives way. On a window too short to show a
     long message whole, it scrolls; nothing else moves. */
  .messages {
    flex: 0 1 auto;
    min-height: 0;
    overflow-y: auto;
    width: 100%;
    max-width: 56.25rem;
  }

  .status {
    margin: 0 0 1rem;
    font-size: 1.06rem;
    color: var(--text-dim);
    text-shadow: var(--text-glow);
    text-align: center;
  }

  .status.live {
    color: var(--text);
  }

  .status.done {
    color: var(--ok);
  }

  .rings {
    display: flex;
    gap: var(--ring-gap);
    margin-bottom: var(--block-gap);
  }

  .stats {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    width: 100%;
    max-width: 56.25rem;
    margin: 0;
    padding: clamp(0.5rem, 1.3vh, 0.9rem) 0;
    background: var(--scanlines), var(--pane);
    backdrop-filter: var(--pane-blur);
    -webkit-backdrop-filter: var(--pane-blur);
    border: 1px solid var(--pane-border);
    border-radius: var(--pane-radius);
    box-shadow: var(--pane-shadow);
  }

  .stats > div {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.375rem;
    padding: 0 0.75rem;
    min-width: 0;
  }

  /* The rule between two figures stops short of the box, so the strip reads
     as one panel with divisions rather than as five boxes in a row. */
  .stats > div + div {
    position: relative;
  }

  .stats > div + div::before {
    content: '';
    position: absolute;
    left: 0;
    top: 0.35rem;
    bottom: 0.35rem;
    border-left: 1px solid var(--pane-border);
  }

  dt {
    font-size: 0.84rem;
    color: var(--text-dim);
    text-align: center;
  }

  dd {
    margin: 0;
    font-size: 1.56rem;
    text-shadow: var(--text-glow);
    font-weight: 300;
    letter-spacing: -0.01em;
    color: var(--text);
    white-space: nowrap;
  }

  .warnings {
    list-style: none;
    margin: 0.75rem 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }

  /* No coloured edge, on either kind of message. A stripe down the side of a
     box is decoration standing in for what the sentence already says, and it
     is the first thing that reads as an alarm on a screen a user is watching
     for minutes at a time. */
  .warnings li {
    padding: 0.56rem 0.875rem;
    border-radius: var(--radius);
    background: var(--pane);
    font-size: 0.78rem;
    color: var(--text-dim);
  }

  .problem {
    margin: 1rem 0 0;
    padding: 0.56rem 0.875rem;
    border-radius: var(--radius);
    background: var(--pane);
    font-size: 0.78rem;
    color: var(--danger);
  }
</style>
