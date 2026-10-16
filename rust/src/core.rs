//! 公共能力：路径、日志、错误弹框、目录创建。
//!
//! 对应 Python 版 `mumu_core` 中的基础部分，被 CLI 与图形面板共用。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Local;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::PCWSTR;

/// 模拟器相册目录（默认值）。
pub const DEFAULT_REMOTE_DIR: &str = "/sdcard/Pictures/MuMuShot";
/// 日志文件超过此大小就轮转一次（与 Python 版一致）。
const LOG_MAX_BYTES: u64 = 256 * 1024;

/// 输出目录不可写时的临时目录名（与 Python 版一致）。
pub const TEMP_DIR_NAME: &str = "MuMuTool";

/// 打包后取 exe 所在目录；开发时取当前工作目录。
pub fn app_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.to_path_buf();
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// 日志文件路径：exe 旁边。
pub fn log_path() -> PathBuf {
    app_dir().join("mumu-tool.log")
}

/// 本机留档目录：exe 旁边的 `shots/`。
pub fn shots_dir() -> PathBuf {
    app_dir().join("shots")
}

static LOG_LOCK: Mutex<()> = Mutex::new(());

/// 同时写到控制台和日志文件；日志超限时先轮转为 `.old`。
///
/// 注意：双击启动（没有控制台）时 stdout 句柄是空的，`println!` 会直接 panic，
/// 所以这里用不会 panic 的写法，失败就当没写成。
pub fn log(msg: &str) {
    let line = format!("{}  {}", Local::now().format("%m-%d %H:%M:%S"), msg);
    {
        let mut out = std::io::stdout();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    let _guard = LOG_LOCK.lock();
    let path = log_path();
    if let Ok(md) = fs::metadata(&path) {
        if md.len() > LOG_MAX_BYTES {
            let old = PathBuf::from(format!("{}.old", path.display()));
            let _ = fs::remove_file(&old);
            let _ = fs::rename(&path, &old);
        }
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{line}");
    }
}

/// 创建一个目录；失败则退到临时目录。
pub fn ensure_dir(path: &PathBuf) -> PathBuf {
    if fs::create_dir_all(path).is_ok() {
        return path.clone();
    }
    let alt = std::env::temp_dir().join(TEMP_DIR_NAME);
    if fs::create_dir_all(&alt).is_ok() {
        return alt;
    }
    std::env::temp_dir()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 自己有没有控制台窗口。
pub fn has_console() -> bool {
    unsafe {
        let h = windows::Win32::System::Console::GetConsoleWindow();
        !h.0.is_null()
    }
}

/// 没有控制台（打包成窗口版 exe）时，用弹框把错误告诉用户。
///
/// 有控制台时用户能直接看到日志，就不再弹框打断。
pub fn show_error(msg: &str, title: &str) {
    log(&format!("  [x] {msg}"));
    if has_console() {
        return;
    }
    let text = wide(msg);
    let cap = wide(title);
    unsafe {
        MessageBoxW(
            Some(HWND(std::ptr::null_mut())),
            PCWSTR(text.as_ptr()),
            PCWSTR(cap.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}
