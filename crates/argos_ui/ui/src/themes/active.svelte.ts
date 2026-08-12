/**
 * The theme in force, and the one place it is applied.
 *
 * Applying a theme writes its tokens onto the document root and keeps its
 * module so components can read the artwork out of it. Nothing here is
 * remounted and no state is lost, which is why the picker can be used in the
 * middle of a running scan.
 */

import * as ipc from '../lib/ipc';
import type { ThemeIcon, ThemeModule } from './contract';
import { DEFAULT_THEME, loadTheme } from './index';

/** The key inside the stored preferences. A view preference, nothing more. */
const STORAGE_KEY = 'theme';

class Active {
  /** The module in force, or `null` before the first one has loaded. */
  module = $state<ThemeModule | null>(null);

  /** The id in force, for the picker's tick. */
  get id(): string {
    return this.module?.id ?? DEFAULT_THEME;
  }

  /**
   * One glyph's artwork, or nothing before a theme has loaded.
   *
   * Empty is the right answer for that moment: an icon drawn in colours the
   * page has not adopted yet would flash the wrong theme for a frame.
   */
  icon(name: ThemeIcon): string {
    return this.module?.icons[name] ?? '';
  }
}

/** The one theme the whole window reads. */
export const active = new Active();

/** Loads `id`, applies its tokens to the document, and remembers it. */
export async function apply(id: string): Promise<void> {
  const module = await loadTheme(id);
  const root = document.documentElement;
  for (const [token, value] of Object.entries(module.tokens)) {
    root.style.setProperty(token, value);
  }
  root.style.colorScheme = module.scheme;
  active.module = module;
  // Stored where the person who opened Argos can find it. A window that cannot
  // persist a preference still has to render, so this is never awaited into a
  // failure path.
  void ipc
    .preferencesWrite(JSON.stringify({ [STORAGE_KEY]: module.id }))
    .catch(() => undefined);
}

/** The theme to open with: the one last chosen, or the default. */
export async function remembered(): Promise<string> {
  const text = await ipc.preferencesRead().catch(() => '');
  if (text === '') return DEFAULT_THEME;
  try {
    const stored: unknown = JSON.parse(text);
    const theme =
      typeof stored === 'object' && stored !== null
        ? (stored as Record<string, unknown>)[STORAGE_KEY]
        : undefined;
    return typeof theme === 'string' ? theme : DEFAULT_THEME;
  } catch {
    // A file someone edited by hand into nonsense is not a reason not to draw.
    return DEFAULT_THEME;
  }
}
