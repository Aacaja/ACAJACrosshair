//! ACAJA 设置窗口（egui 现代化界面）。
//!
//! - 工作副本模式：界面上所有改动先落到 `self.preset`，帧末经 IPC 推给后台壳进程；
//! - 「保存预设」才持久化到磁盘；语言/主题即时持久化到 app.json；
//! - 退出按钮直接退出进程（托盘常驻后的优雅退出协议后续版再做）。

pub mod fonts;
pub mod preview;
pub mod strings;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use egui::{Align, Color32, ComboBox, Context, RichText, TextEdit, Ui, Visuals};
use log::{info, warn};

use crate::config::{
    AdsButton, AdsMode, Hotkey, PosVal, Preset, PresetStore, RightClickMode, Shape,
};
use crate::i18n::Lang;
use crate::ui::strings::{ads_mode_name, shape_name, t};

const ACCENT: Color32 = Color32::from_rgb(10, 132, 255);
const STATUS_TTL: std::time::Duration = std::time::Duration::from_millis(1600);

pub struct AcajaApp {
    store: Arc<Mutex<PresetStore>>,
    /// 最近一次向后端推送时间（IPC 节流）
    last_send: std::time::Instant,
    template_name: &'static str,

    /// 工作副本（界面直接编辑此预设）
    preset: Preset,
    active_name: String,
    dirty: bool,
    visible: bool,

    lang: Lang,
    theme: String,

    new_preset_name: String,
    status: Option<(String, Instant)>,

    // hex 编辑缓冲（与预设颜色字段分离，避免输入中间态污染）
    hex_main: String,
    hex_top: String,
    hex_bottom: String,
    hex_left: String,
    hex_right: String,
    hex_outline: String,
    hotkey_buf: String,
}

/// 启动设置窗口（独立进程模式：阻塞直到窗口关闭，关闭即进程结束）
pub fn run(
    store: Arc<Mutex<PresetStore>>,
    title: &'static str,
) -> eframe::Result<()> {
    // 窗口图标：内置品牌 A 图标（64px PNG → egui IconData）
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([760.0, 920.0])
        .with_min_inner_size([620.0, 640.0])
        .with_title(title);
    if let Ok(im) = image::load_from_memory(include_bytes!("../../assets/icons/ACAJA_64.png")) {
        let rgba = im.to_rgba8();
        let (w, h) = rgba.dimensions();
        viewport = viewport.with_icon(Arc::new(egui::IconData {
            rgba: rgba.into_raw(),
            width: w,
            height: h,
        }));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        title,
        options,
        Box::new(move |cc| Ok(Box::new(AcajaApp::new(cc, store)) as Box<dyn eframe::App>)),
    )
}

impl AcajaApp {
    fn new(cc: &eframe::CreationContext<'_>, store: Arc<Mutex<PresetStore>>) -> Self {
        // ---- 中文字体 ----
        if let Some(font_bytes) = fonts::load_cjk_font() {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("msyh".to_owned(), Arc::new(egui::FontData::from_owned(font_bytes)));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                if let Some(list) = fonts.families.get_mut(&family) {
                    list.insert(0, "msyh".to_owned());
                }
            }
            cc.egui_ctx.set_fonts(fonts);
        } else {
            info!("未找到中文字体，界面将回退系统字体");
        }

        let store_guard = store.lock().unwrap();
        let preset = store_guard.get_active().clone();
        let active_name = store_guard.active_name();
        let lang = store_guard.app.lang();
        let theme = store_guard.app.theme.clone();
        drop(store_guard);

        let mut app = AcajaApp {
            store,
            last_send: std::time::Instant::now(),
            template_name: "tpl_apex",
            preset,
            active_name,
            dirty: true,
            visible: true,
            lang,
            theme,
            new_preset_name: String::new(),
            status: None,
            hex_main: String::new(),
            hex_top: String::new(),
            hex_bottom: String::new(),
            hex_left: String::new(),
            hex_right: String::new(),
            hex_outline: String::new(),
            hotkey_buf: String::new(),
        };
        app.sync_hex_buffers();
        app
    }

    // ---- 辅助 ----

    fn sync_hex_buffers(&mut self) {
        let f = |s: &str| if s.starts_with('#') { s.to_string() } else { format!("#{s}") };
        self.hex_main = f(&self.preset.color);
        self.hex_top = f(&self.preset.colors.top);
        self.hex_bottom = f(&self.preset.colors.bottom);
        self.hex_left = f(&self.preset.colors.left);
        self.hex_right = f(&self.preset.colors.right);
        self.hex_outline = f(&self.preset.outline.color);
        self.hotkey_buf = self.preset.hotkey_toggle.to_string();
    }

    fn flash(&mut self, text: String) {
        self.status = Some((text, Instant::now()));
    }

    fn save_current(&mut self) {
        let result = {
            let mut store = self.store.lock().unwrap();
            let res = store.save_preset(&self.active_name.clone(), &self.preset);
            if res.is_ok() {
                let _ = store.save_app();
            }
            res
        };
        match result {
            Ok(()) => self.flash(format!("{} {}", t(self.lang, "saved"), self.active_name)),
            Err(e) => self.flash(format!("{}: {e}", t(self.lang, "error"))),
        }
    }

    fn apply_preset(&mut self, name: &str) {
        let name = name.to_string();
        {
            let mut store = self.store.lock().unwrap();
            if let Some(p) = store.get(&name).cloned() {
                self.preset = p;
                self.active_name = name.clone();
                store.activate(&name);
                let _ = store.save_app();
            }
        }
        self.sync_hex_buffers();
        self.dirty = true;
    }

    /// 实时推送最新参数到后台壳进程（WM_COPYDATA，33ms 节流）。
    /// 后端起覆盖层/热键/手柄即时更新；找不到后端（异常）则静默，保存仍走文件。
    fn send_to_backend(&mut self) {
        let now = std::time::Instant::now();
        if now.duration_since(self.last_send) < std::time::Duration::from_millis(33) {
            return;
        }
        self.last_send = now;
        let json = crate::ipc::preset_payload(&self.preset, self.visible);
        if let Some(hwnd) = crate::ipc::find_backend() {
            let _ = crate::ipc::send_json(hwnd, crate::ipc::IPC_TAG_PRESET, &json);
        }
    }

    // ---- UI 区块 ----

    fn header_ui(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} {}", t(self.lang, "title"), crate::VERSION))
                    .size(19.0)
                    .strong()
                    .color(ACCENT),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                // 主题
                ComboBox::from_id_salt("theme")
                    .width(90.0)
                    .selected_text(match self.theme.as_str() {
                        "light" => t(self.lang, "theme_light"),
                        "dark" => t(self.lang, "theme_dark"),
                        _ => t(self.lang, "theme_auto"),
                    })
                    .show_ui(ui, |ui| {
                        for (val, key) in [("auto", "theme_auto"), ("light", "theme_light"), ("dark", "theme_dark")] {
                            if ui.selectable_label(self.theme == val, t(self.lang, key)).clicked() {
                                self.theme = val.to_string();
                            }
                        }
                    });
                // 语言
                ComboBox::from_id_salt("lang")
                    .width(60.0)
                    .selected_text(match self.lang {
                        Lang::Zh => "中文",
                        Lang::En => "EN",
                    })
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(self.lang == Lang::Zh, "中文").clicked() {
                            self.lang = Lang::Zh;
                        }
                        if ui.selectable_label(self.lang == Lang::En, "English").clicked() {
                            self.lang = Lang::En;
                        }
                    });
            });
        });
        ui.label(RichText::new(t(self.lang, "subtitle")).size(11.0).color(ui.visuals().weak_text_color()));
        ui.horizontal(|ui| {
            // v1.1.0：设置进程独立——本窗口关闭即退出本进程，准星与托盘仍在主进程后台运行
            ui.label(RichText::new(t(self.lang, "close_quits")).size(10.5).weak());
        });
        ui.add_space(6.0);

        // 语言/主题变更持久化
        let mut store = self.store.lock().unwrap();
        if store.app.language != self.lang.code() || store.app.theme != self.theme {
            store.app.language = self.lang.code().to_string();
            store.app.theme = self.theme.clone();
            let _ = store.save_app();
        }
    }

    fn preview_ui(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let label = if self.visible { t(self.lang, "hide") } else { t(self.lang, "show") };
            if ui.button(label).clicked() {
                self.visible = !self.visible;
                self.dirty = true;
            }
            ui.label(RichText::new(t(self.lang, "preview")).size(13.0).weak());
        });
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 200.0), egui::Sense::hover());
        preview::paint_preview(ui, rect, &self.preset);
        ui.add_space(4.0);
    }

    fn section_title(ui: &mut Ui, text: &str) {
        ui.add_space(6.0);
        ui.label(RichText::new(text).size(15.0).strong().color(ACCENT));
        ui.separator();
    }

    fn shape_style_ui(&mut self, ui: &mut Ui) {
        Self::section_title(ui, t(self.lang, "shape_style"));

        // ---- 风格模板（一键套用 Crosshair X 风格） ----
        ui.horizontal(|ui| {
            ui.label(RichText::new(t(self.lang, "templates")).size(12.0).strong());
            ComboBox::from_id_salt("tpl")
                .width(160.0)
                .selected_text(self.template_name)
                .show_ui(ui, |ui| {
                    for (key, _apply) in TEMPLATES {
                        if ui.selectable_label(self.template_name == key, t(self.lang, key)).clicked() {
                            self.template_name = key;
                        }
                    }
                });
            if ui.button(t(self.lang, "template_apply")).clicked() {
                if let Some((_key, apply)) = TEMPLATES.iter().find(|(k, _)| *k == self.template_name) {
                    apply(&mut self.preset);
                    self.sync_hex_buffers();
                    self.dirty = true;
                    self.flash(t(self.lang, "template_applied").to_string());
                }
            }
        });
        ui.add_space(2.0);

        egui::Grid::new("shape_grid").num_columns(3).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label(t(self.lang, "shape"));
            ComboBox::from_id_salt("shape")
                .width(180.0)
                .selected_text(shape_name(self.lang, self.preset.shape))
                .show_ui(ui, |ui| {
                    for s in Shape::ALL {
                        if ui.selectable_value(&mut self.preset.shape, s, shape_name(self.lang, s)).changed() {
                            self.dirty = true;
                        }
                    }
                });
            ui.label("");
            ui.end_row();

            ui.label(t(self.lang, "size"));
            if ui.add(egui::Slider::new(&mut self.preset.size, 1.0..=200.0).show_value(true)).changed() {
                self.dirty = true;
            }
            ui.end_row();

            ui.label(t(self.lang, "thickness"));
            if ui.add(egui::Slider::new(&mut self.preset.thickness, 0.2..=20.0).show_value(true)).changed() {
                self.dirty = true;
            }
            ui.end_row();

            ui.label(t(self.lang, "opacity"));
            if ui.add(egui::Slider::new(&mut self.preset.opacity, 0.05..=1.0).show_value(true)).changed() {
                self.dirty = true;
            }
            ui.end_row();

            ui.label(t(self.lang, "rotation"));
            if ui.add(egui::Slider::new(&mut self.preset.rotation, 0.0..=360.0).show_value(true).suffix("°")).changed() {
                self.dirty = true;
            }
            ui.end_row();

            if ui.checkbox(&mut self.preset.multicolor, t(self.lang, "multicolor")).changed() {
                self.dirty = true;
            }
            ui.end_row();
        });

        // 颜色编辑（主色 + 可选四象限）
        ui.add_space(4.0);
        if self.preset.multicolor {
            color_row_ui(ui, t(self.lang, "color_top"), &mut self.hex_top, &mut self.preset.colors.top, &mut self.dirty);
            color_row_ui(ui, t(self.lang, "color_bottom"), &mut self.hex_bottom, &mut self.preset.colors.bottom, &mut self.dirty);
            color_row_ui(ui, t(self.lang, "color_left"), &mut self.hex_left, &mut self.preset.colors.left, &mut self.dirty);
            color_row_ui(ui, t(self.lang, "color_right"), &mut self.hex_right, &mut self.preset.colors.right, &mut self.dirty);
        } else {
            color_row_ui(ui, t(self.lang, "main_color"), &mut self.hex_main, &mut self.preset.color, &mut self.dirty);
        }

        // 空心参数（仅相关形状显示）
        let has_gap = matches!(
            self.preset.shape,
            Shape::HollowCross | Shape::HollowSquare | Shape::HollowCrossDot | Shape::GapHair
        );
        if has_gap {
            ui.horizontal(|ui| {
                ui.label(t(self.lang, "hollow_gap"));
                if ui.add(egui::Slider::new(&mut self.preset.hollow.gap, 0.0..=80.0).show_value(true)).changed() {
                    self.dirty = true;
                }
            });
            if matches!(self.preset.shape, Shape::HollowCrossDot | Shape::GapHair) {
                ui.horizontal(|ui| {
                    ui.label(t(self.lang, "center_dot"));
                    if ui.add(egui::Slider::new(&mut self.preset.hollow.center_dot_size, 1.0..=30.0).show_value(true)).changed() {
                        self.dirty = true;
                    }
                });
            }
        }

        // 描边
        ui.add_space(4.0);
        if ui.checkbox(&mut self.preset.outline.enabled, t(self.lang, "outline")).changed() {
            self.dirty = true;
        }
        if self.preset.outline.enabled {
            ui.horizontal(|ui| {
                ui.label(t(self.lang, "outline_thickness"));
                if ui.add(egui::Slider::new(&mut self.preset.outline.thickness, 0.5..=10.0).show_value(true)).changed() {
                    self.dirty = true;
                }
            });
            color_row_ui(ui, t(self.lang, "outline_color"), &mut self.hex_outline, &mut self.preset.outline.color, &mut self.dirty);
            ui.horizontal(|ui| {
                ui.label(t(self.lang, "outline_opacity"));
                if ui.add(egui::Slider::new(&mut self.preset.outline.opacity, 0.1..=1.0).show_value(true)).changed() {
                    self.dirty = true;
                }
            });
        }
    }
}

/// 风格模板：借鉴 Crosshair X 的常见预设风格，一键套用参数。
/// key 同时是 strings.rs 的文案 key。
const TEMPLATES: [(&str, fn(&mut Preset)); 8] = [
    ("tpl_apex", |p: &mut Preset| {
        // Apex 风格：四段 + 中心点，半透明白
        p.shape = Shape::HollowCrossDot;
        p.size = 14.0;
        p.thickness = 1.6;
        p.hollow.gap = 3.0;
        p.color = "#FFFFFF".into();
        p.multicolor = false;
        p.opacity = 0.85;
        p.hollow.center_dot_size = 2.0;
        p.dynamic.fire_expand_px = 4.0;
        p.dynamic.recover_ms = 140;
    }),
    ("tpl_valorant", |p: &mut Preset| {
        // Valorant 风格：瘦四段，青绿色
        p.shape = Shape::Gate;
        p.size = 16.0;
        p.thickness = 1.0;
        p.hollow.gap = 2.0;
        p.color = "#3DFF6E".into();
        p.multicolor = false;
        p.opacity = 0.9;
    }),
    ("tpl_cs2", |p: &mut Preset| {
        // CS2 风格：经典绿十字
        p.shape = Shape::Cross;
        p.size = 18.0;
        p.thickness = 1.5;
        p.hollow.gap = 0.0;
        p.color = "#00FF00".into();
        p.multicolor = false;
        p.opacity = 0.9;
    }),
    ("tpl_sniper", |p: &mut Preset| {
        // 狙击风格：大圆环 + 中心点
        p.shape = Shape::RingDot;
        p.size = 26.0;
        p.thickness = 2.0;
        p.color = "#FFFFFF".into();
        p.multicolor = false;
        p.opacity = 0.7;
        p.hollow.center_dot_size = 2.5;
    }),
    ("tpl_classic", |p: &mut Preset| {
        // 经典默认：红十字
        p.shape = Shape::Cross;
        p.size = 20.0;
        p.thickness = 2.0;
        p.hollow.gap = 0.0;
        p.color = "#FF0000".into();
        p.multicolor = false;
        p.opacity = 0.8;
    }),
    ("tpl_thick_gate", |p: &mut Preset| {
        // 厚门形：粗线段，战斗可见性
        p.shape = Shape::Gate;
        p.size = 24.0;
        p.thickness = 4.0;
        p.hollow.gap = 3.0;
        p.color = "#FFFFFF".into();
        p.multicolor = false;
        p.opacity = 1.0;
    }),
    ("tpl_dot", |p: &mut Preset| {
        // 纯点：最小遮挡
        p.shape = Shape::Dot;
        p.size = 6.0;
        p.color = "#FF2D2D".into();
        p.opacity = 0.9;
    }),
    ("tpl_cross_x", |p: &mut Preset| {
        // X 形准星
        p.shape = Shape::XShape;
        p.size = 18.0;
        p.thickness = 2.0;
        p.color = "#FFA500".into();
        p.multicolor = false;
        p.opacity = 0.85;
    }),
];

/// 颜色编辑行：色块 + hex 输入（静态函数避免 self 双重借用）
fn color_row_ui(
    ui: &mut Ui,
    label: &str,
    buf: &mut String,
    target: &mut String,
    dirty: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        // 色块
        let (r, g, b) = crate::overlay::parse_hex(target);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 20.0), egui::Sense::hover());
        ui.painter().rect_filled(
            rect,
            4.0,
            Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8),
        );
        let resp = ui.add(
            TextEdit::singleline(buf)
                .hint_text("#RRGGBB")
                .desired_width(110.0),
        );
        if resp.changed() {
            let s = buf.trim().trim_start_matches('#');
            if s.len() == 6 && u32::from_str_radix(s, 16).is_ok() {
                *target = format!("#{}", s.to_uppercase());
                *dirty = true;
            }
        }
    });
}

impl AcajaApp {

    fn dynamic_ui(&mut self, ui: &mut Ui) {
        Self::section_title(ui, t(self.lang, "dynamic"));
        ui.horizontal(|ui| {
            ui.label(t(self.lang, "fire_expand"));
            if ui.add(egui::Slider::new(&mut self.preset.dynamic.fire_expand_px, 0.0..=80.0).show_value(true)).changed() {
                self.dirty = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label(t(self.lang, "recover_ms"));
            if ui.add(egui::Slider::new(&mut self.preset.dynamic.recover_ms, 20..=500).show_value(true)).changed() {
                self.dirty = true;
            }
        });
        if ui.checkbox(&mut self.preset.dynamic.recoil_indicator, t(self.lang, "recoil_indicator")).changed() {
            self.dirty = true;
        }
    }

    fn position_ui(&mut self, ui: &mut Ui) {
        Self::section_title(ui, t(self.lang, "position"));
        ui.horizontal(|ui| {
            if ui.button(t(self.lang, "center_btn")).clicked() {
                self.preset.position.x = PosVal::Center;
                self.preset.position.y = PosVal::Center;
                self.dirty = true;
            }
            ui.label(t(self.lang, "pos_x"));
            let mut x = match self.preset.position.x {
                PosVal::Px(v) => v as i32,
                _ => 0,
            };
            if ui.add(egui::DragValue::new(&mut x).speed(2.0)).changed() {
                self.preset.position.x = PosVal::Px(x as f32);
                self.dirty = true;
            }
            ui.label(t(self.lang, "pos_y"));
            let mut y = match self.preset.position.y {
                PosVal::Px(v) => v as i32,
                _ => 0,
            };
            if ui.add(egui::DragValue::new(&mut y).speed(2.0)).changed() {
                self.preset.position.y = PosVal::Px(y as f32);
                self.dirty = true;
            }
            ui.label(t(self.lang, "monitor"));
            let mut mon = self.preset.position.monitor;
            if ui.add(egui::DragValue::new(&mut mon).speed(1.0).range(-1..=7)).changed() {
                self.preset.position.monitor = mon;
                self.dirty = true;
            }
        });
    }

    fn image_ui(&mut self, ui: &mut Ui) {
        Self::section_title(ui, t(self.lang, "custom_image"));
        ui.horizontal(|ui| {
            ui.label(t(self.lang, "image_path"));
            if ui
                .add(TextEdit::singleline(&mut self.preset.image.path).desired_width(300.0))
                .changed()
            {
                self.dirty = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label(t(self.lang, "image_scale"));
            if ui.add(egui::Slider::new(&mut self.preset.image.scale, 0.1..=5.0).show_value(true)).changed() {
                self.dirty = true;
            }
        });
    }

    fn hotkey_ui(&mut self, ui: &mut Ui) {
        Self::section_title(ui, t(self.lang, "hotkey"));
        ui.horizontal(|ui| {
            ui.label(t(self.lang, "hotkey_toggle"));
            if ui
                .add(TextEdit::singleline(&mut self.hotkey_buf).desired_width(140.0))
                .changed()
            {
                self.preset.hotkey_toggle = Hotkey::parse(&self.hotkey_buf).unwrap_or_default();
                self.dirty = true;
            }
        });
        ui.label(RichText::new(t(self.lang, "hotkey_note")).size(10.5).weak());

        // 右键切换
        if ui.checkbox(&mut self.preset.right_click_toggle, t(self.lang, "right_click")).changed() {
            self.dirty = true;
        }
        if self.preset.right_click_toggle {
            ui.horizontal(|ui| {
                ui.label(t(self.lang, "right_click_mode"));
                ComboBox::from_id_salt("rc_mode")
                    .width(130.0)
                    .selected_text(match self.preset.right_click_mode {
                        RightClickMode::Click => t(self.lang, "rc_click"),
                        RightClickMode::HoldShow => t(self.lang, "rc_hold_show"),
                        RightClickMode::HoldHide => t(self.lang, "rc_hold_hide"),
                    })
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut self.preset.right_click_mode, RightClickMode::Click, t(self.lang, "rc_click"))
                            .changed()
                        {
                            self.dirty = true;
                        }
                        if ui
                            .selectable_value(&mut self.preset.right_click_mode, RightClickMode::HoldShow, t(self.lang, "rc_hold_show"))
                            .changed()
                        {
                            self.dirty = true;
                        }
                        if ui
                            .selectable_value(&mut self.preset.right_click_mode, RightClickMode::HoldHide, t(self.lang, "rc_hold_hide"))
                            .changed()
                        {
                            self.dirty = true;
                        }
                    });
            });
        }
    }

    fn gamepad_ui(&mut self, ui: &mut Ui) {
        Self::section_title(ui, t(self.lang, "gamepad"));
        ui.horizontal(|ui| {
            ui.label(t(self.lang, "ads_mode"));
            ComboBox::from_id_salt("ads_mode")
                .width(130.0)
                .selected_text(ads_mode_name(self.lang, self.preset.gamepad.ads_mode))
                .show_ui(ui, |ui| {
                    for m in [AdsMode::Off, AdsMode::HoldHide, AdsMode::Toggle, AdsMode::HoldShow] {
                        if ui.selectable_value(&mut self.preset.gamepad.ads_mode, m, ads_mode_name(self.lang, m)).changed() {
                            self.dirty = true;
                        }
                    }
                });
            ui.label(t(self.lang, "ads_button"));
            ComboBox::from_id_salt("ads_button")
                .width(170.0)
                .selected_text(match self.preset.gamepad.ads_button {
                    AdsButton::LeftTrigger => t(self.lang, "ads_left_trigger"),
                    AdsButton::RightTrigger => t(self.lang, "ads_right_trigger"),
                    AdsButton::LeftBumper => t(self.lang, "ads_left_bumper"),
                    AdsButton::RightBumper => t(self.lang, "ads_right_bumper"),
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut self.preset.gamepad.ads_button, AdsButton::LeftTrigger, t(self.lang, "ads_left_trigger"))
                        .changed()
                    {
                        self.dirty = true;
                    }
                    if ui
                        .selectable_value(&mut self.preset.gamepad.ads_button, AdsButton::RightTrigger, t(self.lang, "ads_right_trigger"))
                        .changed()
                    {
                        self.dirty = true;
                    }
                    if ui
                        .selectable_value(&mut self.preset.gamepad.ads_button, AdsButton::LeftBumper, t(self.lang, "ads_left_bumper"))
                        .changed()
                    {
                        self.dirty = true;
                    }
                    if ui
                        .selectable_value(&mut self.preset.gamepad.ads_button, AdsButton::RightBumper, t(self.lang, "ads_right_bumper"))
                        .changed()
                    {
                        self.dirty = true;
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label(t(self.lang, "trigger_threshold"));
            let mut t = self.preset.gamepad.trigger_threshold as u32;
            if ui.add(egui::Slider::new(&mut t, 0..=255).show_value(true)).changed() {
                self.preset.gamepad.trigger_threshold = t as u8;
                self.dirty = true;
            }
        });
        if ui.checkbox(&mut self.preset.gamepad.fire_expand, t(self.lang, "gamepad_fire_expand")).changed() {
            self.dirty = true;
        }
        ui.label(RichText::new(t(self.lang, "gamepad_note")).size(10.5).weak());
    }

    fn presets_ui(&mut self, ui: &mut Ui) {
        Self::section_title(ui, t(self.lang, "presets"));
        ui.horizontal(|ui| {
            ui.label(t(self.lang, "preset_current"));
            let names = {
                let store = self.store.lock().unwrap();
                store.preset_names()
            };
            ComboBox::from_id_salt("preset")
                .width(160.0)
                .selected_text(self.active_name.clone())
                .show_ui(ui, |ui| {
                    for n in &names {
                        if ui
                            .selectable_label(n == &self.active_name, n.clone())
                            .clicked()
                        {
                            self.apply_preset(n);
                        }
                    }
                });
            if ui.button(t(self.lang, "save")).clicked() {
                self.save_current();
            }
            if ui.button(t(self.lang, "delete")).clicked() {
                if self.active_name == "default" {
                    self.flash(t(self.lang, "cannot_delete_default").to_string());
                } else {
                    let (ok, name) = {
                        let mut store = self.store.lock().unwrap();
                        let name = self.active_name.clone();
                        let ok = store.delete_preset(&name).unwrap_or(false);
                        if ok {
                            let _ = store.save_app();
                        }
                        (ok, name)
                    };
                    if ok {
                        self.flash(format!("{} {}", t(self.lang, "deleted"), name));
                        self.apply_preset("default");
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(t(self.lang, "new_name"));
            ui.add(TextEdit::singleline(&mut self.new_preset_name).desired_width(140.0));
            if ui.button(t(self.lang, "create")).clicked() {
                let name = self.new_preset_name.trim().to_string();
                if !name.is_empty() {
                    let result = {
                        let mut store = self.store.lock().unwrap();
                        let res = store.save_preset(&name, &self.preset);
                        if res.is_ok() {
                            let _ = store.save_app();
                        }
                        res
                    };
                    match result {
                        Ok(()) => {
                            self.active_name = name;
                            self.flash(format!("{} {}", t(self.lang, "created"), self.active_name));
                        }
                        Err(e) => self.flash(format!("{}: {e}", t(self.lang, "error"))),
                    }
                }
            }
        });
        ui.label(RichText::new(t(self.lang, "new_preset")).size(10.5).weak());
    }
}

impl eframe::App for AcajaApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // ---- 主题 ----
        let dark = match self.theme.as_str() {
            "light" => false,
            "dark" => true,
            _ => true, // auto → 深色（Windows 主流）
        };
        if dark {
            ctx.set_visuals(Visuals::dark());
        } else {
            ctx.set_visuals(Visuals::light());
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().inner_margin(12.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.header_ui(ui);
                        self.preview_ui(ui);

                        self.shape_style_ui(ui);
                        self.dynamic_ui(ui);
                        self.position_ui(ui);
                        self.image_ui(ui);
                        self.hotkey_ui(ui);
                        self.gamepad_ui(ui);
                        self.presets_ui(ui);

                        // 状态提示
                        if let Some((text, at)) = &self.status {
                            if at.elapsed() < STATUS_TTL {
                                ui.add_space(4.0);
                                ui.label(RichText::new(text).color(Color32::from_rgb(48, 209, 88)));
                            } else {
                                self.status = None;
                            }
                        }
                    });
            });

        // ---- 帧末推送后端进程 ----
        if self.dirty {
            self.dirty = false;
            self.send_to_backend();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // 退出前自动保存当前准星设置 → 下次打开沿用（不需要手动点保存）
        let (name, preset) = (self.active_name.clone(), self.preset.clone());
        let result = {
            let mut store = self.store.lock().unwrap();
            let res = store.save_preset(&name, &preset);
            if res.is_ok() {
                let _ = store.save_app();
            }
            res
        };
        match result {
            Ok(()) => info!("退出前已自动保存预设 {name}"),
            Err(e) => warn!("退出前自动保存失败: {e}"),
        }
    }
}