# 04 — The theme preference belonged to the machine, not to the person

Argos opened in the Retro theme rather than the default.

## What was measured

The theme code is correct. `remembered()` reads the stored preference and falls
back to `DEFAULT_THEME` — `'default'` — when there is none; the three theme
modules are distinct; the picker dispatches on `choice.id`. Retro had been chosen
during use, and the window remembered it, which is what it is meant to do.

The defect is where it remembered it. The window runs as an administrator, so
everything the web view stores lands in the administrator's profile. The user's
own profile still held two now-orphaned origins from before the change:

```
http_localhost_5173  →  aero      (from `tauri dev`)
tauri_localhost_0    →  default   (from the bundled application)
```

So the preference had silently moved to `/root`, where it belongs to the machine:
shared by every account that opens Argos, and invisible to all of them. There is
nowhere to look at it and nothing to reset.

## Cause

`localStorage` is scoped to the web view's data directory, which is derived from
the *process's* home. Elevating the window moved that directory without moving
anything else, and no code was involved in the move.

## Change

`crates/argos_ui/src/preference.rs` writes the preference to
`$ARGOS_INVOKER_HOME/.config/argos/ui.json` — the home of whoever opened Argos,
which the elevation already carries — and hands the file to that account with
`chown`, the same way the recovery output is handed back. Two Tauri commands read
and write it; `localStorage` is no longer used.

The result is a preference that belongs to a person, in a file they can read,
edit or delete.

## Note

The shell does not link `argos_report`, so it does not reuse that crate's
ownership handback: it calls `std::os::unix::fs::chown` directly. Linking it
would put the reporting crate inside the presentation shell, which is exactly
what `A-SHELL-NO-DOMAIN` forbids — the shell's dependency list is the mechanism
that keeps recovery logic out of it, and one shared syscall wrapper is not worth
weakening it.
