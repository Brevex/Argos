/**
 * The stored preference document, read once and merged on every write.
 *
 * There is one file behind this and more than one thing kept in it, so a writer
 * that serialized only its own key would erase everyone else's. Every write
 * merges into the document last read.
 *
 * Nothing here is a recovery decision. A preference is which theme is in force
 * and which stages a person last chose to run; what those stages *do* is the
 * engine's, and this file never looks inside a value it stores
 * (`A-SHELL-NO-DOMAIN`).
 */

import * as ipc from './ipc';

/** The document as last read or written, so a merge has something to merge into. */
let document: Record<string, unknown> = {};

/**
 * The read in flight, or the one that finished.
 *
 * The promise is cached rather than a "have we read it" flag: two callers ask
 * for this on mount, and a flag set before the await would hand the second one
 * the empty document the first had not filled in yet.
 */
let reading: Promise<Record<string, unknown>> | null = null;

/**
 * The whole stored document.
 *
 * An empty one is the answer for a first run, for a file that cannot be read,
 * and for a file someone edited by hand into nonsense — a window that cannot
 * read a preference still has to draw.
 */
export function load(): Promise<Record<string, unknown>> {
  reading ??= read();
  return reading;
}

async function read(): Promise<Record<string, unknown>> {
  const text = await ipc.preferencesRead().catch(() => '');
  if (text !== '') {
    try {
      const stored: unknown = JSON.parse(text);
      if (typeof stored === 'object' && stored !== null && !Array.isArray(stored)) {
        // Merged under, not over: a write that landed while this read was in
        // flight is newer than the file it read.
        document = { ...(stored as Record<string, unknown>), ...document };
      }
    } catch {
      // Left empty on purpose; see above.
    }
  }
  return document;
}

/**
 * Merges `patch` into the stored document.
 *
 * Never awaited into a failure path: a preference that did not reach disk still
 * applies to the window that set it.
 */
export function save(patch: Record<string, unknown>): void {
  document = { ...document, ...patch };
  void ipc.preferencesWrite(JSON.stringify(document)).catch(() => undefined);
}

/** The value stored under `key`, when there is one and it is an object. */
export function section(stored: Record<string, unknown>, key: string): Record<string, unknown> {
  const value = stored[key];
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}
