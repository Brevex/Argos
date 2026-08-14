/**
 * Which stages the next scan is asked to run, and how much of the machine it
 * may use.
 *
 * Every field here is a field of `ScanRequest`, and the defaults are the
 * engine's own — a window that opened with anything else would be quietly
 * running a different recovery from `argos scan` with no flags (`A-CLI-FIRST`).
 *
 * Nothing here decides anything about a recovery. Turning a stage off is
 * choosing what to ask for; what a stage *does*, whether a combination is
 * allowed and what any of it finds are the engine's, and it stays the authority
 * — an invalid request is refused there, with its reason, rather than being
 * pre-judged here (`A-SHELL-NO-DOMAIN`).
 */

import type { ScanRequest } from './dto';
import { load, save, section } from './preferences';

/** The key this section is stored under in the preference document. */
const STORAGE_KEY = 'recovery';

/**
 * What each stage cost on the measured run: a 1 TB mechanical disk, 12
 * workers, 5 h 31 m in all. Shown beside each switch so the choice is made
 * against a number rather than a guess.
 *
 * One disk is one sample. The order between the stages is dependable; the
 * absolute figures are not, which is why they are shown as approximations.
 */
export const MEASURED_COST: Record<string, string> = {
  filesystem: '≈ 32 min',
  carving: '≈ 2 h',
  reassembly: 'up to 2 h',
  triage: '≈ 3 min',
};

class Settings {
  /** Recover from filesystem metadata. */
  filesystem = $state(true);

  /** Carve the whole surface by signature. */
  carving = $state(true);

  /** Reassemble images the medium stored in pieces. */
  reassembly = $state(true);

  /** Label artifacts photograph vs synthetic asset. */
  triage = $state(true);

  /**
   * Render a thumbnail of every artifact that decodes.
   *
   * On by default because the results gallery draws these, and they are the
   * only part of a session this window is ever granted a path to.
   */
  previews = $state(true);

  /**
   * Smallest long side, in pixels, an image is written to disk for. `null`
   * takes the engine's own floor.
   *
   * Whatever is not written is still examined, hashed and recorded with its
   * extents and dimensions, so the manifest stays a complete account of the
   * medium either way.
   */
  minLongSide = $state<number | null>(null);

  /** Worker threads; `null` takes the machine's available parallelism. */
  jobs = $state<number | null>(null);

  /** Whether anything has been changed away from the engine's defaults. */
  get customized(): boolean {
    return (
      !this.filesystem ||
      !this.carving ||
      !this.reassembly ||
      !this.triage ||
      !this.previews ||
      this.minLongSide !== null ||
      this.jobs !== null
    );
  }

  /**
   * Turns a stage on or off, keeping the pair that depend on each other
   * consistent.
   *
   * Reassembly works on the candidates carving could not complete, so it has
   * nothing to work from when carving is off. Following that here is an
   * affordance, not a second copy of the rule: the engine refuses the
   * combination regardless, and this only stops the window offering a switch
   * that could not take effect.
   */
  setStage(stage: 'filesystem' | 'carving' | 'reassembly', on: boolean): void {
    this[stage] = on;
    if (stage === 'carving' && !on) this.reassembly = false;
    if (stage === 'reassembly' && on) this.carving = true;
    this.remember();
  }

  /** Sets a plain switch that nothing else depends on. */
  setFlag(flag: 'triage' | 'previews', on: boolean): void {
    this[flag] = on;
    this.remember();
  }

  /** Sets a numeric limit, treating anything unusable as "take the default". */
  setNumber(field: 'minLongSide' | 'jobs', value: number | null): void {
    this[field] = value !== null && Number.isFinite(value) && value >= 0 ? Math.floor(value) : null;
    this.remember();
  }

  /** Puts every field back to what `argos scan` does with no flags. */
  reset(): void {
    this.filesystem = true;
    this.carving = true;
    this.reassembly = true;
    this.triage = true;
    this.previews = true;
    this.minLongSide = null;
    this.jobs = null;
    this.remember();
  }

  /**
   * The scan request for `source` and `out`.
   *
   * This is the only place the window builds one, so what the button runs and
   * what the panel shows cannot drift apart.
   */
  request(source: string, out: string): ScanRequest {
    return {
      source,
      out,
      jobs: this.jobs,
      filesystem: this.filesystem,
      carving: this.carving,
      reassembly: this.reassembly,
      triage: this.triage,
      minLongSide: this.minLongSide,
      previews: this.previews,
    };
  }

  /** Reads back whatever was last chosen. Absent fields keep their default. */
  async restore(): Promise<void> {
    const stored = section(await load(), STORAGE_KEY);
    const flag = (key: string, fallback: boolean): boolean =>
      typeof stored[key] === 'boolean' ? stored[key] : fallback;
    const count = (key: string): number | null =>
      typeof stored[key] === 'number' && Number.isFinite(stored[key]) && stored[key] >= 0
        ? Math.floor(stored[key])
        : null;

    this.filesystem = flag('filesystem', true);
    this.carving = flag('carving', true);
    // Never restored into a combination the engine would refuse, which a
    // hand-edited file could otherwise ask for.
    this.reassembly = flag('reassembly', true) && this.carving;
    this.triage = flag('triage', true);
    this.previews = flag('previews', true);
    this.minLongSide = count('minLongSide');
    this.jobs = count('jobs');
  }

  private remember(): void {
    save({
      [STORAGE_KEY]: {
        filesystem: this.filesystem,
        carving: this.carving,
        reassembly: this.reassembly,
        triage: this.triage,
        previews: this.previews,
        minLongSide: this.minLongSide,
        jobs: this.jobs,
      },
    });
  }
}

/** The one set of settings the whole window reads. */
export const settings = new Settings();
