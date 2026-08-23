/**
 * Negative tests for the theme contract, checked by the compiler.
 *
 * There is no test runner in this frontend and there should not be: it holds
 * no logic worth running. What it does hold is one guarantee — that a theme
 * supplies every token, every glyph and every control idiom the base layout
 * reads — and that guarantee is a type, so the compiler is the thing that
 * checks it.
 *
 * Each `@ts-expect-error` below **fails the build if the error stops
 * happening**. That is the point: if `Record<ThemeToken, string>` ever stops
 * being total, or a token is dropped from the union, these lines quietly start
 * compiling and `svelte-check` reports the unused suppression.
 */

import { theme, type ThemeControls, type ThemeIcon, type ThemeToken } from './contract';

const complete: Record<ThemeToken, string> = {
  '--backdrop': '#000',
  '--backdrop-glow': 'none',
  '--backdrop-noise': 'none',
  '--window-border': '#000',
  '--window-edge': '#000',
  '--window-radius': '0',
  '--window-shadow': 'none',
  '--window-inset': '0rem',
  '--main-surface': '#000',
  '--main-border': '#000',
  '--main-shadow': 'none',
  '--main-radius': '0',
  '--titlebar': '#000',
  '--titlebar-border': '#000',
  '--titlebar-text': '#000',
  '--titlebar-text-size': '1rem',
  '--titlebar-text-weight': '400',
  '--titlebar-text-shadow': 'none',
  '--winbtn-align': 'center',
  '--winbtn-offset-top': '0rem',
  '--winbtn-offset-right': '0rem',
  '--winbtn-height': '0rem',
  '--winbtn-width': '0rem',
  '--winbtn-close-width': '0rem',
  '--winbtn-glyph': '0rem',
  '--winbtn-border': '#000',
  '--winbtn-radius': '0',
  '--winbtn-gap': '0rem',
  '--winbtn-group-pull': '0rem',
  '--winbtn-group': '#000',
  '--winbtn-group-border': '#000',
  '--winbtn-group-radius': '0',
  '--winbtn-group-shadow': 'none',
  '--winbtn-divider': '#000',
  '--winbtn-divider-light': '#000',
  '--winbtn-face': '#000',
  '--winbtn-text': '#000',
  '--winbtn-hover': '#000',
  '--winbtn-hover-text': '#000',
  '--winbtn-hover-filter': 'none',
  '--winbtn-close-hover-filter': 'none',
  '--winbtn-active': '#000',
  '--winbtn-close': '#000',
  '--winbtn-close-text': '#000',
  '--winbtn-close-hover': '#000',
  '--winbtn-close-hover-text': '#000',
  '--winbtn-close-active': '#000',
  '--winbtn-stroke': '1',
  '--winbtn-face-off': '#000',
  '--winbtn-close-off': '#000',
  '--winbtn-group-border-off': '#000',
  '--winbtn-divider-off': '#000',
  '--pane': '#000',
  '--pane-blur': 'none',
  '--pane-border': '#000',
  '--pane-shadow': 'none',
  '--pane-radius': '0',
  '--scrim': '#000',
  '--form-pane': '#000',
  '--form-pane-border': '#000',
  '--form-pane-shadow': 'none',
  '--dialog-strip': '#000',
  '--dialog-strip-height': '0rem',
  '--dialog-inset': '0rem',
  '--dialog-blur': 'none',
  '--row': '#000',
  '--row-hover': '#000',
  '--row-selected': '#000',
  '--row-selected-border': '#000',
  '--row-selected-shadow': 'none',
  '--row-radius': '0',
  '--choice': '#000',
  '--choice-border': '#000',
  '--choice-border-selected': '#000',
  '--choice-mark': '#000',
  '--choice-shadow': 'none',
  '--inset': '#000',
  '--inset-border': '#000',
  '--inset-border-hover': '#000',
  '--inset-shadow': 'none',
  '--input-radius': '0',
  '--track': '#000',
  '--track-edge': '#000',
  '--track-lit': '#000',
  '--track-shadow': 'none',
  '--action': '#000',
  '--action-hover': '#000',
  '--action-active': '#000',
  '--action-text': '#000',
  '--action-text-hover': '#000',
  '--action-border': '#000',
  '--action-shadow': 'none',
  '--action-shadow-active': 'none',
  '--button': '#000',
  '--button-hover': '#000',
  '--button-active': '#000',
  '--button-text': '#000',
  '--button-text-hover': '#000',
  '--button-border': '#000',
  '--button-border-hover': '#000',
  '--button-shadow': 'none',
  '--button-shadow-active': 'none',
  '--button-radius': '0',
  '--link': '#000',
  '--link-hover': '#000',
  '--disabled-text': '#000',
  '--disabled-opacity': '1',
  '--focus-outline': '1px solid #fff',
  '--focus-offset': '0rem',
  '--ring': '#000',
  '--ring-highlight': '#000',
  '--ring-edge': '#000',
  '--ring-glow': 'none',
  '--ring-cap': 'butt',
  '--progress-running': '#000',
  '--progress-paused': '#000',
  '--progress-done': '#000',
  '--progress-cancelled': '#000',
  '--progress-failed': '#000',
  '--sheen': '#000',
  '--sheen-duration': '0s',
  '--sheen-easing': 'linear',
  '--switch-track': '#000',
  '--switch-track-on': '#000',
  '--switch-border': '#000',
  '--switch-border-on': '#000',
  '--switch-thumb': '#000',
  '--switch-radius': '0',
  '--switch-thumb-radius': '0',
  '--check-box': '#000',
  '--check-box-checked': '#000',
  '--check-border': '#000',
  '--check-border-checked': '#000',
  '--check-mark': '#000',
  '--check-radius': '0',
  '--check-shadow': 'none',
  '--check-border-hover': '#000',
  '--check-shadow-hover': 'none',
  '--badge': '#000',
  '--badge-border': '#000',
  '--badge-text': '#000',
  '--badge-quiet': '#000',
  '--badge-quiet-border': '#000',
  '--badge-quiet-text': '#000',
  '--badge-radius': '0',
  '--scrollbar-track': '#000',
  '--scrollbar-thumb': '#000',
  '--scrollbar-thumb-hover': '#000',
  '--scrollbar-border': '#000',
  '--scrollbar-radius': '0',
  '--scanlines': 'none',
  '--text-glow': 'none',
  '--bevel-raised': 'none',
  '--bevel-sunken': 'none',
  '--specular': 'none',
  '--text': '#000',
  '--text-dim': '#000',
  '--text-faint': '#000',
  '--heading': '#000',
  '--accent': '#000',
  '--accent-strong': '#000',
  '--accent-text': '#000',
  '--ok': '#000',
  '--warn': '#000',
  '--danger': '#000',
  '--font': 'sans-serif',};

const drawn: Record<ThemeIcon, string> = {
  minimise: '',
  maximise: '',
  close: '',
  tick: '',
  up: '',
  down: '',
};

const shaped: ThemeControls = { checkbox: 'switch', choice: 'dot' };

/** A theme supplying every token, glyph and idiom is accepted. */
export const accepted = theme({
  id: 'guard-complete',
  name: 'Complete',
  scheme: 'dark',
  controls: shaped,
  tokens: complete,
  icons: drawn,
});

/** A theme missing one token is rejected. */
export const missingToken = theme({
  id: 'guard-incomplete',
  name: 'Incomplete',
  scheme: 'dark',
  controls: shaped,
  // @ts-expect-error a theme that omits a token must not compile
  tokens: (({ '--pane-shadow': _dropped, ...rest }) => rest)(complete),
  icons: drawn,
});

/** A token the layout does not read is rejected too, so themes stay in step. */
export const strayToken = theme({
  id: 'guard-stray',
  name: 'Stray',
  scheme: 'dark',
  controls: shaped,
  // @ts-expect-error a token outside the contract must not compile
  tokens: { ...complete, '--not-a-real-token': '#000' },
  icons: drawn,
});

/** A theme missing one glyph is rejected. */
export const missingIcon = theme({
  id: 'guard-unglyphed',
  name: 'Unglyphed',
  scheme: 'dark',
  controls: shaped,
  tokens: complete,
  // @ts-expect-error a theme that omits a glyph must not compile
  icons: (({ close: _dropped, ...rest }) => rest)(drawn),
});

/** An idiom outside the ones the layout can draw is rejected. */
export const strayIdiom = theme({
  id: 'guard-idiom',
  name: 'Stray idiom',
  scheme: 'dark',
  // @ts-expect-error only the idioms the layout draws are controls
  controls: { checkbox: 'lever', choice: 'dot' },
  tokens: complete,
  icons: drawn,
});

/** A scheme outside the two the contract allows is rejected. */
export const strayScheme = theme({
  id: 'guard-scheme',
  name: 'Stray scheme',
  // @ts-expect-error only 'dark' and 'light' are colour schemes here
  scheme: 'sepia',
  controls: shaped,
  tokens: complete,
  icons: drawn,
});
