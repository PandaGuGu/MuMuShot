//! 在工作线程里注册系统级热键，触发后把动作名丢进队列。
//!
//! 对应 Python 版 `mumu_shot` 的 `Hotkeys` 类。全局热键用 RegisterHotKey，
//! 必须在带消息循环的线程上创建，所以这里单开一个线程常驻。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

/// 热键清单：(动作名, 展示文案)。与 Python 版 `HOTKEY_LIST` 一致。
pub const HOTKEY_LIST: &[(&str, &str)] = &[
    ("region", "Ctrl+Alt+A  框选截图"),
    ("snip", "Ctrl+Alt+Q  系统截图工具框选"),
    ("full", "Ctrl+Alt+S  全屏截图"),
    ("clip", "Ctrl+Alt+C  上传剪贴板图片"),
];

/// 热键被占用时给用户看的短名。
pub fn short_name(action: &str) -> &'static str {
    match action {
        "region" => "框选",
        "snip" => "系统截图",
        "full" => "全屏",
        "clip" => "剪贴板",
        other => Box::leak(other.to_string().into_boxed_str()),
    }
}

fn planned() -> [(Code, &'static str); 4] {
    [
        (Code::KeyA, "region"),
        (Code::KeyQ, "snip"),
        (Code::KeyS, "full"),
        (Code::KeyC, "clip"),
    ]
}

fn mods() -> Modifiers {
    Modifiers::CONTROL | Modifiers::ALT
}

fn id_of(code: Code) -> u32 {
    HotKey::new(Some(mods()), code).id()
}

fn action_of_id(id: u32) -> Option<&'static str> {
    planned()
        .iter()
        .find(|(code, _)| id_of(*code) == id)
        .map(|(_, a)| *a)
}

/// 全局热键句柄：持有工作线程，退出时停掉。
pub struct Hotkeys {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    /// 注册失败的热键（被别的软件占用了）
    pub failed: Vec<String>,
}

impl Hotkeys {
    /// 启动热键线程，返回句柄。注册失败的会记在 `failed` 里。
    pub fn start() -> Hotkeys {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Vec<String>>();

        let handle = thread::spawn(move || {
            let manager = match GlobalHotKeyManager::new() {
                Ok(m) => m,
                Err(_) => {
                    let names: Vec<String> =
                        HOTKEY_LIST.iter().map(|(a, _)| a.to_string()).collect();
                    let _ = tx.send(names);
                    return;
                }
            };
            let mut registered: Vec<HotKey> = Vec::new();
            let mut failed: Vec<String> = Vec::new();
            for (code, action) in planned() {
                let hk = HotKey::new(Some(mods()), code);
                match manager.register(hk) {
                    Ok(()) => registered.push(hk),
                    Err(_) => failed.push(action.to_string()),
                }
            }
            let _ = tx.send(failed);

            unsafe {
                let mut msg = MSG::default();
                while !stop_thread.load(Ordering::Relaxed) {
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }
            for hk in registered {
                let _ = manager.unregister(hk);
            }
        });

        let failed = rx.recv_timeout(Duration::from_secs(3)).unwrap_or_default();
        Hotkeys {
            stop,
            handle: Some(handle),
            failed,
        }
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// 当前按住的键。RegisterHotKey 没带 MOD_NOREPEAT，按住会反复触发，
/// 这里自己按「按下 / 松开」去重，等价于 Python 版的 NOREPEAT 语义。
static HELD: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// 取一个待处理的热键动作（非阻塞）。按住不放只触发一次。
pub fn poll_action() -> Option<&'static str> {
    let mut pressed: Option<&'static str> = None;
    while let Ok(ev) = GlobalHotKeyEvent::receiver().try_recv() {
        let Some(action) = action_of_id(ev.id) else {
            continue;
        };
        let Ok(mut held) = HELD.lock() else { continue };
        match ev.state {
            HotKeyState::Pressed => {
                if !held.contains(&action) {
                    held.push(action);
                    pressed = Some(action);
                }
            }
            HotKeyState::Released => {
                held.retain(|a| *a != action);
            }
        }
    }
    pressed
}

/// 消耗掉历史事件（避免启动时补触发）。
pub fn drain() {
    while GlobalHotKeyEvent::receiver().try_recv().is_ok() {}
    if let Ok(mut held) = HELD.lock() {
        held.clear();
    }
}
