//! 截图能力：全屏 / 区域 / 剪贴板 / 指定窗口 / 系统截图工具框选。
//!
//! 对应 Python 版 `mumu_shot` 里的抓图部分。全部走 Win32 GDI，
//! 不引入额外截图库，二进制更小、启动更快。

use std::ffi::c_void;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

pub use image::RgbaImage;
use image::{ImageBuffer, Rgba};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HGDIOBJ, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::HiDpi::{PROCESS_PER_MONITOR_DPI_AWARE, SetProcessDpiAwareness};
use windows::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, keybd_event};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetClassNameW, GetSystemMetrics, GetWindowLongW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, IsIconic, IsWindowVisible, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use crate::core;

/// 这些是桌面壳 / 托盘窗口，不该出现在窗口列表里。
const SKIP_WINDOW_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "%ApplicationManager_DesktopShellWindow",
    "Windows.UI.Core.CoreWindow",
];

/// 一个可截取的顶层窗口。
#[derive(Clone, Debug)]
pub struct WinInfo {
    pub title: String,
    /// 窗口句柄，留给后续需要精确定位窗口的场景
    #[allow(dead_code)]
    pub hwnd: isize,
    /// (x, y, width, height)，屏幕物理像素
    pub rect: (i32, i32, i32, i32),
}

/// 让截图坐标和屏幕物理像素一一对应，避免缩放屏截歪。
pub fn enable_dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE);
    }
}

/// 整个虚拟桌面（含多显示器）的 (left, top, width, height)。
pub fn virtual_screen_rect() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// 用 GDI BitBlt 抓取虚拟桌面上的一块矩形（坐标为虚拟桌面坐标）。
fn capture_rect(x: i32, y: i32, w: i32, h: i32) -> Result<RgbaImage, String> {
    if w <= 0 || h <= 0 {
        return Err("截图区域为空".to_string());
    }
    unsafe {
        let hdc_screen = GetDC(None);
        if hdc_screen.0.is_null() {
            return Err("GetDC(屏幕) 失败".to_string());
        }
        let hdc_mem = CreateCompatibleDC(Some(hdc_screen));
        let hbmp = CreateCompatibleBitmap(hdc_screen, w, h);
        if hbmp.0.is_null() {
            ReleaseDC(None, hdc_screen);
            return Err("CreateCompatibleBitmap 失败".to_string());
        }
        let old = SelectObject(hdc_mem, HGDIOBJ(hbmp.0));

        let blt = BitBlt(hdc_mem, 0, 0, w, h, Some(hdc_screen), x, y, SRCCOPY);

        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h; // 负数 = 自上而下
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB.0;

        let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
        let lines = GetDIBits(
            hdc_mem,
            hbmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut c_void),
            &mut bi,
            DIB_RGB_COLORS,
        );

        SelectObject(hdc_mem, old);
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);

        if blt.is_err() {
            return Err("BitBlt 失败".to_string());
        }
        if lines == 0 {
            return Err("GetDIBits 失败".to_string());
        }

        // GDI 给的是 BGRA，转成 RGBA；屏幕抓图的 alpha 无意义，统一置为不透明
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(w as u32, h as u32, buf)
            .ok_or_else(|| "构造图像失败".to_string())
    }
}

/// 全屏（所有显示器拼起来的虚拟桌面）。
pub fn grab_full() -> Result<RgbaImage, String> {
    let (l, t, w, h) = virtual_screen_rect();
    capture_rect(l, t, w, h)
}

/// 按虚拟桌面坐标抓一块区域，(x0,y0,x1,y1) 左上右下。
pub fn grab_bbox(x0: i32, y0: i32, x1: i32, y1: i32) -> Result<RgbaImage, String> {
    let (l, t, _, _) = virtual_screen_rect();
    let left = x0.min(x1) - l;
    let top = y0.min(y1) - t;
    let w = (x1 - x0).abs();
    let h = (y1 - y0).abs();
    capture_rect(l + left, t + top, w, h)
}

/// 窗口的真实可见边界（去掉 Win10+ 的透明外框）；宽高非正返回 None。
fn window_rect(hwnd: HWND) -> Option<(i32, i32, i32, i32)> {
    unsafe {
        let mut rc = RECT::default();
        // Win10+ 用 DWM 的真实可见边界，去掉透明外框
        let ok = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rc as *mut _ as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        );
        if ok.is_err() {
            let _ = GetWindowRect(hwnd, &mut rc);
        }
        if rc.right - rc.left <= 0 || rc.bottom - rc.top <= 0 {
            return None;
        }
        Some((rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top))
    }
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = unsafe { &mut *(lparam.0 as *mut Vec<WinInfo>) };
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return BOOL(1);
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return BOOL(1);
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n <= 0 {
            return BOOL(1);
        }
        let title = String::from_utf16_lossy(&buf[..n as usize]).trim().to_string();
        if title.is_empty() {
            return BOOL(1);
        }

        let mut cbuf = vec![0u16; 256];
        let cn = GetClassNameW(hwnd, &mut cbuf);
        let class = String::from_utf16_lossy(&cbuf[..cn.max(0) as usize]);
        if SKIP_WINDOW_CLASSES.contains(&class.as_str()) {
            return BOOL(1);
        }
        // 工具窗口（悬浮面板、托盘图标之类）不算可截取的窗口
        if GetWindowLongW(hwnd, GWL_EXSTYLE) & (WS_EX_TOOLWINDOW.0 as i32) != 0 {
            return BOOL(1);
        }

        if let Some(rect) = window_rect(hwnd) {
            out.push(WinInfo {
                title,
                hwnd: hwnd.0 as isize,
                rect,
            });
        }
    }
    BOOL(1)
}

/// 所有可见且带标题的顶层窗口，按面积从大到小。
pub fn list_windows() -> Vec<WinInfo> {
    let mut out: Vec<WinInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut out as *mut _ as isize));
    }
    out.sort_by(|a, b| {
        let aa = (a.rect.2 as i64) * (a.rect.3 as i64);
        let bb = (b.rect.2 as i64) * (b.rect.3 as i64);
        bb.cmp(&aa)
    });
    out
}

/// 截取标题包含关键词的窗口（截屏幕对应区域，和 Python 版一致）。
pub fn grab_window(keyword: &str) -> Result<RgbaImage, String> {
    let kw = keyword.to_lowercase();
    let wins = list_windows();
    let hit = wins
        .iter()
        .find(|w| w.title.to_lowercase().contains(&kw))
        .ok_or_else(|| format!("没找到标题包含「{keyword}」的窗口"))?;
    let (x, y, w, h) = hit.rect;
    capture_rect(x, y, w, h)
}

/// 剪贴板里的图片。优先直接取位图，其次取复制的图片文件。
pub fn grab_clipboard() -> Option<RgbaImage> {
    // 1) 直接是位图
    if let Ok(mut cb) = arboard::Clipboard::new() {
        if let Ok(img) = cb.get_image() {
            if img.width > 0 && img.height > 0 {
                if let Some(mut rgba) = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(
                    img.width as u32,
                    img.height as u32,
                    img.bytes.into_owned(),
                ) {
                    force_opaque(&mut rgba);
                    return Some(rgba);
                }
            }
        }
    }
    // 2) 复制的是图片文件（资源管理器里复制文件）
    for f in clipboard_files() {
        let p = PathBuf::from(&f);
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "bmp" | "webp" | "gif") {
            if let Ok(img) = image::open(&p) {
                let mut rgba = img.to_rgba8();
                force_opaque(&mut rgba);
                return Some(rgba);
            }
        }
    }
    None
}

/// 把 alpha 全部置为不透明（Python 版是 convert("RGB")，等价做法）。
fn force_opaque(img: &mut RgbaImage) {
    for px in img.as_mut().chunks_exact_mut(4) {
        px[3] = 255;
    }
}

/// 读取剪贴板里的文件列表（CF_HDROP）。
fn clipboard_files() -> Vec<String> {
    let mut files = Vec::new();
    unsafe {
        if OpenClipboard(None).is_err() {
            return files;
        }
        if let Ok(h) = GetClipboardData(CF_HDROP.0 as u32) {
            let hdrop = HDROP(h.0);
            let count = DragQueryFileW(hdrop, 0xFFFF_FFFF, None);
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, None);
                if len == 0 {
                    continue;
                }
                let mut buf = vec![0u16; (len + 1) as usize];
                let got = DragQueryFileW(hdrop, i, Some(&mut buf));
                if got > 0 {
                    files.push(String::from_utf16_lossy(&buf[..got as usize]));
                }
            }
        }
        let _ = CloseClipboard();
    }
    files
}

/// 剪贴板序号，用来判断剪贴板有没有变化。
pub fn clipboard_seq() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// 模拟 Win+Shift+S（系统截图工具的备用触发方式）。
pub fn send_win_shift_s() {
    const VK_LWIN: u8 = 0x5B;
    const VK_SHIFT: u8 = 0x10;
    unsafe {
        keybd_event(VK_LWIN, 0, Default::default(), 0);
        keybd_event(VK_SHIFT, 0, Default::default(), 0);
        keybd_event(0x53, 0, Default::default(), 0); // 'S'
        keybd_event(0x53, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(VK_SHIFT, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(VK_LWIN, 0, KEYEVENTF_KEYUP, 0);
    }
}

/// 调起系统截图工具（优先 ms-screenclip: 协议，失败就模拟热键）。
pub fn trigger_snip() -> bool {
    let ok = Command::new("explorer.exe")
        .arg("ms-screenclip:")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok();
    if ok {
        return true;
    }
    send_win_shift_s();
    true
}

/// 让系统截图工具接管框选，完成后从剪贴板取图。
pub fn snip_via_system(timeout: Duration) -> Option<RgbaImage> {
    let seq0 = clipboard_seq();
    if !trigger_snip() {
        core::log("  [x] 没能调起系统截图工具");
        return None;
    }
    core::log("  已调起系统截图工具，请框选要上传的区域（ESC 取消）…");

    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        thread::sleep(Duration::from_millis(250));
        if clipboard_seq() == seq0 {
            continue;
        }
        // 剪贴板变了，等截图工具把文件也落盘
        thread::sleep(Duration::from_millis(300));
        for _ in 0..10 {
            if let Some(img) = grab_clipboard() {
                return Some(img);
            }
            thread::sleep(Duration::from_millis(300));
        }
        thread::sleep(Duration::from_millis(400));
    }
    core::log("  [!] 等待系统截图超时，已取消");
    None
}

/// 把 RgbaImage 存成 PNG（按 Python 版口径转成 RGB，不带 alpha 通道）。
pub fn save_png(img: &RgbaImage, path: &std::path::Path) -> Result<(), String> {
    let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    rgb.save_with_format(path, image::ImageFormat::Png)
        .map_err(|e| e.to_string())
}
