/**
 * The mirror of what the engine has said.
 *
 * Everything in here arrived in a notification or a command reply. Nothing is
 * derived from a recovery: no confidence is computed, no score is compared to
 * a threshold, no artifact is judged. The engine decided all of that, and a
 * second opinion formed in a web view would be a second, unreviewed
 * implementation of forensics (`A-SHELL-NO-DOMAIN`).
 *
 * The two figures a viewer watches — artifacts recovered and bytes recovered —
 * are the engine's own counts of what reached the output directory. They are
 * never candidates seen or signatures matched: a signature hit that has not
 * passed its format's state machine is not a recovery, and showing one as such
 * would overstate the result (`A-CONFIDENCE-HONEST`).
 */

import type { Summary } from './dto';
import { onEngineMessage, type EngineMessage } from './ipc';

/** What the window is doing, as far as the user is concerned. */
export type Phase = 'idle' | 'connecting' | 'scanning' | 'done' | 'cancelled' | 'failed';

/** The stage that reads the medium end to end, counted in bytes. */
const SWEEP_STAGE = 'carve';

/** The stage that writes artifacts out; the "Recovery" figure. */
const REPORT_STAGE = 'report';

/**
 * Stages that run once everything recoverable has been written.
 *
 * Reaching any of them means every stage that examines the medium has ended,
 * which is why the scan figure reads full from here on.
 */
const AFTER_RECOVERY = new Set([REPORT_STAGE, 'preview', 'triage']);

/**
 * What each stage the engine can report is called on screen.
 *
 * A stage the engine names but this table does not is shown by its own name
 * rather than hidden: an unlabelled stage still tells the viewer the run is
 * moving, which is the whole point of announcing one.
 */
const STAGE_NAMES: Record<string, string> = {
  volumes: 'Reading partition tables',
  filesystem: 'Recovering filesystem records',
  carve: 'Reading the medium',
  validation: 'Validating candidates',
  reassembly: 'Reassembling fragmented images',
  report: 'Writing recovered images',
  preview: 'Rendering previews',
  triage: 'Labelling recovered images',
};

/** Progress of one stage, as last reported. */
interface StageProgress {
  done: number;
  total: number;
}

class Session {
  /** Lifecycle, as the engine last reported it. */
  phase = $state<Phase>('idle');

  /** One line naming the source being scanned, as the engine described it. */
  source = $state('');

  /** The stage the engine last said it had begun. */
  stage = $state('');

  /** Progress of that stage, in whatever that stage counts. */
  work = $state<StageProgress>({ done: 0, total: 0 });

  /** Bytes read off the medium by the sweep, and how many there are. */
  sweep = $state<StageProgress>({ done: 0, total: 0 });

  /**
   * How far the stage that writes artifacts has got, in the bytes it has to
   * read back.
   *
   * Progress through that stage's work, which is not the same as the bytes it
   * stored: a finding that duplicates another, one the medium cannot read back
   * and one the run was asked to leave unwritten each cost the stage the same
   * read and none of them reach the directory. Measuring the ring in stored
   * bytes is what left it resting short of the end on runs that had in fact
   * finished. What was stored is [`stored`](Session.stored).
   */
  recovery = $state<StageProgress>({ done: 0, total: 0 });

  /** Bytes actually written to the output directory. */
  stored = $state(0);

  /** Artifacts stored so far. Recoveries, never candidates. */
  artifacts = $state(0);

  /**
   * Artifacts recorded but deliberately not written, once a run has ended.
   *
   * Known only at the end, because it is the engine's count over the manifest
   * rather than anything derived here.
   */
  omitted = $state(0);

  /** Everything the engine warned about, oldest first. */
  warnings = $state<string[]>([]);

  /** Whatever went wrong, when something did. */
  problem = $state('');

  /** When the current run started, for the elapsed clock. */
  startedAt = $state(0);

  /** Ticks while a scan runs, so the elapsed and remaining figures move. */
  now = $state(0);

  /**
   * How far the examination of the medium has got, 0–1.
   *
   * This is the stage in progress, not the run: reading the medium, validating
   * what the read turned up and reassembling fragments are separate passes
   * with separate totals, and one bar over all three would need a made-up
   * exchange rate between bytes and candidates. The label beside it says which
   * pass the figure belongs to.
   */
  get scanned(): number {
    if (AFTER_RECOVERY.has(this.stage)) return 1;
    return fraction(this.work);
  }

  /**
   * How far the stage that writes artifacts has got, 0–1.
   *
   * A run that reached its end reached the end of that stage — including one
   * that found nothing to write, which has no denominator to divide by and
   * would otherwise read as nought per cent forever.
   */
  get recovered(): number {
    if (this.phase === 'done') return 1;
    return fraction(this.recovery);
  }

  /** What the engine is doing, named for a reader. */
  get doing(): string {
    if (this.stage === '') return '';
    return STAGE_NAMES[this.stage] ?? this.stage;
  }

  /** The active stage's percentage, or `null` when it cannot say. */
  get doneOfStage(): number | null {
    if (this.work.total <= 0) return null;
    return Math.round(fraction(this.work) * 100);
  }

  /** Seconds since the run started. */
  get elapsed(): number {
    if (this.startedAt === 0) return 0;
    return Math.max(0, (this.now - this.startedAt) / 1000);
  }

  /**
   * Seconds left in the read of the medium, from the rate it is being read at.
   *
   * Only while that read is the stage running. The passes after it are paced
   * by what the read turned up rather than by the size of the medium, so
   * extrapolating the read's rate across them would be a number with nothing
   * behind it — and a countdown that reached zero while the run continued is
   * exactly how a display stops being believed. `—` is the honest answer.
   */
  get remaining(): number | null {
    const { done, total } = this.sweep;
    if (this.phase !== 'scanning' || this.stage !== SWEEP_STAGE) return null;
    if (total <= 0 || done <= 0) return null;
    const seconds = this.elapsed;
    if (seconds <= 0) return null;
    return (total - done) / (done / seconds);
  }

  /** Whether a run is under way, and so whether the button stops it. */
  get running(): boolean {
    return this.phase === 'scanning' || this.phase === 'connecting';
  }

  /** Resets everything a new run replaces. */
  begin(source: string): void {
    this.phase = 'scanning';
    this.source = source;
    this.stage = '';
    this.work = { done: 0, total: 0 };
    this.sweep = { done: 0, total: 0 };
    this.recovery = { done: 0, total: 0 };
    this.stored = 0;
    this.artifacts = 0;
    this.omitted = 0;
    this.warnings = [];
    this.problem = '';
    this.startedAt = Date.now();
    this.now = this.startedAt;
  }

  /** Folds one engine notification into the mirror. */
  apply(message: EngineMessage): void {
    switch (message.method) {
      case 'stageBegan': {
        const { stage, total } = message.params;
        this.stage = stage;
        this.work = { done: 0, total };
        // The writing stage announces how much it has to get through before it
        // starts, so its ring has a denominator from the first frame rather
        // than after the first artifact.
        if (stage === REPORT_STAGE) this.recovery = { done: 0, total };
        break;
      }
      case 'progress': {
        const { stage, done, total } = message.params;
        if (stage === REPORT_STAGE) {
          this.recovery = { done, total };
          break;
        }
        this.work = { done, total };
        if (stage === SWEEP_STAGE) this.sweep = { done, total };
        break;
      }
      case 'stored':
        this.artifacts = message.params.artifacts;
        this.stored = message.params.bytes;
        break;
      case 'stageDone':
        // A stage that ended covered all of what it said it had, and saying so
        // stops a bar resting a hair short of full because the last batch of
        // work was smaller than the step between two events.
        if (message.params.stage === SWEEP_STAGE && this.sweep.total > 0) {
          this.sweep = { ...this.sweep, done: this.sweep.total };
        }
        if (message.params.stage === REPORT_STAGE && this.recovery.total > 0) {
          this.recovery = { ...this.recovery, done: this.recovery.total };
        }
        if (message.params.stage === this.stage && this.work.total > 0) {
          this.work = { ...this.work, done: this.work.total };
        }
        break;
      case 'state':
        if (message.params.state === 'cancelled') this.phase = 'cancelled';
        break;
      case 'warning':
        this.warnings = [...this.warnings, message.params.text];
        break;
      case 'unreadable':
        break;
      case 'finished':
        this.finish(message.params);
        break;
    }
  }

  /**
   * Takes the final account of a run from the engine's own record.
   *
   * The engine counts and this reads the counts. It used to receive every
   * artifact record and derive them, which on a system disk is tens of
   * thousands of entries parsed and materialized on the thread that draws the
   * window — at the exact moment the scan succeeds.
   */
  private finish(summary: Summary): void {
    // Stop the clock here rather than at the next tick, so the elapsed time
    // shown afterwards is the run's, not the run's plus part of a poll.
    this.now = Date.now();
    this.stage = '';
    this.artifacts = summary.artifacts;
    this.stored = summary.bytes;
    this.omitted = summary.omitted;
    this.phase = summary.state === 'failed'
      ? 'failed'
      : summary.state === 'cancelled'
        ? 'cancelled'
        : 'done';
  }
}

/** A stage's completed fraction, clamped, 0 when it cannot say. */
function fraction({ done, total }: StageProgress): number {
  if (total <= 0) return 0;
  return Math.min(1, Math.max(0, done / total));
}

/** The one mirror the whole window reads. */
export const session = new Session();

/**
 * Starts folding engine notifications into it.
 *
 * Messages are applied a frame at a time rather than one at a time. The engine
 * paces what it sends, so this is not load-bearing — but a window whose
 * responsiveness depends on a well-behaved sender is a window that stops
 * responding the day something changes on the other side. Applying a burst in
 * one batch costs one render instead of one per message.
 */
export function subscribe(): Promise<() => void> {
  let queued: EngineMessage[] = [];
  let frame = 0;

  const drain = (): void => {
    frame = 0;
    const batch = queued;
    queued = [];
    for (const message of batch) session.apply(message);
  };

  return onEngineMessage((message) => {
    queued.push(message);
    if (frame === 0) frame = requestAnimationFrame(drain);
  }).then((unlisten) => () => {
    if (frame !== 0) cancelAnimationFrame(frame);
    unlisten();
  });
}
