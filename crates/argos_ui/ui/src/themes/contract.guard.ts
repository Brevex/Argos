/**
 * Negative tests for the theme contract, checked by the compiler.
 *
 * There is no test runner in this frontend and there should not be: it holds
 * no logic worth running. What it does hold is one guarantee — that a theme
 * supplies every token the base layout reads — and that guarantee is a type,
 * so the compiler is the thing that checks it.
 *
 * Each `@ts-expect-error` below **fails the build if the error stops
 * happening**. That is the point: if `Record<ThemeToken, string>` ever stops
 * being total, or a token is dropped from the union, these lines quietly start
 * compiling and `svelte-check` reports the unused suppression.
 */

import { theme, type ThemeToken } from './contract';

const complete: Record<ThemeToken, string> = {
  '--backdrop': '#000',
  '--backdrop-glow': 'none',
  '--pane': '#111',
  '--pane-blur': 'none',
  '--pane-border': '#222',
  '--pane-shadow': 'none',
  '--pane-radius': '0',
  '--scrim': 'none',
  '--row': 'transparent',
  '--row-hover': '#111',
  '--row-selected': '#123',
  '--row-selected-border': '#345',
  '--inset': '#000',
  '--inset-border': '#222',
  '--track': '#222',
  '--ring': '#00f',
  '--ring-glow': 'none',
  '--text': '#fff',
  '--text-dim': '#ccc',
  '--text-faint': '#888',
  '--accent': '#00f',
  '--accent-strong': '#33f',
  '--accent-text': '#fff',
  '--accent-glow': 'none',
  '--action': '#00f',
  '--action-text': '#fff',
  '--ok': '#0f0',
  '--warn': '#ff0',
  '--danger': '#f00',
  '--radius': '0',
  '--font': 'sans-serif',
  '--font-mono': 'monospace',
};

/** A theme supplying every token is accepted. */
export const accepted = theme({
  id: 'guard-complete',
  name: 'Complete',
  description: 'Supplies every token.',
  scheme: 'dark',
  tokens: complete,
});

/** A theme missing one token is rejected. */
export const missingToken = theme({
  id: 'guard-incomplete',
  name: 'Incomplete',
  description: 'Omits --pane-shadow.',
  scheme: 'dark',
  // @ts-expect-error a theme that omits a token must not compile
  tokens: (({ '--pane-shadow': _dropped, ...rest }) => rest)(complete),
});

/** A token the layout does not read is rejected too, so themes stay in step. */
export const strayToken = theme({
  id: 'guard-stray',
  name: 'Stray',
  description: 'Declares a token the layout never reads.',
  scheme: 'dark',
  // @ts-expect-error a token outside the contract must not compile
  tokens: { ...complete, '--not-a-real-token': '#000' },
});

/** A scheme outside the two the contract allows is rejected. */
export const strayScheme = theme({
  id: 'guard-scheme',
  name: 'Stray scheme',
  description: 'Uses a colour scheme the contract does not define.',
  // @ts-expect-error only 'dark' and 'light' are colour schemes here
  scheme: 'sepia',
  tokens: complete,
});
