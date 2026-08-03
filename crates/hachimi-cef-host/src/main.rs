#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

#[cfg(not(all(feature = "sandbox", target_os = "windows")))]
fn main() -> Result<(), String> {
    let args = cef::args::Args::new();
    let Some(command_line) = args.as_cmd_line() else {
        return Err("cef_command_line_invalid".into());
    };
    hachimi_cef_host::run_cef_host(args.as_main_args(), &command_line, std::ptr::null_mut())
}

#[cfg(all(feature = "sandbox", target_os = "windows"))]
fn main() -> Result<(), String> {
    Err("hachimi-cef-host must be launched through the CEF sandbox bootstrap".into())
}
