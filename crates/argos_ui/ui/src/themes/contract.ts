/**
 * The contract between the base layout and a theme.
 *
 * A theme is presentation and nothing else: a name, a colour scheme, the
 * idiom its controls are drawn in, a value for every design token the layout
 * uses, and the artwork for a fixed set of glyphs. It has no markup of its
 * own, no behaviour, and no access to anything the engine said.
 *
 * **A theme declares form, not only colour.** A token here can carry a
 * gradient, a bevel, a corner, a specular band, a texture or a shape — which
 * is what lets one set of components read as three different generations of
 * interface rather than as one component set repainted. Where two idioms
 * disagree about the *kind* of control rather than its colour, the theme says
 * so in [`ThemeControls`](ThemeControls) and the layout draws what it asked
 * for: a sliding switch is right for one generation and a tick box for
 * another, and both are the same checkbox to the keyboard and the screen
 * reader.
 *
 * **Almost nothing here is a measurement.** Sizes, spacings and positions
 * belong to the layout and are the same under every theme, so a control
 * cannot sit in one place on one theme and somewhere else on another. The
 * exception is the window frame — the band, and the caption group that hangs
 * from it — because the frame *is* what a window generation looks like: one
 * has a wide translucent border with the buttons flush against its top edge,
 * another has no border at all. Everything else a theme decides is colour,
 * fill, edge, shadow, corner, texture, typeface and artwork.
 *
 * Switching one changes custom properties and two attributes on the document
 * root; no component is remounted, so a scan in progress and everything on
 * screen survive the switch.
 *
 * `Record<ThemeToken, string>` and `Record<ThemeIcon, string>` are total. **A
 * theme that omits either does not compile**, which is the same kind of
 * guarantee the rest of this project gets from its types rather than from a
 * checklist.
 */

/**
 * Every custom property the base layout reads.
 *
 * Adding one here breaks every theme until each supplies it — deliberately.
 * The alternative is a token that silently falls back to nothing in the themes
 * nobody remembered to update.
 */
export const THEME_TOKENS = [
  // The frame band itself — the surface between the window's outer edge and
  // its client area, which the title bar is part of rather than sitting on
  // top of. Where a theme leaves it translucent, the real desktop shows
  // through it, because the window is transparent outside what it paints.
  //
  // `--backdrop-noise` is the grain of the material. A pane of glass is not
  // a flat wash of colour, and the difference between a translucent rectangle
  // and something that reads as glass is mostly this: a texture fine enough
  // not to be seen as a pattern. `none` for a theme whose surfaces are just
  // surfaces.
  '--backdrop',
  '--backdrop-glow',
  '--backdrop-noise',

  // The window's own edge: the bright line just inside it, the dark line that
  // separates it from the desktop, its corner and the shadow it casts.
  //
  // `--window-inset` is how far the client area sits inside that edge — the
  // width of the frame band itself. Zero makes the frame a hairline; a real
  // measure makes it the wide translucent border a window of a certain
  // generation had, with the desktop showing through it.
  '--window-border',
  '--window-edge',
  '--window-radius',
  '--window-shadow',
  '--window-inset',

  // The client area: the surface the controls actually sit on, inside the
  // frame. A theme that wants no distinction between frame and client gives
  // this no fill and no border.
  '--main-surface',
  '--main-border',
  '--main-shadow',
  '--main-radius',

  // The title bar across the top of that frame.
  '--titlebar',
  '--titlebar-border',
  '--titlebar-text',
  '--titlebar-text-size',
  '--titlebar-text-weight',
  '--titlebar-text-shadow',

  // Minimise, maximise and close: where the group sits against the frame, how
  // big its buttons are, and what one looks like in each of its states.
  //
  // The geometry is here rather than in the layout because the group belongs
  // to the frame: one generation hangs it flush from the window's top edge in
  // a divided glass strip, another floats three bare glyphs beside the title.
  // Both are the same three buttons in the same order.
  '--winbtn-align',
  '--winbtn-offset-top',
  '--winbtn-offset-right',
  '--winbtn-height',
  '--winbtn-width',
  '--winbtn-close-width',
  '--winbtn-glyph',
  // Each button's own edge and corner, and the air between them. A frame that
  // divides one strip into three gives these no colour and no gap; a frame
  // that floats three separate targets draws each one.
  '--winbtn-border',
  '--winbtn-radius',
  '--winbtn-gap',
  // How far the group rises above the title strip's own padding when a dialog
  // carries one, so a frame that hangs its buttons from the window's edge does
  // the same inside a dialog and one that centres them stays centred.
  '--winbtn-group-pull',
  '--winbtn-group',
  '--winbtn-group-border',
  '--winbtn-group-radius',
  '--winbtn-group-shadow',
  '--winbtn-divider',
  '--winbtn-divider-light',
  '--winbtn-face',
  '--winbtn-text',
  '--winbtn-hover',
  '--winbtn-hover-text',
  // The light a caption button throws when the pointer is on it. A filter
  // rather than a shadow, because the group clips its children and a shadow
  // drawn inside one would be cut off at its edges.
  '--winbtn-hover-filter',
  '--winbtn-close-hover-filter',
  '--winbtn-active',
  '--winbtn-close',
  '--winbtn-close-text',
  '--winbtn-close-hover',
  '--winbtn-close-hover-text',
  '--winbtn-close-active',
  '--winbtn-stroke',
  // The same three buttons on a window that does not have the focus. Measured
  // off the reference's own background window: the faces give way to the
  // glass, the edges between them go pale, and the close is not red — being
  // dangerous is a property of the window you are working in.
  '--winbtn-face-off',
  '--winbtn-close-off',
  '--winbtn-group-border-off',
  '--winbtn-divider-off',

  // `--pane` is a fill; `--pane-blur` is the backdrop filter behind it. A
  // theme with no translucency sets the fill opaque and the blur to `none`,
  // and the layout is unchanged.
  '--pane',
  '--pane-blur',
  '--pane-border',
  '--pane-shadow',
  '--pane-radius',
  '--scrim',

  // The sheet the drive table and the destination field share.
  //
  // Whether they share one at all is a theme's decision: giving this no fill,
  // no border and no shadow leaves the two controls sitting directly on the
  // client area as two separate things, which is what some designs want and
  // others do not.
  '--form-pane',
  '--form-pane-border',
  '--form-pane-shadow',

  // A settings window is a window. `--dialog-strip` is the material its frame
  // is made of — the band its title sits on and, where `--dialog-inset` gives
  // it width, the border around its panel as well. On a system whose windows
  // are framed in glass, that material is the glass and the dialog is a small
  // copy of the main window; on one whose window is a single unbroken surface,
  // it is the panel's own fill and there is no frame to see.
  //
  // This is the only glass in the application that can be real. The window's
  // own frame sits over the desktop, which no filter here can reach; a dialog
  // sits over the window, which is this page — so a blur behind it blurs
  // something.
  '--dialog-strip',
  '--dialog-strip-height',
  '--dialog-inset',
  '--dialog-blur',

  // Rows in the drive table, and the one that is selected.
  '--row',
  '--row-hover',
  '--row-selected',
  '--row-selected-border',
  '--row-selected-shadow',
  '--row-radius',

  // The one-of-many control on each row. `--choice-mark` is what fills it
  // when the row is the chosen one; a theme that shows nothing on the others
  // gives the face and the border no colour.
  '--choice',
  '--choice-border',
  '--choice-border-selected',
  '--choice-mark',
  '--choice-shadow',

  // Inset surfaces: the destination field, the job cards, the progress track.
  // `--inset-shadow` is the sunken bevel — the dark line along the top inside
  // edge and the light one along the bottom that make a surface read as cut
  // into the sheet rather than laid on it.
  //
  // A groove is closed by two lines of unequal weight: `--track-edge` along
  // the outer edge, where a channel is deepest, and the lighter `--track-lit`
  // along the inner one. Both are drawn over the filled part as well as the
  // empty, because the groove does not stop where the fill starts.
  '--inset',
  '--inset-border',
  '--inset-border-hover',
  '--inset-shadow',
  '--input-radius',
  '--track',
  '--track-edge',
  '--track-lit',
  '--track-shadow',

  // Three kinds, and the layout uses no others: the button that runs the job,
  // the ordinary button, and the one that is a word rather than an object.
  //
  // Every fill is a whole background value rather than a colour, so a theme
  // can make a button a gloss, a flat block or an outline without the layout
  // knowing which. The `-text-hover` pair exists for the themes whose hover
  // *inverts* rather than lightens: swapping the ground without swapping the
  // ink leaves a label the same colour as what is now behind it.
  '--action',
  '--action-hover',
  '--action-active',
  '--action-text',
  '--action-text-hover',
  '--action-border',
  '--action-shadow',
  '--action-shadow-active',
  '--button',
  '--button-hover',
  '--button-active',
  '--button-text',
  '--button-text-hover',
  '--button-border',
  '--button-border-hover',
  '--button-shadow',
  '--button-shadow-active',
  '--button-radius',
  '--link',
  '--link-hover',

  // What a control that cannot be used looks like. A theme whose disabled
  // state is a paler ink sets the colour and leaves the opacity at 1; one
  // that fades the whole control does the reverse.
  '--disabled-text',
  '--disabled-opacity',

  // The keyboard's own mark, as an outline: never only a colour, because the
  // ring has to be visible on a control whose colours already changed for
  // another reason — and never a shadow, because a shadow is what half these
  // controls already use to draw their own edge.
  '--focus-outline',
  '--focus-offset',

  // What the arcs are made of. `--ring` is set by the layout from the state
  // of the run and read by everything else here, so a theme can derive the
  // rest from whatever colour the state put there.
  //
  // The arc is banded across its width rather than shaded along its length:
  // `--ring-highlight` is the light band along its inner edge and
  // `--ring-edge` the one along its outer edge, which is how a fill of this
  // kind is actually built — light at the top, saturated through the middle,
  // brightening again at the bottom.
  '--ring',
  '--ring-highlight',
  '--ring-edge',
  '--ring-glow',
  '--ring-cap',

  // One meaning, one colour, declared once for every theme: a run under way,
  // a run suspended, a run that finished, one stopped early, one that failed.
  // The layout decides which state a run is in; a theme decides only what
  // each of them looks like.
  '--progress-running',
  '--progress-paused',
  '--progress-done',
  '--progress-cancelled',
  '--progress-failed',

  // The highlight that travels the filled part of the arc, the way a progress
  // bar of a certain vintage did. How wide it is and how far it goes are the
  // layout's; a theme decides what it is made of, how long it takes and how it
  // accelerates. One that wants none sets the duration to `0s`, which is what
  // stops the animation from running at all rather than merely hiding it.
  '--sheen',
  '--sheen-duration',
  '--sheen-easing',

  // A binary control drawn as a sliding switch.
  '--switch-track',
  '--switch-track-on',
  '--switch-border',
  '--switch-border-on',
  '--switch-thumb',
  '--switch-radius',
  '--switch-thumb-radius',

  // The same control drawn as a tick box. Which of the two a theme gets is
  // [`ThemeControls.checkbox`](ThemeControls); both sets are supplied either
  // way, because a theme that changes its mind should not have to be edited
  // in two places to do it.
  '--check-box',
  '--check-box-checked',
  '--check-border',
  '--check-border-checked',
  '--check-mark',
  '--check-radius',
  '--check-shadow',
  // A tick box lights up under the pointer on the desktop this theme comes
  // from — the well turns pale blue and its edge with it.
  '--check-border-hover',
  '--check-shadow-hover',

  // The seal on a drive row: what the operating system says about that medium
  // that a person should know before trusting a result. Two weights — one for
  // a fact that bears on the recovery, one for a fact that merely describes
  // the medium.
  '--badge',
  '--badge-border',
  '--badge-text',
  '--badge-quiet',
  '--badge-quiet-border',
  '--badge-quiet-text',
  '--badge-radius',

  // The scrollbar, which is a control like any other and reads as an
  // afterthought when it is the one thing left in the browser's own style.
  '--scrollbar-track',
  '--scrollbar-thumb',
  '--scrollbar-thumb-hover',
  '--scrollbar-border',
  '--scrollbar-radius',

  // The texture of a lit surface, laid over the fill of the panes, the table,
  // the field, the statistics strip and the progress arcs — and over nothing
  // else. A display that drew in rows leaves rows; the frame, the title and
  // the window edge were never on that display and stay crisp. `none` for a
  // theme whose surfaces are just surfaces.
  '--scanlines',

  // The bloom a phosphor dot leaves around whatever it lights: a `text-shadow`
  // applied to every piece of text. `none` for a theme with no phosphor.
  '--text-glow',

  // The two edges that make a surface read as raised or sunken: a light line
  // inside the top and a dark one inside the bottom, or the reverse. Whole
  // `box-shadow` values, so a theme with no relief gives them `none`.
  '--bevel-raised',
  '--bevel-sunken',
  // The band of light lying across the upper half of anything clickable.
  '--specular',

  // Text.
  '--text',
  '--text-dim',
  '--text-faint',
  '--heading',

  // Meaning. These name a role, never a colour: a theme decides what "danger"
  // looks like, the layout only decides what is dangerous.
  '--accent',
  '--accent-strong',
  '--accent-text',
  '--ok',
  '--warn',
  '--danger',

  '--font',
] as const;

/** One custom property the base layout reads. */
export type ThemeToken = (typeof THEME_TOKENS)[number];

/**
 * The shape a control takes, where two idioms disagree about the kind of
 * object rather than about its colour.
 *
 * Each is written to the document root as a `data-` attribute, and the one
 * component that draws that control selects on it. The control itself does
 * not change: a checkbox is a checkbox to the keyboard, to the accessibility
 * tree and to the state it holds, whichever of these it is painted as.
 */
export interface ThemeControls {
  /**
   * How something that is on or off is drawn: a sliding switch, a tick box,
   * or a lit cell in square brackets.
   */
  readonly checkbox: 'switch' | 'checkbox' | 'bracket';
  /**
   * How the chosen one of several is marked: a filled dot, a bevelled radio
   * button, or a bracketed mark.
   */
  readonly choice: 'dot' | 'radio' | 'bracket';
}

/**
 * Every glyph a theme draws.
 *
 * Artwork, not markup: each is the inside of an `<svg>` whose viewBox the
 * layout fixes, so a theme chooses how a drive is drawn and cannot choose that
 * a drive is drawn somewhere else.
 */
export const THEME_ICONS = [
  /** Minimise, maximise, close. `viewBox="0 0 12 12"`. */
  'minimise',
  'maximise',
  'close',
  /** The mark on the theme in force. `viewBox="0 0 14 14"`. */
  'tick',
  /** The two halves of a number field's stepper. `viewBox="0 0 8 6"`. */
  'up',
  'down',
] as const;

/** One glyph a theme draws. */
export type ThemeIcon = (typeof THEME_ICONS)[number];

/** A theme module, as `themes/<id>/index.ts` default-exports one. */
export interface ThemeModule {
  /** Stable identifier; also the directory name and what is persisted. */
  readonly id: string;
  /** Name shown in the theme picker, and the only thing it shows. */
  readonly name: string;
  /**
   * What the browser should assume about the page's own colours, so form
   * controls and scrollbars match rather than fighting the theme.
   */
  readonly scheme: 'dark' | 'light';
  /** Which shape each control of two minds takes. */
  readonly controls: ThemeControls;
  /** A value for every token. Total by construction. */
  readonly tokens: Readonly<Record<ThemeToken, string>>;
  /** Artwork for every glyph. Total by construction. */
  readonly icons: Readonly<Record<ThemeIcon, string>>;
}

/**
 * Helper that gives a theme module its type without widening it.
 *
 * Themes call this instead of annotating, so a missing token is reported at
 * the property that is missing rather than as a mismatch on the whole object.
 */
export function theme(module: ThemeModule): ThemeModule {
  return module;
}
