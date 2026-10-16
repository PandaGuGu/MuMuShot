//! MuMuShot —— 截电脑屏幕并上传到 MuMu 模拟器相册（Rust 版）。
//!
//! 功能与 Python 版对齐：命令行 / 图形面板 / 全局热键三种用法。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adb;
mod app;
mod capture;
mod core;
mod gui;
mod hotkeys;

use std::time::Duration;

use clap::Parser;

use app::Engine;

/// 截取电脑屏幕并上传到 MuMu 模拟器相册。
#[derive(Parser, Debug, Clone)]
#[command(
    name = "MuMuShot",
    about = "截取电脑屏幕并上传到 MuMu 模拟器相册",
    disable_help_subcommand = true
)]
pub struct Args {
    /// 全屏截图上传（默认动作）
    #[arg(long)]
    pub full: bool,

    /// 框选截图；给坐标表示固定区域，如 100,100,800,600
    #[arg(long, value_name = "x0,y0,x1,y1", num_args = 0..=1, default_missing_value = "select")]
    pub region: Option<String>,

    /// 调起系统截图工具框选后上传
    #[arg(long)]
    pub snip: bool,

    /// 等待系统截图完成的秒数，默认 90
    #[arg(long, default_value_t = 90.0)]
    pub snip_timeout: f64,

    /// 上传剪贴板里的图片
    #[arg(long)]
    pub clipboard: bool,

    /// 截取标题包含该关键词的窗口
    #[arg(long, value_name = "关键词")]
    pub window: Option<String>,

    /// 列出所有窗口标题
    #[arg(long)]
    pub list_windows: bool,

    /// 强制用控制台模式（无图形面板）
    #[arg(long)]
    pub console: bool,

    /// 自检：验证依赖与日志功能后退出
    #[arg(long)]
    pub selftest: bool,

    /// 上传后自动在模拟器里打开图片
    #[arg(long)]
    pub open_gallery: bool,

    /// 截图瞬间隐藏面板
    #[arg(long)]
    pub hide_panel_on_shot: bool,

    /// 模拟器相册目录，默认 /sdcard/Pictures/MuMuShot
    #[arg(long, default_value = core::DEFAULT_REMOTE_DIR)]
    pub remote_dir: String,

    /// 手动指定 adb.exe 路径
    #[arg(long)]
    pub adb: Option<String>,

    /// 手动指定设备，如 127.0.0.1:7555
    #[arg(long)]
    pub device: Option<String>,
}

fn main() {
    let args = Args::parse();
    capture::enable_dpi_awareness();

    // Python 版把整个 main 包在 try/except 里兜底弹框，这里等价处理。
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch(args)));
    let code = match result {
        Ok(c) => c,
        Err(_) => {
            core::show_error("程序遇到未预期的错误，详见日志文件", "MuMuShot - 出错了");
            1
        }
    };
    std::process::exit(code);
}

fn dispatch(args: Args) -> i32 {
    if args.selftest {
        return selftest();
    }
    if args.list_windows {
        list_windows();
        return 0;
    }

    let headless = args.full
        || args.clipboard
        || args.snip
        || args.window.is_some()
        || args.region.is_some();
    if headless {
        return run_cli(&args);
    }
    if args.console {
        return run_console(&args);
    }
    gui::run_gui(args)
}

/// 解析 --region 的值：空串 / "select" = 交互框选，否则是 x0,y0,x1,y1。
fn parse_region(v: Option<&String>) -> Result<Option<(i32, i32, i32, i32)>, String> {
    let Some(s) = v else { return Ok(None) };
    if s.is_empty() || s == "select" {
        return Ok(None);
    }
    let clean = s.replace(' ', "");
    let parts: Vec<&str> = clean.split(',').collect();
    if parts.len() == 4 {
        let nums: Option<Vec<i32>> = parts.iter().map(|p| p.parse::<i32>().ok()).collect();
        if let Some(n) = nums {
            return Ok(Some((n[0], n[1], n[2], n[3])));
        }
    }
    Err("  [x] --region 格式应为 x0,y0,x1,y1，或直接写 --region 表示交互框选".to_string())
}

/// CLI：一次性截图并上传。
fn run_cli(args: &Args) -> i32 {
    let mut up = Engine::new(
        args.adb.clone(),
        args.device.clone(),
        args.remote_dir.clone(),
    );
    if !up.connect() {
        return 3;
    }

    let snip_timeout = Duration::from_secs_f64(args.snip_timeout);
    let (img, tag) = if let Some(region) = &args.region {
        match parse_region(Some(region)) {
            Ok(Some(bbox)) => (
                capture::grab_bbox(bbox.0, bbox.1, bbox.2, bbox.3).ok(),
                "Region",
            ),
            Ok(None) => (gui::region_shot_standalone(), "Region"),
            Err(msg) => {
                core::log(&msg);
                return 2;
            }
        }
    } else if args.snip {
        (capture::snip_via_system(snip_timeout), "Snip")
    } else if args.clipboard {
        (capture::grab_clipboard(), "Clip")
    } else if let Some(kw) = &args.window {
        let (img, _rect) = app::capture_window(kw);
        if img.is_none() {
            core::log(&format!(
                "  [x] 没找到标题包含「{kw}」的窗口，用 --list-windows 看看"
            ));
            return 5;
        }
        (img, "Win")
    } else {
        (capture::grab_full().ok(), "Shot")
    };

    if up.send(img, tag, args.open_gallery) {
        0
    } else {
        4
    }
}

/// 控制台模式：没有图形面板时的降级方案，热键照常用。
pub fn run_console(args: &Args) -> i32 {
    let mut up = Engine::new(
        args.adb.clone(),
        args.device.clone(),
        args.remote_dir.clone(),
    );
    let hk = hotkeys::Hotkeys::start();

    core::log("MuMuShot（控制台模式）");
    for (action, label) in hotkeys::HOTKEY_LIST {
        let mark = if hk.failed.iter().any(|f| f == action) {
            "×"
        } else {
            "√"
        };
        core::log(&format!("  [{mark}] {label}"));
    }
    core::log("  这个窗口别关，关掉热键就失效了。");
    core::log("");

    let snip_timeout = Duration::from_secs_f64(args.snip_timeout);
    loop {
        if let Some(action) = hotkeys::poll_action() {
            let (img, tag) = app::capture_by_action(action, None, snip_timeout);
            up.send(img, tag, args.open_gallery);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 列出所有窗口标题。
fn list_windows() {
    for w in capture::list_windows() {
        core::log(&format!("  {:>5}×{:<5}  {}", w.rect.2, w.rect.3, w.title));
    }
}

/// 自检：确认依赖都在、目录可写、日志能落盘。
fn selftest() -> i32 {
    core::log(&format!(
        "自检开始：Rust 版，程序目录 {}",
        core::app_dir().display()
    ));

    let mut ok = true;

    // 1) 截图模块（GDI）可用
    match capture::grab_bbox(0, 0, 1, 1) {
        Ok(img) if img.width() == 1 => core::log("  [√] 截图模块（GDI）"),
        Ok(_) => {
            core::log("  [x] 截图模块返回尺寸异常");
            ok = false;
        }
        Err(e) => {
            core::log(&format!("  [x] 截图模块：{e}"));
            ok = false;
        }
    }

    // 2) 输出目录可写
    let dir = core::ensure_dir(&core::shots_dir());
    let probe = dir.join(".mumushot_write_probe");
    match std::fs::write(&probe, b"ok") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            core::log(&format!("  [√] 输出目录可写：{}", dir.display()));
        }
        Err(e) => {
            core::log(&format!("  [x] 输出目录不可写：{e}"));
            ok = false;
        }
    }

    // 3) 日志能落盘
    let log = core::log_path();
    if log.exists() {
        core::log(&format!("  [√] 日志文件：{}", log.display()));
    } else {
        core::log("  [x] 日志文件没能创建");
        ok = false;
    }

    // 4) 能枚举窗口
    let wins = capture::list_windows();
    core::log(&format!("  [√] 窗口枚举可用（{} 个）", wins.len()));

    core::log(if ok { "自检通过" } else { "自检失败" });
    // 给 CI 留一个纯 ASCII 标记，避免日志匹配受编码影响
    core::log(if ok { "selftest ok" } else { "selftest failed" });
    if ok { 0 } else { 1 }
}
