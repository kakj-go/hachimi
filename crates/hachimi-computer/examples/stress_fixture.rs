#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(not(windows))]
fn main() {
    eprintln!("hachimi-computer stress fixture requires Windows");
}

#[cfg(windows)]
#[allow(unsafe_op_in_unsafe_fn)]
mod windows_fixture {
    use std::{
        ffi::c_void,
        sync::atomic::{AtomicPtr, AtomicUsize, Ordering},
        thread,
    };

    use windows::{
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                Input::KeyboardAndMouse::SetFocus,
                WindowsAndMessaging::{
                    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW,
                    DispatchMessageW, GetMessageW, HMENU, IDC_ARROW, LoadCursorW, MSG, MoveWindow,
                    PostQuitMessage, RegisterClassW, SW_SHOW, SetWindowTextW, ShowWindow,
                    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_CREATE,
                    WM_DESTROY, WM_SIZE, WNDCLASSW, WS_BORDER, WS_CHILD, WS_EX_CLIENTEDGE,
                    WS_OVERLAPPEDWINDOW, WS_VISIBLE, WS_VSCROLL,
                },
            },
        },
        core::w,
    };

    const EDIT_ID: usize = 1;
    const BUTTON_ID: usize = 2;
    static EDIT_HANDLE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_CREATE => {
                let edit = CreateWindowExW(
                    WS_EX_CLIENTEDGE,
                    w!("EDIT"),
                    w!("Hachimi Computer fixture ready"),
                    WS_CHILD | WS_VISIBLE | WS_BORDER | WS_VSCROLL | WINDOW_STYLE(0x0004),
                    12,
                    12,
                    560,
                    32,
                    Some(hwnd),
                    Some(HMENU(EDIT_ID as *mut c_void)),
                    Some(HINSTANCE::default()),
                    Some(std::ptr::null()),
                )
                .expect("create edit control");
                let _button = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("BUTTON"),
                    w!("Increment counter"),
                    WS_CHILD | WS_VISIBLE,
                    12,
                    56,
                    180,
                    34,
                    Some(hwnd),
                    Some(HMENU(BUTTON_ID as *mut c_void)),
                    Some(HINSTANCE::default()),
                    Some(std::ptr::null()),
                )
                .expect("create counter button");
                EDIT_HANDLE.store(edit.0, Ordering::Release);
                let _ = SetFocus(Some(edit));
                LRESULT(0)
            }
            WM_COMMAND if (wparam.0 & 0xffff) == BUTTON_ID => {
                let edit = HWND(EDIT_HANDLE.load(Ordering::Acquire));
                let count = COUNTER.fetch_add(1, Ordering::AcqRel) + 1;
                let text = format!("Hachimi Computer fixture counter {count}");
                let wide = text
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect::<Vec<_>>();
                let _ = SetWindowTextW(edit, windows::core::PCWSTR(wide.as_ptr()));
                LRESULT(0)
            }
            WM_SIZE => {
                let width = (lparam.0 as u32 & 0xffff) as i32;
                let edit = HWND(EDIT_HANDLE.load(Ordering::Acquire));
                let _ = MoveWindow(edit, 12, 12, (width - 24).max(200), 32, true);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    pub fn run() {
        unsafe {
            let module = GetModuleHandleW(None).expect("module handle");
            let instance = HINSTANCE(module.0);
            let class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW).expect("cursor"),
                lpszClassName: w!("HachimiComputerStressFixture"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let window = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("HachimiComputerStressFixture"),
                w!("Hachimi Computer Stress Fixture"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                640,
                480,
                None,
                None,
                Some(instance),
                Some(std::ptr::null()),
            )
            .expect("create stress fixture window");
            let _ = ShowWindow(window, SW_SHOW);
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        thread::yield_now();
    }
}

#[cfg(windows)]
fn main() {
    windows_fixture::run();
}
