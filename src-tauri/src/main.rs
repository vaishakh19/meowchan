// Diskwise desktop entry point.
// Uses the native Windows title bar.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    diskwise_lib::run();
}
