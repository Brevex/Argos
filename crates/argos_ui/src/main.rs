//! Entry point of the Argos shell.

// The shell has no console of its own on Windows: it is a window, and an
// orphan terminal behind it is not part of the interface.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    argos_ui::run();
}
