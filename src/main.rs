mod helpers;
mod models;
mod settings;
mod state;
mod themes;

use models::Input;
use std::io::Read;
use themes::Theme;

const DEFAULT_TERMINAL_COLS: usize = 120;
const CLAUDE_UI_RIGHT_MARGIN: usize = 4;

fn parse_columns(value: &str) -> Option<usize> {
    value.parse::<usize>().ok().filter(|&cols| cols > 0)
}

fn columns_from_env() -> Option<usize> {
    std::env::var("COLUMNS").ok().and_then(|value| parse_columns(&value))
}

fn render_width(manual_width: Option<usize>, detected_width: usize) -> usize {
    manual_width.unwrap_or_else(|| {
        // Claude Code 會在右側保留內建留白或通知區；填滿 COLUMNS 會讓尾端被省略。
        detected_width.saturating_sub(CLAUDE_UI_RIGHT_MARGIN).max(1)
    })
}

#[cfg(unix)]
fn parent_id() -> u32 {
    std::os::unix::process::parent_id()
}

#[cfg(windows)]
fn parent_id() -> u32 {
    std::process::id()
}

/// Detect terminal width via /dev/tty (bypasses stdin/stdout/stderr redirection)
#[cfg(unix)]
fn terminal_cols() -> usize {
    use std::os::unix::io::AsRawFd;

    #[repr(C)]
    struct Winsize {
        ws_row: u16,
        ws_col: u16,
        ws_xpixel: u16,
        ws_ypixel: u16,
    }
    unsafe extern "C" {
        fn ioctl(
            fd: std::os::raw::c_int,
            request: std::os::raw::c_ulong,
            ...
        ) -> std::os::raw::c_int;
    }
    #[cfg(target_os = "macos")]
    const TIOCGWINSZ: std::os::raw::c_ulong = 0x40087468;
    #[cfg(not(target_os = "macos"))]
    const TIOCGWINSZ: std::os::raw::c_ulong = 0x5413;

    if let Some(cols) = columns_from_env() {
        return cols;
    }

    // 狀態列的標準輸出會被 Claude Code 接管，因此必須直接查詢控制終端。
    if let Ok(tty) = std::fs::File::open("/dev/tty") {
        let fd = tty.as_raw_fd();
        unsafe {
            let mut ws = std::mem::zeroed::<Winsize>();
            if ioctl(fd, TIOCGWINSZ, &mut ws as *mut Winsize) == 0 && ws.ws_col > 0 {
                return ws.ws_col as usize;
            }
        }
    }
    DEFAULT_TERMINAL_COLS
}

#[cfg(windows)]
fn terminal_cols() -> usize {
    use std::ffi::c_void;

    #[repr(C)]
    struct Coord {
        x: i16,
        y: i16,
    }

    #[repr(C)]
    struct SmallRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }

    #[repr(C)]
    struct ConsoleScreenBufferInfo {
        size: Coord,
        cursor_position: Coord,
        attributes: u16,
        window: SmallRect,
        maximum_window_size: Coord,
    }

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            file_name: *const u16,
            desired_access: u32,
            share_mode: u32,
            security_attributes: *mut c_void,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: *mut c_void,
        ) -> *mut c_void;
        fn GetConsoleScreenBufferInfo(
            console_output: *mut c_void,
            info: *mut ConsoleScreenBufferInfo,
        ) -> i32;
        fn CloseHandle(object: *mut c_void) -> i32;
    }

    if let Some(cols) = columns_from_env() {
        return cols;
    }

    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const OPEN_EXISTING: u32 = 3;
    const INVALID_HANDLE_VALUE: *mut c_void = -1isize as *mut c_void;

    let console_name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
    unsafe {
        // stdout 通常是 pipe；開啟 CONOUT$ 才能取得實際的 Windows Console 視窗寬度。
        let handle = CreateFileW(
            console_name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if handle != INVALID_HANDLE_VALUE {
            let mut info = std::mem::zeroed::<ConsoleScreenBufferInfo>();
            let ok = GetConsoleScreenBufferInfo(handle, &mut info);
            let _ = CloseHandle(handle);
            if ok != 0 {
                let width = i32::from(info.window.right) - i32::from(info.window.left) + 1;
                if width > 0 {
                    return width as usize;
                }
            }
        }
    }

    DEFAULT_TERMINAL_COLS
}

fn print_help() {
    eprintln!("arcade-statusline — Arcade-themed statusline for Claude Code");
    eprintln!();
    eprintln!("Usage: arcade-statusline [OPTIONS]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --theme <NAME>       Theme: pacman (default), pikmin");
    eprintln!("  --width <N>          Override statusline width (auto-detected)");
    eprintln!("  --narrow-emoji       Force narrow emoji mode (auto-detected for JetBrains)");
    eprintln!("  --buddy-padding      Add padding lines around statusline for Claude Code");
    eprintln!("                       buddy companion alignment in narrow-emoji terminals.");
    eprintln!("                       Adds 1 line above and 5 lines below the statusline");
    eprintln!("                       so the buddy sprite doesn't overlap with content.");
    eprintln!("  --slot-offset <N>    Shift time slots by N (demo/debug)");
    eprintln!("  --help               Show this help");
    eprintln!();
    eprintln!("Example (settings.json):");
    eprintln!(r#"  "statusLine": {{"#);
    eprintln!(r#"    "type": "command","#);
    eprintln!(r#"    "command": "arcade-statusline --theme pikmin --buddy-padding""#);
    eprintln!(r#"  }}"#);
}

fn main() {
    // Parse arguments
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }

    let theme_name = args.iter()
        .position(|a| a == "--theme")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or("pacman");

    let demo_mode = args.iter()
        .position(|a| a == "--slot-offset")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<i64>().ok());

    let manual_width = args.iter()
        .position(|a| a == "--width")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<usize>().ok());

    // Detect narrow-emoji terminals (JetBrains JediTerm renders emoji as 1 col instead of 2)
    let narrow_emoji = args.iter().any(|a| a == "--narrow-emoji")
        || std::env::var("TERMINAL_EMULATOR")
            .is_ok_and(|v| v.contains("JetBrains"));

    let buddy_padding = args.iter().any(|a| a == "--buddy-padding") && narrow_emoji;

    if let Some(offset) = demo_mode {
        state::set_slot_offset(offset);
    }

    // Read JSON from stdin
    let mut input_str = String::new();
    std::io::stdin().read_to_string(&mut input_str).unwrap_or_default();

    let input: Input = serde_json::from_str(&input_str).unwrap_or_default();

    let mut raw_state = state::load();

    if demo_mode.is_none() {
        // Normal mode: update and persist state
        let session_id = parent_id().to_string();
        state::update(&mut raw_state, &input, &session_id);
        state::save(&raw_state);
    }
    // Demo mode: read-only, don't modify state

    let max_width = render_width(manual_width, terminal_cols());

    // Render with selected theme
    let output = match theme_name {
        "pikmin" => themes::pikmin::Pikmin { narrow_emoji, buddy_padding }.render(&input, &raw_state, max_width),
        "pacman" | _ => themes::pacman::Pacman.render(&input, &raw_state, max_width),
    };

    println!("{output}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_columns_values_are_ignored() {
        assert_eq!(parse_columns(""), None);
        assert_eq!(parse_columns("0"), None);
        assert_eq!(parse_columns("abc"), None);
    }

    #[test]
    fn valid_columns_value_is_accepted() {
        assert_eq!(parse_columns("96"), Some(96));
    }

    #[test]
    fn automatic_width_reserves_space_for_claude_ui() {
        assert_eq!(render_width(None, 50), 50 - CLAUDE_UI_RIGHT_MARGIN);
    }

    #[test]
    fn manual_width_is_used_without_a_margin() {
        assert_eq!(render_width(Some(50), 120), 50);
    }
}
