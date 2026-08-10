// Desktop-dev entry point for the host (running the host on Linux is a
// convenience for iterating on host logic; the product target is Android,
// where lib.rs's mobile_entry_point is used instead).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tendrl_mobile_lib::run()
}
