/**
 * Negative tests for the theme contract, checked by the compiler.
 *
 * There is no test runner in this frontend and there should not be: it holds
 * no logic worth running. What it does hold is one guarantee — that a theme
 * supplies every token and every glyph the base layout reads — and that
 * guarantee is a type, so the compiler is the thing that checks it.
 *
 * Each `@ts-expect-error` below **fails the build if the error stops
 * happening**. That is the point: if `Record<ThemeToken, string>` ever stops
 * being total, or a token is dropped from the union, these lines quietly start
 * compiling and `svelte-check` reports the unused suppression.
 */

import { theme, type ThemeIcon, type ThemeToken } from './contract';

const complete: Record<ThemeToken, string> = {
  '--backdrop': '#000',
  '--backdrop-glow': 'none',
  '--window-border': '#222',
  '--window-radius': '0',
  '--window-shadow': 'none',
  '--window-inset': '0rem',
  '--main-surface': 'transparent',
  '--main-border': 'transparent',
  '--main-shadow': 'none',
  '--main-radius': '0',
  '--titlebar': 'transparent',
  '--titlebar-border': 'transparent',
  '--titlebar-text': '#fff',
  '--winbtn-group': 'transparent',
  '--winbtn-group-border': 'transparent',
  '--winbtn-group-radius': '0',
  '--winbtn-divider': 'transparent',
  '--winbtn-text': '#ccc',
  '--winbtn-hover': '#111',
  '--winbtn-close-hover': '#f00',
  '--winbtn-close-hover-text': '#fff',
  '--winbtn-stroke': '1',
  '--pane': '#111',
  '--pane-blur': 'none',
  '--pane-border': '#222',
  '--pane-shadow': 'none',
  '--pane-radius': '0',
  '--scrim': 'none',
  '--form-pane': 'transparent',
  '--form-pane-border': 'transparent',
  '--form-pane-shadow': 'none',
  '--row': 'transparent',
  '--row-hover': '#111',
  '--row-selected': '#123',
  '--row-selected-border': '#345',
  '--dot-idle': '#888',
  '--inset': '#000',
  '--inset-border': '#222',
  '--track': '#222',
  '--track-edge': '#333',
  '--ring': '#00f',
  '--ring-highlight': '#66f',
  '--ring-shadow': '#006',
  '--ring-glow': 'none',
  '--ring-cap': 'butt',
  '--ring-pulse': 'transparent',
  '--ring-pulse-duration': '0s',
  '--scanlines': 'none',
  '--text-glow': 'none',
  '--text': '#fff',
  '--text-dim': '#ccc',
  '--text-faint': '#888',
  '--accent': '#00f',
  '--accent-strong': '#33f',
  '--accent-text': '#fff',
  '--action': '#00f',
  '--action-text': '#fff',
  '--action-border': '#33f',
  '--action-shadow': 'none',
  '--ok': '#0f0',
  '--warn': '#ff0',
  '--danger': '#f00',
  '--switch-track': '#111',
  '--switch-track-on': '#4d94ff',
  '--switch-border': '#222',
  '--switch-border-on': '#4d94ff',
  '--switch-thumb': '#fff',
  '--switch-radius': '999px',
  '--switch-thumb-radius': '50%',
  '--radius': '0',
  '--font': 'sans-serif',
};

const drawn: Record<ThemeIcon, string> = {
  disk: '',
  minimise: '',
  maximise: '',
  close: '',
};

/** A theme supplying every token and every glyph is accepted. */
export const accepted = theme({
  id: 'guard-complete',
  name: 'Complete',
  scheme: 'dark',
  tokens: complete,
  icons: drawn,
});

/** A theme missing one token is rejected. */
export const missingToken = theme({
  id: 'guard-incomplete',
  name: 'Incomplete',
  scheme: 'dark',
  // @ts-expect-error a theme that omits a token must not compile
  tokens: (({ '--pane-shadow': _dropped, ...rest }) => rest)(complete),
  icons: drawn,
});

/** A token the layout does not read is rejected too, so themes stay in step. */
export const strayToken = theme({
  id: 'guard-stray',
  name: 'Stray',
  scheme: 'dark',
  // @ts-expect-error a token outside the contract must not compile
  tokens: { ...complete, '--not-a-real-token': '#000' },
  icons: drawn,
});

/** A theme missing one glyph is rejected. */
export const missingIcon = theme({
  id: 'guard-unglyphed',
  name: 'Unglyphed',
  scheme: 'dark',
  tokens: complete,
  // @ts-expect-error a theme that omits a glyph must not compile
  icons: (({ disk: _dropped, ...rest }) => rest)(drawn),
});

/** A scheme outside the two the contract allows is rejected. */
export const strayScheme = theme({
  id: 'guard-scheme',
  name: 'Stray scheme',
  // @ts-expect-error only 'dark' and 'light' are colour schemes here
  scheme: 'sepia',
  tokens: complete,
  icons: drawn,
});
