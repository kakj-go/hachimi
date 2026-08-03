use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use cef::*;
use hachimi_browser::{CEF_IPC_PROTOCOL_VERSION, CefHostMessage};

use crate::ipc::{EventSink, run_command_loop};
use crate::tab_manager::TabManager;

#[derive(Debug, Clone)]
struct HostOptions {
    parent_hwnd: usize,
    profile_dir: PathBuf,
    log_file: PathBuf,
}

impl HostOptions {
    fn from_process() -> Result<Self, String> {
        let value = |prefix: &str| {
            std::env::args().find_map(|argument| argument.strip_prefix(prefix).map(str::to_owned))
        };
        let parent_hwnd = value("--hachimi-parent-hwnd=")
            .ok_or_else(|| "cef_parent_window_missing".to_owned())?
            .parse::<usize>()
            .map_err(|_| "cef_parent_window_invalid".to_owned())?;
        if parent_hwnd == 0 {
            return Err("cef_parent_window_invalid".into());
        }
        let profile_dir = value("--hachimi-profile-dir=")
            .map(PathBuf::from)
            .ok_or_else(|| "cef_profile_dir_missing".to_owned())?;
        let log_file = value("--hachimi-log-file=")
            .map(PathBuf::from)
            .unwrap_or_else(|| profile_dir.join("cef.log"));
        Ok(Self {
            parent_hwnd,
            profile_dir,
            log_file,
        })
    }
}

wrap_app! {
    struct HachimiCefApp {
        manager: TabManager,
        sink: EventSink,
    }

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(HachimiBrowserProcessHandler::new(
                self.manager.clone(),
                self.sink.clone(),
            ))
        }
    }
}

wrap_browser_process_handler! {
    struct HachimiBrowserProcessHandler {
        manager: TabManager,
        sink: EventSink,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            self.manager.mark_context_ready();
            self.sink.send(&CefHostMessage::Ready {
                protocol_version: CEF_IPC_PROTOCOL_VERSION,
                chromium_version: "151.3.14".into(),
            });
        }
    }
}

pub fn run_cef_host(
    main_args: &MainArgs,
    command_line: &CommandLine,
    sandbox_info: *mut u8,
) -> Result<(), String> {
    let is_browser_process = command_line.has_switch(Some(&CefString::from("type"))) == 0;
    crate::bootstrap_trace::record(if is_browser_process {
        "cef_execute_process_browser_begin"
    } else {
        "cef_execute_process_subprocess_begin"
    });
    let process_exit_code = execute_process(Some(main_args), None, sandbox_info);
    crate::bootstrap_trace::record(&format!("cef_execute_process_returned {process_exit_code}"));
    if !is_browser_process {
        return (process_exit_code >= 0)
            .then_some(())
            .ok_or_else(|| "cef_subprocess_failed".to_owned());
    }
    if process_exit_code != -1 {
        return Err("cef_browser_process_routed_as_subprocess".into());
    }

    let options = HostOptions::from_process()?;
    crate::bootstrap_trace::record("cef_host_options_ready");
    std::fs::create_dir_all(&options.profile_dir).map_err(|error| error.to_string())?;
    let sink = EventSink::default();
    let manager = TabManager::new(options.parent_hwnd, sink.clone());
    let mut app = HachimiCefApp::new(manager.clone(), sink.clone());
    let profile_dir = CefString::from(options.profile_dir.to_string_lossy().as_ref());
    let log_file = CefString::from(options.log_file.to_string_lossy().as_ref());
    let settings = Settings {
        no_sandbox: (!cfg!(feature = "sandbox")).into(),
        multi_threaded_message_loop: 1,
        cache_path: profile_dir.clone(),
        root_cache_path: profile_dir,
        persist_session_cookies: 1,
        log_file,
        log_severity: LogSeverity::WARNING,
        ..Settings::default()
    };
    crate::bootstrap_trace::record("cef_initialize_begin");
    if initialize(
        Some(main_args),
        Some(&settings),
        Some(&mut app),
        sandbox_info,
    ) != 1
    {
        sink.fatal("cef_initialize_failed", "CEF initialization returned false");
        return Err("cef_initialize_failed".into());
    }
    crate::bootstrap_trace::record("cef_initialize_completed");

    let shutdown_requested = run_command_loop(manager.clone(), &sink);
    crate::bootstrap_trace::record("cef_command_loop_completed");
    if !shutdown_requested {
        manager.close_all();
    }
    for _ in 0..500 {
        if manager.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    shutdown();
    Ok(())
}
