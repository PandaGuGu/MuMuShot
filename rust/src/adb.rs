//! 安卓模拟器的 adb 公共能力 —— `mumu_core.py` 的 Rust 对应实现。
//!
//! 自动找到 MuMu 模拟器并连上，把本地图片推进模拟器相册，
//! 并触发 MediaStore 索引，让图库立即可见。
//!
//! 识别顺序、超时、错误文案都对齐 Python 版，便于两边对拍。

use std::fs;
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::core;

/// MuMu 常见安装根目录。
const MUMU_ROOTS: &[&str] = &[
    r"F:\Program Files\Netease\MuMu",
    r"C:\Program Files\Netease\MuMu",
    r"C:\Program Files (x86)\Netease\MuMu",
    r"D:\Program Files\Netease\MuMu",
    r"E:\Program Files\Netease\MuMu",
];

/// 最后才用的兜底候选设备。
const DEVICE_CANDIDATES: &[&str] = &[
    "127.0.0.1:7555",
    "127.0.0.1:16384",
    "127.0.0.1:5555",
    "emulator-5554",
];

/// 创建子进程时不弹黑窗。
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 进程名里带这些片段的，认为是模拟器虚拟机进程。
const MUMU_VM_PROC_HINTS: &[&str] = &["device", "player", "headless", "nemu"];

/// 各家模拟器常用的 adb 端口，用来排优先级。
const ADB_LIKELY_PORTS: &[&str] = &[
    "7555", "16384", "5555", "21503", "21513", "62001", "62025", "62026", "5554",
];

static LAST_ERROR: Mutex<String> = Mutex::new(String::new());
static LAST_DEVICE_DESC: Mutex<String> = Mutex::new(String::new());

pub fn last_error() -> String {
    LAST_ERROR.lock().map(|g| g.clone()).unwrap_or_default()
}

pub fn last_device_desc() -> String {
    LAST_DEVICE_DESC.lock().map(|g| g.clone()).unwrap_or_default()
}

fn set_last_error(msg: &str) {
    if let Ok(mut g) = LAST_ERROR.lock() {
        *g = msg.to_string();
    }
}

fn set_last_device_desc(msg: &str) {
    if let Ok(mut g) = LAST_DEVICE_DESC.lock() {
        *g = msg.to_string();
    }
}

// ------------------------------------------------------------------ 进程执行

/// 执行命令，返回 (returncode, 文本输出)。
///
/// 打包成窗口版后自身没有控制台，必须加 CREATE_NO_WINDOW，
/// 否则每次调 adb 都会闪出一个黑色命令行窗口。超时返回码 124。
pub fn run(cmd: &[String], timeout: Duration, cwd: Option<&Path>) -> (i32, String) {
    let Some(program) = cmd.first() else {
        return (1, String::new());
    };
    let mut c = Command::new(program);
    c.args(&cmd[1..])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    if let Some(d) = cwd {
        c.current_dir(d);
    }

    let mut child = match c.spawn() {
        Ok(x) => x,
        Err(e) => return (1, format!("{e}")),
    };
    let mut out = child.stdout.take().expect("stdout piped");
    let mut err = child.stderr.take().expect("stderr piped");
    let ho = thread::spawn(move || {
        let mut v = Vec::new();
        let _ = out.read_to_end(&mut v);
        v
    });
    let he = thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err.read_to_end(&mut v);
        v
    });

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let code = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st.code().unwrap_or(-1),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break -1;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break -1,
        }
    };
    if timed_out {
        return (124, "timeout".to_string());
    }

    let mut raw = ho.join().unwrap_or_default();
    raw.extend_from_slice(&he.join().unwrap_or_default());
    (code, String::from_utf8_lossy(&raw).into_owned())
}

fn run_strs(cmd: &[&str], timeout: Duration) -> (i32, String) {
    let v: Vec<String> = cmd.iter().map(|s| s.to_string()).collect();
    run(&v, timeout, None)
}

// ------------------------------------------------------------- 找 adb / 管理器

/// 按 MuMu 自带 adb → 环境变量 adb 的顺序找一个可用的 adb。
pub fn find_adb(explicit: Option<&str>) -> Option<String> {
    if let Some(e) = explicit.filter(|s| !s.is_empty()) {
        // 手动指定了就以它为准，找不到就是找不到，不悄悄换别的
        return if Path::new(e).exists() {
            Some(e.to_string())
        } else {
            None
        };
    }

    let mut cands: Vec<String> = Vec::new();
    for root in MUMU_ROOTS {
        // nx_device/<版本>/shell/adb.exe，版本从高到低
        let dev = Path::new(root).join("nx_device");
        let mut subs: Vec<String> = Vec::new();
        if let Ok(rd) = fs::read_dir(&dev) {
            for e in rd.flatten() {
                let p = e.path().join("shell").join("adb.exe");
                if p.is_file() {
                    subs.push(p.display().to_string());
                }
            }
        }
        subs.sort();
        subs.reverse();
        cands.extend(subs);

        for rel in ["nx_main/adb.exe", "shell/adb.exe"] {
            cands.push(Path::new(root).join(rel).display().to_string());
        }
    }
    for c in &cands {
        if Path::new(c).exists() {
            return Some(c.clone());
        }
    }
    which("adb.exe")
}

/// 在 PATH 里找一个可执行文件。
fn which(name: &str) -> Option<String> {
    let path = std::env::var("PATH").ok()?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand.display().to_string());
        }
    }
    None
}

pub fn find_mumu_manager() -> Option<String> {
    for root in MUMU_ROOTS {
        let p = Path::new(root).join("nx_main").join("MuMuManager.exe");
        if p.is_file() {
            return Some(p.display().to_string());
        }
    }
    None
}

/// 问 MuMuManager 要所有实例的信息（adb 端口、Android 是否已启动）。
///
/// 比硬编码端口靠谱得多：MuMu 各版本的 adb 端口并不固定（实测这台是 16384，
/// 而不是常见的 7555），多开时每个实例还各不相同。
pub fn mumu_instances() -> Vec<Map<String, Value>> {
    let Some(mg) = find_mumu_manager() else {
        return Vec::new();
    };
    let cmd = vec![mg, "info".to_string(), "-v".to_string(), "all".to_string()];
    let (rc, out) = run(&cmd, Duration::from_secs(20), None);
    let Some(start) = out.find('{') else {
        return Vec::new();
    };
    if rc != 0 {
        return Vec::new();
    }
    let Ok(data) = serde_json::from_str::<Value>(&out[start..]) else {
        return Vec::new();
    };
    let Value::Object(map) = data else {
        return Vec::new();
    };
    map.into_values()
        .filter_map(|v| match v {
            Value::Object(m) if m.get("adb_port").is_some() => Some(m),
            _ => None,
        })
        .collect()
}

/// 没有任何配置可读时，直接看 MuMu 进程在监听哪些本地端口。
pub fn mumu_ports_from_processes() -> Vec<String> {
    let (rc, out) = run_strs(
        &["tasklist", "/FO", "CSV", "/NH"],
        Duration::from_secs(20),
    );
    if rc != 0 {
        return Vec::new();
    }

    let mut vm_pids: Vec<String> = Vec::new();
    let mut other_pids: Vec<String> = Vec::new();
    for line in out.lines() {
        let parts: Vec<String> = line
            .split("\",\"")
            .map(|p| p.trim().trim_matches('"').to_string())
            .collect();
        if parts.len() < 2 {
            continue;
        }
        let name = parts[0].clone();
        let pid = parts[1].clone();
        let low = name.to_lowercase();
        if !(low.starts_with("mumu") || low.starts_with("nemu")) {
            continue;
        }
        if MUMU_VM_PROC_HINTS.iter().any(|h| low.contains(h)) {
            vm_pids.push(pid);
        } else {
            other_pids.push(pid);
        }
    }

    let (rc, out) = run_strs(&["netstat", "-ano"], Duration::from_secs(20));
    if rc != 0 {
        return Vec::new();
    }

    let collect = |pids: &[String]| -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        if pids.is_empty() {
            return found;
        }
        for line in out.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 5
                || !f[0].eq_ignore_ascii_case("TCP")
                || !f[3].eq_ignore_ascii_case("LISTENING")
            {
                continue;
            }
            if !pids.iter().any(|p| p == f[4]) {
                continue;
            }
            let Some((addr, port)) = f[1].rsplit_once(':') else {
                continue;
            };
            if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) || port == "0" {
                continue;
            }
            if !matches!(addr, "127.0.0.1" | "0.0.0.0" | "[::]" | "::") {
                continue;
            }
            if !found.iter().any(|p| p == port) {
                found.push(port.to_string());
            }
        }
        found
    };

    // 虚拟机进程持有的端口最可能是 adb，优先；找不到再看 MuMu 的其他进程
    let vm_ports = collect(&vm_pids);
    let mut ports = vm_ports.clone();
    for p in collect(&other_pids) {
        if !ports.contains(&p) {
            ports.push(p);
        }
    }
    // 同类里先试"长得像 adb"的端口，免得先去连渲染/控制端口白等握手超时
    ports.sort_by_key(|p| !ADB_LIKELY_PORTS.contains(&p.as_str()));
    ports.truncate(8);
    ports
}

// -------------------------------------------------------------------- 选设备

fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(m)) => !m.is_empty(),
        _ => false,
    }
}

fn as_str(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

/// 试着连一个序列号；连得上返回 true。
fn try_serial(adb: &str, serial: &str, boot_retry: u32) -> bool {
    if serial.contains(':') {
        // 连真实 adb 端口是毫秒级；连到非 adb 端口会等握手超时，所以别给太长
        let _ = run_strs(&[adb, "connect", serial], Duration::from_secs(6));
    }
    for i in 0..=boot_retry {
        let (rc, out) = run_strs(
            &[adb, "-s", serial, "shell", "getprop", "ro.build.version.release"],
            Duration::from_secs(12),
        );
        let low = out.to_lowercase();
        if rc == 0 && !out.trim().is_empty() && !low.contains("error") && !low.contains("offline") {
            return true;
        }
        if i < boot_retry {
            thread::sleep(Duration::from_millis(1500)); // 模拟器正在开机，给它一点时间
        }
    }
    false
}

/// 自动找到正在运行的 MuMu 实例并连上它。
///
/// 识别顺序（越靠前越准确）：
///   1. 显式指定的设备
///   2. MuMuManager 报出的实例 adb 端口（最准，还能区分多开实例）
///   3. 扫 MuMu 进程实际监听的本地端口（不依赖任何配置，也不猜端口号）
///   4. 常见端口 / adb devices 兜底
pub fn pick_device(adb: &str, wanted: Option<&str>) -> Option<String> {
    set_last_error("");

    if let Some(w) = wanted.filter(|s| !s.is_empty()) {
        if try_serial(adb, w, 3) {
            set_last_device_desc(&format!("{w}（手动指定）"));
            return Some(w.to_string());
        }
        set_last_error(&format!("连不上指定的设备 {w}"));
        return None;
    }

    let instances = mumu_instances();

    // 1) MuMuManager 报的端口：已启动的实例优先，并容忍它还在开机
    let mut ordered = instances.clone();
    ordered.sort_by(|a, b| {
        let ka = (
            !truthy(a.get("is_android_started")),
            as_str(a.get("index")).unwrap_or_else(|| "0".to_string()),
        );
        let kb = (
            !truthy(b.get("is_android_started")),
            as_str(b.get("index")).unwrap_or_else(|| "0".to_string()),
        );
        ka.cmp(&kb)
    });
    for item in &ordered {
        let host = as_str(item.get("adb_host_ip")).unwrap_or_else(|| "127.0.0.1".to_string());
        let Some(port) = as_str(item.get("adb_port")) else {
            continue;
        };
        let serial = format!("{host}:{port}");
        let retry = if truthy(item.get("is_android_started")) {
            3
        } else {
            0
        };
        if try_serial(adb, &serial, retry) {
            let idx = as_str(item.get("index")).unwrap_or_default();
            let name = as_str(item.get("name")).unwrap_or_else(|| "MuMu".to_string());
            set_last_device_desc(&format!("{serial}（实例 {idx}「{name}」）"));
            return Some(serial);
        }
    }

    // 2) 扫 MuMu 进程实际监听的端口
    for port in mumu_ports_from_processes() {
        let serial = format!("127.0.0.1:{port}");
        if try_serial(adb, &serial, 0) {
            set_last_device_desc(&format!("{serial}（从 MuMu 进程的监听端口里找到）"));
            return Some(serial);
        }
    }

    // 3) MuMuManager 明确知道实例状态时，不用再猜常见端口了（省好几秒）
    if !instances.is_empty() {
        set_last_error("MuMu 模拟器没在运行（或正在启动/关闭中），请先打开模拟器再试");
        return None;
    }

    // 4) 最后才试常见端口 / adb devices
    for serial in DEVICE_CANDIDATES {
        if try_serial(adb, serial, 0) {
            set_last_device_desc(serial);
            return Some(serial.to_string());
        }
    }

    let (_, out) = run_strs(&[adb, "devices"], Duration::from_secs(15));
    for line in out.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1] == "device" {
            set_last_device_desc(&format!("{}（来自 adb devices）", parts[0]));
            return Some(parts[0].to_string());
        }
    }

    set_last_error("没检测到运行中的 MuMu 实例，请先启动模拟器");
    None
}

// ---------------------------------------------------------------- 相册投送

pub fn adb_shell(adb: &str, serial: &str, args: &[&str], timeout: Duration) -> (i32, String) {
    let mut cmd = vec![
        adb.to_string(),
        "-s".to_string(),
        serial.to_string(),
        "shell".to_string(),
    ];
    cmd.extend(args.iter().map(|s| s.to_string()));
    run(&cmd, timeout, None)
}

/// 触发 MediaStore 重新扫描存储卷。
pub fn scan_media(adb: &str, serial: &str) {
    adb_shell(
        adb,
        serial,
        &[
            "content",
            "call",
            "--uri",
            "content://media/external/file",
            "--method",
            "scan_volume",
            "--arg",
            "external_primary",
        ],
        Duration::from_secs(40),
    );
}

/// 在媒体库里查这张图的 _id，查到说明图库已经能看到它。
///
/// 注意：Windows 下 adb 转发命令会把单引号吃掉，所以不能带 --where 过滤，
/// 改成把图片列表拉回来在本地按文件名匹配。
pub fn query_media_id(adb: &str, serial: &str, remote_path: &str) -> Option<String> {
    let name = remote_path.rsplit(['/', '\\']).next().unwrap_or(remote_path);
    let (_, out) = adb_shell(
        adb,
        serial,
        &[
            "content",
            "query",
            "--uri",
            "content://media/external/images/media",
            "--projection",
            "_id:_data",
        ],
        Duration::from_secs(30),
    );
    for line in out.lines() {
        if let Some((id, data)) = split_id_data(line) {
            if data.ends_with(name) {
                return Some(id);
            }
        }
    }
    None
}

/// 从 `Row: 0 _id=123, _data=/sdcard/xxx/a.png` 抽出 (_id, _data)。
fn split_id_data(line: &str) -> Option<(String, String)> {
    let idx = line.find("_id=")?;
    let rest = &line[idx + 4..];
    let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if id.is_empty() {
        return None;
    }
    let didx = line.find("_data=")?;
    let drest = line[didx + 6..].trim();
    let data: String = drest
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ',')
        .collect();
    if data.is_empty() {
        return None;
    }
    Some((id, data))
}

/// 把照片推到模拟器相册目录，并让图库立即可见。返回是否推送成功。
pub fn push_photo(adb: &str, serial: &str, local: &str, remote_dir: &str, open_gallery: bool) -> bool {
    let local_path = Path::new(local);
    let name = local_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("shot.png");
    let remote = format!("{}/{}", remote_dir.trim_end_matches('/'), name);

    adb_shell(adb, serial, &["mkdir", "-p", remote_dir], Duration::from_secs(20));

    // 在截图所在目录内以相对文件名调用 adb，并把完整目标路径（全 ASCII）显式传给 adb：
    // 若让 adb 自己从本地路径推导远端文件名，遇到非 ASCII 路径会把名字截短。
    let cmd = vec![
        adb.to_string(),
        "-s".to_string(),
        serial.to_string(),
        "push".to_string(),
        name.to_string(),
        remote.clone(),
    ];
    let (rc, out) = run(&cmd, Duration::from_secs(120), local_path.parent());
    if rc != 0 {
        let brief: String = out.trim().chars().take(200).collect();
        set_last_error(&format!("推送到模拟器失败：{brief}"));
        core::log(&format!("  [x] 推送失败：{}", out.trim()));
        return false;
    }

    // 让所有进程都能读到
    adb_shell(adb, serial, &["chmod", "666", &remote], Duration::from_secs(20));
    scan_media(adb, serial);

    let mut media_id = None;
    for _ in 0..6 {
        media_id = query_media_id(adb, serial, &remote);
        if media_id.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(800));
    }

    match &media_id {
        Some(id) => {
            set_last_error("");
            core::log(&format!("  [√] 已进入模拟器相册：{remote}  (media id={id})"));
        }
        None => {
            core::log(&format!(
                "  [!] 文件已推送，但媒体库暂未索引：{remote}（在图库里下拉刷新一下即可）"
            ));
        }
    }

    if open_gallery {
        if let Some(id) = &media_id {
            let uri = format!("content://media/external/images/media/{id}");
            adb_shell(
                adb,
                serial,
                &[
                    "am",
                    "start",
                    "-a",
                    "android.intent.action.VIEW",
                    "-d",
                    &uri,
                    "-t",
                    "image/*",
                ],
                Duration::from_secs(20),
            );
        } else {
            adb_shell(
                adb,
                serial,
                &[
                    "monkey",
                    "-p",
                    "com.android.gallery3d",
                    "-c",
                    "android.intent.category.LAUNCHER",
                    "1",
                ],
                Duration::from_secs(20),
            );
        }
    }
    true
}
