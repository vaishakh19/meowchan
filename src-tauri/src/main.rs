// Diskwise desktop entry point.
//
// The window is undecorated so the UI can draw its own title bar; the capability file grants only
// the window controls needed to make that work (minimize, toggle-maximize, close, start-dragging).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    diskwise_lib::run();
}
