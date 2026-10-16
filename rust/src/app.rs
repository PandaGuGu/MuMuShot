//! 统一入口：截图 → 上传。对应 Python 版 `Uploader` + `capture_by_action`。

use std::path::PathBuf;
use std::time::Duration;

use crate::capture::RgbaImage;
use crate::{adb, capture, core};

/// 负责把本地图片送进 MuMu 相册，连接信息只解析一次。
pub struct Engine {
    pub adb_arg: Option<String>,
    pub device: Option<String>,
    pub remote_dir: String,
    adb: Option<String>,
    serial: Option<String>,
    count: u32,
    pub last_error: String,
    pub out_dir: PathBuf,
}

impl Engine {
    pub fn new(adb_arg: Option<String>, device: Option<String>, remote_dir: String) -> Self {
        Engine {
            adb_arg,
            device,
            remote_dir,
            adb: None,
            serial: None,
            count: 0,
            last_error: String::new(),
            out_dir: core::ensure_dir(&core::shots_dir()),
        }
    }

    /// 找 adb 并连上模拟器。已经连上就直接返回。
    pub fn connect(&mut self) -> bool {
        if self.serial.is_some() {
            return true;
        }
        let Some(adb_path) = adb::find_adb(self.adb_arg.as_deref()) else {
            self.last_error = "没找到 adb，请确认已安装 MuMu 模拟器".to_string();
            core::log(&format!("  [x] {}", self.last_error));
            return false;
        };
        let Some(serial) = adb::pick_device(&adb_path, self.device.as_deref()) else {
            let e = adb::last_error();
            self.last_error = if e.is_empty() {
                "没连上 MuMu，请确认模拟器已经启动".to_string()
            } else {
                e
            };
            core::log(&format!("  [x] {}", self.last_error));
            return false;
        };
        self.adb = Some(adb_path);
        self.serial = Some(serial.clone());
        let desc = adb::last_device_desc();
        core::log(&format!(
            "  [√] 已连上 MuMu：{}",
            if desc.is_empty() { serial } else { desc }
        ));
        true
    }

    /// 把图像存一份到本机 shots/，再推进模拟器相册。
    pub fn send(&mut self, image: Option<RgbaImage>, tag: &str, open_gallery: bool) -> bool {
        let Some(img) = image else {
            self.last_error = "没拿到图像".to_string();
            core::log(&format!("  [x] {}", self.last_error));
            return false;
        };
        if !self.connect() {
            return false;
        }
        let (Some(adb_path), Some(serial)) = (self.adb.clone(), self.serial.clone()) else {
            return false;
        };

        self.count += 1;
        let name = format!(
            "{}_{}.png",
            tag,
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );
        let local = self.out_dir.join(&name);
        if let Err(e) = capture::save_png(&img, &local) {
            self.last_error = format!("保存截图失败：{e}");
            core::log(&format!("  [x] {}", self.last_error));
            return false;
        }
        core::log(&format!(
            "[第 {} 张] {}  ({}×{})",
            self.count,
            local.display(),
            img.width(),
            img.height()
        ));

        let ok = adb::push_photo(
            &adb_path,
            &serial,
            &local.display().to_string(),
            &self.remote_dir,
            open_gallery,
        );
        self.last_error = if ok {
            String::new()
        } else {
            let e = adb::last_error();
            if e.is_empty() {
                "推送失败，详见日志文件".to_string()
            } else {
                e
            }
        };
        ok
    }
}

/// 按动作名产出图像，返回 (图像, 标签)。拿不到图时图像为 None。
pub fn capture_by_action(
    action: &str,
    region: Option<(i32, i32, i32, i32)>,
    snip_timeout: Duration,
) -> (Option<RgbaImage>, &'static str) {
    match action {
        "region" => {
            let img = match region {
                Some((x0, y0, x1, y1)) => capture::grab_bbox(x0, y0, x1, y1).ok(),
                None => crate::gui::region_shot_standalone(),
            };
            (img, "Region")
        }
        "snip" => (capture::snip_via_system(snip_timeout), "Snip"),
        "clip" => (capture::grab_clipboard(), "Clip"),
        _ => (capture::grab_full().ok(), "Shot"),
    }
}

/// 截取标题包含关键词的窗口，返回 (图像, 标签)。
pub fn capture_window(keyword: &str) -> (Option<RgbaImage>, &'static str) {
    (capture::grab_window(keyword).ok(), "Win")
}
