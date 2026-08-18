// The overlay is a GUI: on Windows a console window would appear behind it. Harmless here,
// wrong everywhere else, and one line to prevent.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    hippocampus_ui_lib::run()
}
