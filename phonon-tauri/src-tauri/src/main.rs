// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Collect CLI args: skip the first arg (executable path)
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        app_lib::set_cli_args(args);
    }
    app_lib::run();
}
