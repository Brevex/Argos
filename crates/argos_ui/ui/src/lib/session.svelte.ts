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

import type { Acquired, Summary } from './dto';
import { onEngineMessage, type EngineMessage } from './ipc';

/** What the window is doing, as far as the user is concerned. */
type Phase = 'idle' | 'connecting' | 'scanning' | 'done' | 'cancelled' | 'failed';

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
  // The two passes of an acquisition. Named the same way, because to a person
  // watching they are the same thing: the machine is working through a disk.
  sweep: 'Copying the disk',
  refine: 'Retrying the sectors that failed',
};

/** Progress of one stage, as last reported. */
interface StageProgress {
  done: number;
  total: number;
}

class Session {
  /** Lifecycle, as the engine last reported it. */
  phase = $state<Phase>('idle');

  /**
   * Which job is running: recovering images, or copying the disk to an image.
   *
   * They share a screen and a progress ring because to a person waiting they
   * are the same thing — the machine is working through a disk — but what they
   * produce is different, and what is shown at the end has to say which.
   */
  job = $state<'scan' | 'acquire'>('scan');

  /** What an acquisition produced, once one has finished. */
  acquired = $state<Acquired | null>(null);

  /** One line naming the source being scanned, as the engine described it. */
  source = $state('');

  /** The stage the engine last said it had begun. */
  stage = $state('');

  /** Progress of that stage, in whatever that stage counts. */
  work = $state<StageProgress>({ done: 0, total: 0 });

  /** What `work` is counted in, as the engine named it. */
  workUnit = $state('');

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

  /** Whatever went wrong, when something did. */
  problem = $state('');

  /**
   * Whether a stop has been asked for and not yet taken effect.
   *
   * The engine stops between two artifacts, not instantly: it finishes the one
   * in flight, writes it and writes the manifest. That gap is short on a
   * fixture and not always short on a disk, and a screen that says nothing
   * during it is a screen whose stop button looks broken.
   */
  stopping = $state(false);

  /**
   * Whether the engine has reported the run suspended.
   *
   * Taken from the engine's own lifecycle notification rather than set when the
   * button is pressed: the engine stops at the next chunk boundary, so between
   * the press and the pause the run is still reading. Showing it as paused
   * before it is would be the window inventing a state the engine has not
   * reached (`A-SHELL-NO-DOMAIN`).
   */
  paused = $state(false);

  /** When the current run started, for the elapsed clock. */
  startedAt = $state(0);

  /** Ticks while a scan runs, so the elapsed and remaining figures move. */
  now = $state(0);

  /**
   * How far the read of the medium has got, 0–1.
   *
   * The sweep, for as long as the run lasts — not whichever stage happens to
   * be running. The two are not the same figure, and showing the second under
   * a label that says "Scan" is what put three per cent beside a byte count
   * that said the whole disk had been read: the sweep had finished and a later
   * pass with its own denominator had started.
   *
   * The passes that follow the read are narrated in words beside the arcs,
   * where a percentage of candidates cannot be mistaken for a percentage of
   * the disk. A run with no sweep at all — filesystem records only — has
   * nothing else to show here, and falls back to the stage in progress.
   */
  get scanned(): number | null {
    if (this.sweep.total > 0) return fraction(this.sweep);
    if (AFTER_RECOVERY.has(this.stage)) return 1;
    if (!showsPercentage(this.workUnit)) return null;
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
    if (!showsPercentage(this.workUnit)) return null;
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
  begin(source: string, job: 'scan' | 'acquire' = 'scan'): void {
    this.clear();
    this.job = job;
    this.phase = 'scanning';
    this.source = source;
    this.startedAt = Date.now();
    this.now = this.startedAt;
  }

  /**
   * Puts every figure back where it was before any run.
   *
   * The arcs, the counts, the clock: the state a freshly opened window is in.
   * Both a run starting and a run cancelled go through here, so there is one
   * definition of "nothing has happened yet" rather than two that drift.
   */
  private clear(): void {
    this.acquired = null;
    this.stage = '';
    this.work = { done: 0, total: 0 };
    this.workUnit = '';
    this.sweep = { done: 0, total: 0 };
    this.recovery = { done: 0, total: 0 };
    this.stored = 0;
    this.artifacts = 0;
    this.omitted = 0;
    this.problem = '';
    this.stopping = false;
    this.paused = false;
    this.startedAt = 0;
    this.now = 0;
  }

  /** Folds one engine notification into the mirror. */
  apply(message: EngineMessage): void {
    switch (message.method) {
      case 'stageBegan': {
        const { stage, unit, total } = message.params;
        this.stage = stage;
        this.work = { done: 0, total };
        this.workUnit = unit;
        // The writing stage announces how much it has to get through before it
        // starts, so its ring has a denominator from the first frame rather
        // than after the first artifact.
        if (stage === REPORT_STAGE) this.recovery = { done: 0, total };
        break;
      }
      case 'progress': {
        const { stage, unit, done, total } = message.params;
        if (stage === REPORT_STAGE) {
          this.recovery = { done, total };
          break;
        }
        this.work = { done, total };
        this.workUnit = unit;
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
        // A run is suspended and resumed any number of times, so these are read
        // every time rather than latched.
        if (message.params.state === 'paused') this.paused = true;
        if (message.params.state === 'running') this.paused = false;
        break;
      case 'acquireProgress': {
        const { pass, done, total } = message.params;
        this.stage = pass;
        this.work = { done, total };
        // A copy is measured in sectors all the way through, so it always has
        // a percentage to show.
        this.workUnit = 'items';
        break;
      }
      case 'acquired':
        // Stopped here rather than at the next tick, so the time shown is the
        // copy's and not the copy's plus part of a poll.
        this.now = Date.now();
        this.stage = '';
        this.acquired = message.params;
        this.phase = 'done';
        break;
      // What the engine warns about is a property of the medium, and the drive
      // table reports those from the device record itself — mounted, writable,
      // trimming — beside the drive they belong to. The sentence the engine
      // sends is the same fact in prose, and it goes to the console, where a
      // scan run from the command line is the one that has nowhere else to
      // put it.
      case 'warning':
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
    // A run the operator stopped has no account to leave on screen. Its
    // figures describe a search that was abandoned part-way, and reading them
    // as the result of anything would misstate what the medium holds
    // (`A-CONFIDENCE-HONEST`) — what it did write is in the destination folder
    // and its manifest. So the block goes back to how the window opened, and
    // one line says the recovery was cancelled.
    if (summary.state === 'cancelled') {
      this.clear();
      this.phase = 'cancelled';
      return;
    }
    // Stop the clock here rather than at the next tick, so the elapsed time
    // shown afterwards is the run's, not the run's plus part of a poll.
    this.now = Date.now();
    this.stage = '';
    this.stopping = false;
    this.paused = false;
    this.artifacts = summary.artifacts;
    this.stored = summary.bytes;
    this.omitted = summary.omitted;
    this.phase = summary.state === 'failed' ? 'failed' : 'done';
  }
}

/** A stage's completed fraction, clamped, 0 when it cannot say. */
function fraction({ done, total }: StageProgress): number {
  if (total <= 0) return 0;
  return Math.min(1, Math.max(0, done / total));
}

/**
 * Whether `done` of `total` in this unit is a fraction the window may show as
 * a percentage.
 *
 * False for `steps`, and the engine says so for one stage: reassembly's steps
 * cost anything from seconds to over an hour, and its queue hands the
 * expensive ones out first, so a fraction of them runs far behind the work
 * actually done. A run showing 1.75% had covered the material 77% of the
 * queue's weight sat in, and was stopped for it.
 */
function showsPercentage(unit: string): boolean {
  return unit !== 'steps';
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
