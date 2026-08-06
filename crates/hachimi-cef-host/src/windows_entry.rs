use cef::*;

#[unsafe(no_mangle)]
unsafe extern "C" fn RunWinMain(
    instance: sys::HINSTANCE,
    _command_line: *const u8,
    _command_show: i32,
    sandbox_info: *mut u8,
    _version_info: *mut u8,
) -> i32 {
    crate::bootstrap_trace::record("run_win_main_entered");
    if api_hash(sys::CEF_API_VERSION_LAST, 0).is_null()
        || api_version() != sys::CEF_API_VERSION_LAST
    {
        crate::bootstrap_trace::record("cef_api_version_rejected");
        return 1;
    }
    crate::bootstrap_trace::record("cef_api_version_configured");
    let main_args = MainArgs { instance };
    let args = args::Args::from(main_args);
    let Some(command_line) = args.as_cmd_line() else {
        crate::bootstrap_trace::record("cef_command_line_unavailable");
        return 1;
    };
    crate::bootstrap_trace::record("cef_command_line_ready");
    match crate::run_cef_host(args.as_main_args(), &command_line, sandbox_info) {
        Ok(()) => {
            crate::bootstrap_trace::record("run_win_main_completed");
            0
        }
        Err(error) => {
            crate::bootstrap_trace::record(&format!("run_win_main_failed {error}"));
            1
        }
    }
}
