<script lang="ts">
  /**
   * The question a close asks while a run is under way.
   *
   * A recovery of a disk is hours, and the window has no other way of saying
   * so at the moment it is dismissed: a stray click on the caption's X, an
   * Alt+F4 meant for another window, and the hours are gone with no way back.
   * This is the one place the application refuses to act on a single press.
   *
   * It is deliberately not a smaller Cancel. Cancelling lets the engine finish
   * the artifact in flight and write the manifest; closing does not, and the
   * difference is what the second line exists to say.
   *
   * The window it is drawn in is [`Dialog`], the same one the settings panel
   * uses, so there is one dialog in this application and both are it.
   */
  import Dialog from './Dialog.svelte';

  let {
    job,
    onStay,
    onQuit,
  }: {
    /** Which run is under way, so the wording names the right one. */
    job: 'scan' | 'acquire';
    /** Dismissal: the X, Escape, a click outside, and the safe button. */
    onStay: () => void;
    /** Close anyway, giving up whatever the run had not finished. */
    onQuit: () => void;
  } = $props();

  const noun = $derived(job === 'acquire' ? 'A disk copy' : 'A recovery');
</script>

<Dialog title="Close Argos?" width="30rem" onClose={onStay}>
  <div class="body">
    <p class="lead">{noun} is running.</p>
    <p class="note">
      Closing now ends it where it is. Cancelling first lets the run write what
      it has already found, and the manifest that accounts for it.
    </p>

    <div class="buttons">
      <button class="ordinary" type="button" onclick={onQuit}>Close anyway</button>
      <!-- Focused on open, so the key a dialog is dismissed with by reflex is
           the one that keeps the run. -->
      <!-- svelte-ignore a11y_autofocus -->
      <button class="ordinary keep" type="button" autofocus onclick={onStay}>
        Keep running
      </button>
    </div>
  </div>
</Dialog>

<style>
  /* The dialog's client area, on the settings panel's own measures and tokens:
     the two windows differ in what they hold and in nothing else. */
  .body {
    display: flex;
    flex-direction: column;
    gap: 0.55rem;
    margin: 0 var(--dialog-inset) var(--dialog-inset);
    padding: 1.1rem 1.25rem 1.25rem;
    background: var(--pane);
    border: 1px solid var(--main-border);
    border-radius: var(--main-radius);
    box-shadow: var(--main-shadow);
  }

  .lead {
    margin: 0;
    font-size: var(--type-md);
    color: var(--text);
    text-shadow: var(--text-glow);
  }

  .note {
    margin: 0;
    max-width: var(--measure);
    font-size: var(--type-xs);
    line-height: 1.5;
    color: var(--text-dim);
    text-shadow: var(--text-glow);
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.35rem;
  }

  /* The ordinary button of this application, in the measures a dialog uses.
     Neither of the two wears the action fill: one of them ends a run of hours
     and the other changes nothing, and an accent on either would be the panel
     recommending an answer it has no business recommending. */
  .ordinary {
    position: relative;
    padding: 0.4rem 0.95rem;
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

  .ordinary::after {
    content: '';
    position: absolute;
    inset: 0;
    pointer-events: none;
    border-radius: inherit;
    background: var(--specular);
  }

  .ordinary:hover {
    background: var(--button-hover);
    border-color: var(--button-border-hover);
    color: var(--button-text-hover);
  }

  .ordinary:active {
    background: var(--button-active);
    box-shadow: var(--button-shadow-active);
    color: var(--button-text-hover);
  }

  .ordinary:focus-visible {
    outline: var(--focus-outline);
    outline-offset: var(--focus-offset);
  }

  /* The safe answer is the one the frame points at, in the way the drive table
     marks the row it has chosen — the same selection, not a colour of its own. */
  .keep {
    background: var(--row-selected);
    border-color: var(--row-selected-border);
    box-shadow: var(--row-selected-shadow);
    color: var(--text);
  }
</style>
