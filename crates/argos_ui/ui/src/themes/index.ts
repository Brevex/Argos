import type { ThemeModule } from './contract';

/**
 * Every theme, by id, loaded on demand.
 *
 * A dynamic import per theme means the ones nobody selected are code-split out
 * of the initial bundle. The registry itself is the only place a theme is
 * named, so adding one is this file plus its directory — and the base layout
 * does not change, because a theme cannot reach it.
 */
export const THEMES: Readonly<Record<string, () => Promise<{ default: ThemeModule }>>> = {
  default: () => import('./default'),
  aero: () => import('./aero'),
  retro: () => import('./retro'),
};

/** What the window opens with, and what a stored id falls back to. */
export const DEFAULT_THEME = 'default';

/** Ids in the order the picker shows them. */
export function themeIds(): string[] {
  return Object.keys(THEMES);
}

/**
 * Loads one theme, falling back to the default.
 *
 * A stored id that no longer matches a theme — a downgrade, a removed theme —
 * must not leave the window unstyled.
 */
export async function loadTheme(id: string): Promise<ThemeModule> {
  const load = THEMES[id] ?? THEMES[DEFAULT_THEME];
  const module = await load!();
  return module.default;
}
