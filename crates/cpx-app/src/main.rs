// A menu-bar app should not flash a console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    cpx_app_lib::run()
}
