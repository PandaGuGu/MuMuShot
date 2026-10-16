//! 图形面板 + 框选覆盖层 + 窗口选择。对应 Python 版 `run_gui` / `select_region`。

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use crate::Args;
use crate::app::Engine;
use crate::capture::{self, RgbaImage, WinInfo};
use crate::core;
use crate::hotkeys::{self, Hotkeys};

/// 选区框颜色，与 Python 版一致。
const C_REGION: Color32 = Color32::from_rgb(0x31, 0xe0, 0x7a);
/// 面板状态文字颜色，与 Python 版一致。
const C_STATUS: Color32 = Color32::from_rgb(0x7b, 0xd8, 0x8f);
/// 底部提示颜色。
const C_HINT: Color32 = Color32::from_rgb(0x8a, 0x8a, 0x8a);

/// 面板标题（窗口选择时要按它排除自己）。
const PANEL_TITLE: &str = "MuMuShot 截图助手";

/// 一次截图的产出。
struct Shot {
    image: Option<RgbaImage>,
    tag: &'static str,
    note: String,
}

/// 交给后台线程的上传任务。
struct Job {
    image: RgbaImage,
    tag: &'static str,
    note: String,
    open_gallery: bool,
}

/// 后台回报的上传结果。
struct JobDone {
    ok: bool,
    err: String,
    note: String,
    w: u32,
    h: u32,
}

fn spawn_worker(
    adb: Option<String>,
    device: Option<String>,
    remote_dir: String,
) -> (Sender<Job>, Receiver<JobDone>) {
    let (job_tx, job_rx) = channel::<Job>();
    let (done_tx, done_rx) = channel::<JobDone>();
    std::thread::spawn(move || {
        let mut engine = Engine::new(adb, device, remote_dir);
        while let Ok(job) = job_rx.recv() {
            let (w, h) = (job.image.width(), job.image.height());
            let ok = engine.send(Some(job.image), job.tag, job.open_gallery);
            let _ = done_tx.send(JobDone {
                ok,
                err: engine.last_error.clone(),
                note: job.note,
                w,
                h,
            });
        }
    });
    (job_tx, done_rx)
}

/// 框选覆盖层的内部状态。
struct Overlay {
    img: RgbaImage,
    tex: Option<egui::TextureHandle>,
    start: Option<Pos2>,
    cur: Option<Pos2>,
}

impl Overlay {
    fn new(img: RgbaImage) -> Self {
        Overlay {
            img,
            tex: None,
            start: None,
            cur: None,
        }
    }
}

/// 覆盖层每帧的产出。
enum OverlayOutcome {
    Pending,
    /// 已按框选范围裁好
    Picked(RgbaImage),
    Cancelled,
}

/// 显示框选覆盖层一帧。调用方负责在 Picked/Cancelled 后收尾。
fn overlay_frame(ctx: &egui::Context, ov: &mut Overlay) -> OverlayOutcome {
    if ov.tex.is_none() {
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [ov.img.width() as usize, ov.img.height() as usize],
            ov.img.as_raw(),
        );
        ov.tex = Some(ctx.load_texture("overlay", color, egui::TextureOptions::NEAREST));
    }

    let (l, t, vw, vh) = capture::virtual_screen_rect();
    let ppp = ctx.pixels_per_point().max(0.5);
    let builder = egui::ViewportBuilder::default()
        .with_title("MuMuShot 框选")
        .with_decorations(false)
        .with_resizable(false)
        .with_always_on_top()
        .with_taskbar(false)
        .with_position(Pos2::new(l as f32 / ppp, t as f32 / ppp))
        .with_inner_size(Vec2::new(vw as f32 / ppp, vh as f32 / ppp));

    let vid = egui::ViewportId::from_hash_of("mumushot_overlay");

    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    let mut cancel = false;

    ctx.show_viewport_immediate(vid, builder, |ui, _class| {
        let rect = ui.max_rect();
        let p = ui.painter();

        if let Some(tex) = &ov.tex {
            p.image(
                tex.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        // 整屏压暗（Python 用 Brightness.enhance(0.42) 把整张底图调暗）
        p.rect_filled(rect, CornerRadius::ZERO, Color32::from_black_alpha(148));

        let id = ui.scope_id().with("overlay_surface");
        let resp = ui.interact(rect, id, Sense::click_and_drag());

        // 起点用真正的按下位置（press_origin），别用拖拽阈值触发时的位置，
        // 否则选区会比实际拖的范围小一截。
        if resp.drag_started() || resp.dragged() {
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                ov.start = Some(origin);
            } else if ov.start.is_none() {
                ov.start = resp.interact_pointer_pos();
            }
            if let Some(p) = resp.interact_pointer_pos() {
                ov.cur = Some(p);
            }
        }

        if let (Some(a), Some(b)) = (ov.start, ov.cur) {
            let s = Rect::from_two_pos(a, b);
            p.rect_stroke(
                s,
                CornerRadius::ZERO,
                Stroke::new(2.0, C_REGION),
                StrokeKind::Inside,
            );
            p.text(
                Pos2::new(s.min.x + 8.0, s.min.y + 8.0),
                Align2::LEFT_TOP,
                format!("{} × {}", s.width().round(), s.height().round()),
                FontId::proportional(11.0),
                C_REGION,
            );
        }

        if resp.secondary_clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if resp.drag_stopped() {
            if let (Some(a), Some(b)) = (ov.start, ov.cur) {
                let s = Rect::from_two_pos(a, b);
                let iw = ov.img.width() as f32;
                let ih = ov.img.height() as f32;
                let sx = ((s.min.x - rect.min.x) / rect.width() * iw).round() as i32;
                let sy = ((s.min.y - rect.min.y) / rect.height() * ih).round() as i32;
                let sw = (s.width() / rect.width() * iw).round() as i32;
                let sh = (s.height() / rect.height() * ih).round() as i32;
                let sx = sx.clamp(0, ov.img.width() as i32 - 1);
                let sy = sy.clamp(0, ov.img.height() as i32 - 1);
                let sw = sw.clamp(1, ov.img.width() as i32 - sx);
                let sh = sh.clamp(1, ov.img.height() as i32 - sy);
                // Python 版：宽高都 >= 4 才算有效框选
                if sw >= 4 && sh >= 4 {
                    bbox = Some((sx as u32, sy as u32, sw as u32, sh as u32));
                }
            }
        }

        if bbox.is_some() || cancel {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    });

    if let Some((x, y, w, h)) = bbox {
        OverlayOutcome::Picked(image::imageops::crop_imm(&ov.img, x, y, w, h).to_image())
    } else if cancel {
        OverlayOutcome::Cancelled
    } else {
        OverlayOutcome::Pending
    }
}

/// 一次待办动作。
#[derive(Clone)]
enum Action {
    Region(Option<(i32, i32, i32, i32)>),
    Snip,
    Full,
    Clip,
    Window(Option<(String, (i32, i32, i32, i32))>),
}

impl Action {
    /// 面板按钮 / 热键触发的单次动作。
    fn single(name: &str) -> Action {
        match name {
            "region" => Action::Region(None),
            "snip" => Action::Snip,
            "clip" => Action::Clip,
            "window_pick" => Action::Window(None),
            _ => Action::Full,
        }
    }
}

pub struct PanelApp {
    args: Args,
    status: String,
    pending: Option<Action>,
    busy: bool,

    job_tx: Sender<Job>,
    done_rx: Receiver<JobDone>,

    /// 后台抓图（先藏面板再抓）
    shot_rx: Option<Receiver<Shot>>,
    /// 抓完等着在 ui 遍里处理的图（region 还要开覆盖层）
    shot_queue: Option<Shot>,
    overlay: Option<Overlay>,

    show_picker: bool,
    windows: Vec<WinInfo>,
    picker_sel: Option<usize>,

    _hotkeys: Option<Hotkeys>,
    hotkey_failed: Vec<String>,
}

impl PanelApp {
    fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        install_cjk_font(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        hotkeys::drain();
        let hk = Hotkeys::start();
        let hotkey_failed = hk.failed.clone();

        let (job_tx, done_rx) =
            spawn_worker(args.adb.clone(), args.device.clone(), args.remote_dir.clone());

        let mut app = PanelApp {
            status: "就绪".to_string(),
            pending: None,
            busy: false,
            job_tx,
            done_rx,
            shot_rx: None,
            shot_queue: None,
            overlay: None,
            show_picker: false,
            windows: Vec::new(),
            picker_sel: None,
            _hotkeys: Some(hk),
            hotkey_failed,
            args,
        };

        if app.hotkey_failed.is_empty() {
            app.status = "就绪 · 热键 Ctrl+Alt+A/Q/S/C".to_string();
        } else {
            let names: Vec<&str> = app
                .hotkey_failed
                .iter()
                .map(|f| hotkeys::short_name(f))
                .collect();
            core::log(&format!(
                "  [!] 这些热键被别的软件占用了：{}",
                names.join("、")
            ));
            app.status = format!(
                "这些热键被别的软件占用了：{}（用面板按钮即可）",
                names.join("、")
            );
        }
        core::log("面板已打开：Ctrl+Alt+A 框选 / Q 系统截图 / S 全屏 / C 剪贴板，关掉窗口即退出。");
        app
    }

    fn restore_panel(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    }

    /// 统一入口：截图 → 上传。截图放后台线程，别卡住面板。
    fn start(&mut self, ctx: &egui::Context, action: Action) {
        if self.busy {
            self.status = "上一张还在处理，稍等…".to_string();
            return;
        }
        self.busy = true;

        let hide = self.args.hide_panel_on_shot;
        if hide {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        let snip_timeout = Duration::from_secs_f64(self.args.snip_timeout);
        let (tx, rx) = channel::<Shot>();
        std::thread::spawn(move || {
            if hide {
                std::thread::sleep(Duration::from_millis(200));
            }
            let shot = match action {
                Action::Region(Some((x0, y0, x1, y1))) => Shot {
                    image: capture::grab_bbox(x0, y0, x1, y1).ok(),
                    tag: "Region",
                    note: String::new(),
                },
                Action::Region(None) => Shot {
                    image: capture::grab_full().ok(),
                    tag: "Region",
                    note: String::new(),
                },
                Action::Full => Shot {
                    image: capture::grab_full().ok(),
                    tag: "Shot",
                    note: String::new(),
                },
                Action::Snip => Shot {
                    image: capture::snip_via_system(snip_timeout),
                    tag: "Snip",
                    note: String::new(),
                },
                Action::Clip => Shot {
                    image: capture::grab_clipboard(),
                    tag: "Clip",
                    note: String::new(),
                },
                Action::Window(Some((title, (x, y, w, h)))) => Shot {
                    image: capture::grab_bbox(x, y, x + w, y + h).ok(),
                    tag: "Win",
                    note: format!("「{title}」"),
                },
                Action::Window(None) => Shot {
                    image: None,
                    tag: "Win",
                    note: String::new(),
                },
            };
            let _ = tx.send(shot);
        });
        self.shot_rx = Some(rx);
        self.status = "截图中…".to_string();
    }

    /// 抓完图后：region 打开覆盖层，其余直接上传。
    fn consume_shot(&mut self, ctx: &egui::Context, shot: Shot) {
        if shot.tag == "Region" {
            if self.args.hide_panel_on_shot {
                self.restore_panel(ctx);
            }
            match shot.image {
                Some(img) => {
                    self.overlay = Some(Overlay::new(img));
                    self.status = "拖拽框选要上传的区域（Esc / 右键取消）".to_string();
                }
                None => {
                    self.status = "已取消框选".to_string();
                    self.busy = false;
                }
            }
            return;
        }

        if self.args.hide_panel_on_shot {
            self.restore_panel(ctx);
        }
        if shot.image.is_none() {
            self.status = "已取消".to_string();
            self.busy = false;
            return;
        }
        self.upload(shot);
    }

    fn upload(&mut self, shot: Shot) {
        let Some(img) = shot.image else {
            self.busy = false;
            return;
        };
        self.status = format!(
            "已截图{} {}×{}，上传中…",
            shot.note,
            img.width(),
            img.height()
        );
        let _ = self.job_tx.send(Job {
            image: img,
            tag: shot.tag,
            note: shot.note,
            open_gallery: self.args.open_gallery,
        });
    }

    fn poll_shot(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.shot_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(shot) => {
                self.shot_rx = None;
                self.shot_queue = Some(shot);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(30));
            }
            Err(_) => {
                self.shot_rx = None;
                self.status = "截图线程异常退出".to_string();
                if self.args.hide_panel_on_shot {
                    self.restore_panel(ctx);
                }
                self.busy = false;
            }
        }
    }

    fn step_overlay(&mut self, ctx: &egui::Context) {
        if self.overlay.is_none() {
            return;
        }
        let mut ov = self.overlay.take().expect("overlay checked");
        match overlay_frame(ctx, &mut ov) {
            OverlayOutcome::Pending => {
                ctx.request_repaint();
                self.overlay = Some(ov);
            }
            OverlayOutcome::Picked(img) => {
                self.upload(Shot {
                    image: Some(img),
                    tag: "Region",
                    note: String::new(),
                });
            }
            OverlayOutcome::Cancelled => {
                self.status = "已取消框选".to_string();
                self.busy = false;
            }
        }
    }

    fn draw_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let shots = core::shots_dir();
        egui::Frame::NONE
            .fill(Color32::from_rgb(0x1e, 0x1e, 0x22))
            .inner_margin(14.0)
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("截电脑屏幕 → 送进 MuMu 相册")
                        .size(15.0)
                        .strong(),
                );
                ui.add_space(10.0);

                let bw = ui.available_width();
                let mut click: Option<&str> = None;
                for (label, name) in [
                    ("框选截图上传              Ctrl + Alt + A", "region"),
                    ("调系统截图工具框选        Ctrl + Alt + Q", "snip"),
                    ("全屏截图上传              Ctrl + Alt + S", "full"),
                    ("上传剪贴板里的图片        Ctrl + Alt + C", "clip"),
                    ("截取指定窗口", "window_pick"),
                ] {
                    if ui.add_sized([bw, 30.0], egui::Button::new(label)).clicked() {
                        click = Some(name);
                    }
                }
                if let Some(name) = click {
                    self.pending = Some(Action::single(name));
                }

                ui.add_space(10.0);
                ui.checkbox(&mut self.args.open_gallery, "上传后自动在模拟器里打开图片");
                ui.checkbox(
                    &mut self.args.hide_panel_on_shot,
                    "截图瞬间隐藏本面板（避免拍进去）",
                );

                ui.add_space(10.0);
                ui.label(egui::RichText::new(&self.status).size(13.0).color(C_STATUS));
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!("照片同时留一份在本机：{}", shots.display()))
                        .size(11.0)
                        .color(C_HINT),
                );
            });

        if self.show_picker {
            self.draw_picker(ctx);
        }
    }

    fn draw_picker(&mut self, ctx: &egui::Context) {
        let mut chosen: Option<(String, (i32, i32, i32, i32))> = None;
        egui::Window::new("选择窗口")
            .resizable(false)
            .collapsible(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("双击要截取的窗口：");
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for (i, w) in self.windows.iter().enumerate() {
                            let text = format!("{}    ({}×{})", w.title, w.rect.2, w.rect.3);
                            let r = ui.selectable_label(self.picker_sel == Some(i), text);
                            if r.clicked() {
                                self.picker_sel = Some(i);
                            }
                            if r.double_clicked() {
                                chosen = Some((w.title.clone(), w.rect));
                            }
                        }
                    });
                ui.add_space(6.0);
                if ui.button("截取这个窗口").clicked() {
                    if let Some(i) = self.picker_sel {
                        if let Some(w) = self.windows.get(i) {
                            chosen = Some((w.title.clone(), w.rect));
                        }
                    }
                }
            });

        if let Some((title, rect)) = chosen {
            self.show_picker = false;
            self.start(ctx, Action::Window(Some((title, rect))));
        }
    }
}

impl eframe::App for PanelApp {
    /// 每遍 ui 之前都会跑；窗口藏起来时也会跑，所以轮询都放这里。
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(a) = hotkeys::poll_action() {
            if self.pending.is_none() {
                self.pending = Some(Action::single(a));
            }
        }
        while let Ok(d) = self.done_rx.try_recv() {
            if d.ok {
                self.status = format!("已上传到模拟器相册{}（{}×{}）", d.note, d.w, d.h);
            } else {
                self.status = format!("上传失败：{}", d.err);
            }
            self.busy = false;
        }
        self.poll_shot(ctx);

        if self.busy || self.overlay.is_some() || self.shot_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if let Some(shot) = self.shot_queue.take() {
            self.consume_shot(&ctx, shot);
        }
        if let Some(action) = self.pending.take() {
            if matches!(action, Action::Window(None)) {
                self.windows = capture::list_windows()
                    .into_iter()
                    .filter(|w| w.title != PANEL_TITLE)
                    .collect();
                if self.windows.is_empty() {
                    self.status = "没找到可截取的窗口".to_string();
                } else {
                    self.show_picker = true;
                    self.picker_sel = None;
                }
            } else {
                self.start(&ctx, action);
            }
        }
        self.step_overlay(&ctx);
        self.draw_panel(ui, &ctx);
    }
}

/// 装中文字体（egui 自带字体没有汉字）。
fn install_cjk_font(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ];
    for c in candidates {
        if let Ok(bytes) = std::fs::read(c) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("cjk".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "cjk".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("cjk".to_owned());
            ctx.set_fonts(fonts);
            return;
        }
    }
    core::log("  [!] 没找到中文字体，界面可能显示为方块");
}

/// 启动图形面板。返回进程退出码。
pub fn run_gui(args: Args) -> i32 {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([410.0, 400.0])
            .with_min_inner_size([390.0, 380.0])
            .with_always_on_top()
            .with_title(PANEL_TITLE),
        ..Default::default()
    };
    let fallback = args.clone();
    let r = eframe::run_native(
        "MuMuShot",
        options,
        Box::new(move |cc| Ok(Box::new(PanelApp::new(cc, args)))),
    );
    if let Err(e) = r {
        core::log(&format!("  [!] 图形界面启动失败：{e}，改用控制台模式"));
        return crate::run_console(&fallback);
    }
    0
}

/// CLI 里 `--region` 不带坐标时：单独跑一次框选覆盖层，返回选中的图像。
pub fn region_shot_standalone() -> Option<RgbaImage> {
    let img = match capture::grab_full() {
        Ok(i) => i,
        Err(e) => {
            core::log(&format!("  [x] {e}"));
            return None;
        }
    };
    let out: Arc<Mutex<Option<RgbaImage>>> = Arc::new(Mutex::new(None));
    let out2 = out.clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_visible(false),
        ..Default::default()
    };
    let run = eframe::run_native(
        "MuMuShot 框选",
        options,
        Box::new(move |cc| Ok(Box::new(OverlayOnlyApp::new(cc, img, out2)))),
    );
    if let Err(e) = run {
        core::log(&format!("  [x] 框选界面启动失败：{e}"));
        return None;
    }
    out.lock().ok().and_then(|g| g.clone())
}

/// 只做框选的最小 App（CLI 用）。
struct OverlayOnlyApp {
    overlay: Option<Overlay>,
    out: Arc<Mutex<Option<RgbaImage>>>,
}

impl OverlayOnlyApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        img: RgbaImage,
        out: Arc<Mutex<Option<RgbaImage>>>,
    ) -> Self {
        install_cjk_font(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::Visible(false));
        OverlayOnlyApp {
            overlay: Some(Overlay::new(img)),
            out,
        }
    }
}

impl eframe::App for OverlayOnlyApp {
    /// 根窗口是隐藏的：eframe 在窗口隐藏时不会跑 `ui()`，
    /// 必须显式要求「隐藏时也渲染」，否则框选视口根本不会创建。
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        ctx.request_repaint();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let Some(mut ov) = self.overlay.take() else {
            return;
        };
        match overlay_frame(&ctx, &mut ov) {
            OverlayOutcome::Pending => {
                self.overlay = Some(ov);
                ctx.request_repaint();
            }
            OverlayOutcome::Picked(img) => {
                if let Ok(mut g) = self.out.lock() {
                    *g = Some(img);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            OverlayOutcome::Cancelled => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}
