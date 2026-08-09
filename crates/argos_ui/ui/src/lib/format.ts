/**
 * Display formatting, and only that.
 *
 * Every function here turns something the engine already decided into
 * something readable. None of them compute a value that influences what is
 * recovered, reported or ranked — if one ever needs to, it belongs in the
 * engine (`A-SHELL-NO-DOMAIN`).
 */

const UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'] as const;

/** A byte count, in the largest unit that leaves a number under 1024. */
export function bytes(count: number): string {
  if (!Number.isFinite(count) || count < 0) return '—';
  let value = count;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : value >= 10 ? 1 : 2;
  return `${value.toFixed(digits)} ${UNITS[unit]}`;
}

/** A duration as `MM:SS`, or `HH:MM:SS` once it passes an hour. */
export function duration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '—';
  const whole = Math.floor(seconds);
  const pad = (value: number) => value.toString().padStart(2, '0');
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  return hours > 0
    ? `${hours}:${pad(minutes)}:${pad(whole % 60)}`
    : `${pad(minutes)}:${pad(whole % 60)}`;
}

/** A count with thousands separators. */
export function count(value: number): string {
  return value.toLocaleString();
}

/**
 * A fraction 0–1 as a whole percentage, rounded down.
 *
 * Down rather than to nearest, so a hundred per cent means the work is over
 * and not that it is within half a per cent of being over. A figure that
 * reaches the end before the run does is the same kind of wrong as one that
 * stops short of it.
 */
export function percentage(fraction: number): number {
  if (!Number.isFinite(fraction)) return 0;
  const clamped = Math.min(1, Math.max(0, fraction));
  return clamped >= 1 ? 100 : Math.floor(clamped * 100);
}
