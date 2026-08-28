//! ACAJA 设置窗口（v1.1.6 现代化重设计）。
//!
//! 布局：品牌顶栏 → 左侧图标导航 + 右侧内容卡片 → 底部操作条（应用/退出主程序）。
//! 主题：深色分层视觉（自定义 Visuals + 卡片圆角）。
//! 行为：工作副本模式；「应用」= 保存文件 + 尽力实时推送给后台壳；
//! 「退出主程序」= 写命令文件，由后台壳自动检测执行。

pub mod fonts;
pub mod preview;
pub mod strings;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use egui::{Align, Color32, ComboBox, Context, Margin, RichText, Rounding, TextEdit, Ui, Visuals};
use log::{info, warn};

use crate::config::{
    AdsButton, AdsMode, Hotkey, PosVal, Preset, PresetStore, RightClickMode, Shape,
};
use crate::i18n::Lang;
use crate::ui::strings::{ads_mode_name, shape_name, t};

const ACCENT: Color32 = Color32::from_rgb(10, 132, 255);
const ACCENT_SOFT: Color32 = Color32::from_rgb(16, 46, 82);
const CARD_BG: Color32 = Color32::from_rgb(31, 35, 45);
const CARD_BG_HOVER: Color32 = Color32::from_rgb(37, 42, 54);
const BG: Color32 = Color32::from_rgb(20, 22, 28);
const BORDER: Color32 = Color32::from_rgb(42, 47, 58);
const TEXT_DIM: Color32 = Color32::from_rgb(138, 144, 160);
const OK: Color32 = Color32::from_rgb(48, 209, 88);
const WARN: Color32 = Color32::from_rgb(255, 159, 10);
const STATUS_TTL: std::time::Duration = std::time::Duration::from_millis(1800);

/// 导航 section 索引
const SEC_STYLE: usize = 0;
const SEC_DYNAMIC: usize = 1;
const SEC_POSITION: usize = 2;
const SEC_GAMEPAD: usize = 3;
const SEC_HOTKEY: usize = 4;
const SEC_IMAGE: usize = 5;
const SEC_PRESETS: usize = 6;

pub struct AcajaApp {
    store: Arc<Mutex<PresetStore>>,
    last_send: std::time::Instant,
    /// 后端连接状态（找到主进程窗口则 true）
    backend_ok: bool,
    pending_send: bool,
    last_apply_at: std::time::Instant,
    template_name: &'static str,
    /// 当前导航 section
    active_section: usize,

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
pub fn run(store: Arc<Mutex<PresetStore>>, title: &'static str) -> eframe::Result<()> {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([860.0, 780.0])
        .with_min_inner_size([760.0, 620.0])
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

        // ---- 现代深色主题（分层视觉） ----
        let mut visuals = Visuals::dark();
        visuals.panel_fill = BG;
        visuals.window_fill = BG;
        visuals.extreme_bg_color = BG;
        visuals.faint_bg_color = Color32::from_rgb(24, 26, 33);
        visuals.override_text_color = Some(Color32::from_rgb(229, 231, 237));
        visuals.selection.bg_fill = ACCENT;
        visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
        visuals.widgets.noninteractive.bg_fill = CARD_BG;
        visuals.widgets.noninteractive.weak_bg_fill = CARD_BG;
        visuals.widgets.inactive.bg_fill = CARD_BG;
        visuals.widgets.inactive.weak_bg_fill = CARD_BG;
        visuals.widgets.hovered.bg_fill = CARD_BG_HOVER;
        visuals.widgets.hovered.weak_bg_fill = CARD_BG_HOVER;
        visuals.widgets.active.bg_fill = ACCENT_SOFT;
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, BORDER);
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, BORDER);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, ACCENT);
        cc.egui_ctx.set_visuals(visuals);

        let mut app = AcajaApp {
            store,
            last_send: std::time::Instant::now(),
            backend_ok: false,
            pending_send: false,
            last_apply_at: std::time::Instant::now(),
            template_name: "tpl_apex",
            active_section: SEC_STYLE,
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

    /// 推送按钮：保存文件 + 尽力实时推送给后台壳
    fn push_to_backend(&mut self) {
        let (name, preset) = (self.active_name.clone(), self.preset.clone());
        let saved = {
            let mut st = self.store.lock().unwrap();
            let r = st.save_preset(&name, &preset);
            if r.is_ok() {
                let _ = st.save_app();
            }
            r.is_ok()
        };
        let pushed = if let Some(hwnd) = crate::ipc::find_backend() {
            crate::ipc::send_json(
                hwnd,
                crate::ipc::IPC_TAG_PRESET,
                &crate::ipc::preset_payload(&self.preset, self.visible),
            )
        } else {
            false
        };
        self.backend_ok = pushed;
        self.last_apply_at = Instant::now();
        if saved {
            self.flash(if pushed {
                t(self.lang, "pushed_ok").to_string()
            } else {
                t(self.lang, "pushed_via_file").to_string()
            });
        } else {
            self.flash(t(self.lang, "error").to_string());
        }
    }

    // ================================================================
    // 布局组件
    // ================================================================

    /// 品牌顶栏：品牌标记 + 标题 + 版本 + 语言/主题
    fn top_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            // 品牌方块
            let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, Rounding::same(6.0), ACCENT);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "A",
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );
            ui.label(
                RichText::new(format!("{}  v{}", t(self.lang, "title"), crate::VERSION))
                    .size(19.0)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                // 主题
                ComboBox::from_id_salt("theme_bar")
                    .width(86.0)
                    .selected_text(match self.theme.as_str() {
                        "light" => t(self.lang, "theme_light"),
                        "dark" => t(self.lang, "theme_dark"),
                        _ => t(self.lang, "theme_auto"),
                    })
                    .show_ui(ui, |ui| {
                        for (val, key) in [
                            ("auto", "theme_auto"),
                            ("light", "theme_light"),
                            ("dark", "theme_dark"),
                        ] {
                            if ui.selectable_label(self.theme == val, t(self.lang, key)).clicked() {
                                self.theme = val.to_string();
                            }
                        }
                    });
                // 语言
                ComboBox::from_id_salt("lang_bar")
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
        // 语言/主题变更持久化
        let mut store = self.store.lock().unwrap();
        if store.app.language != self.lang.code() || store.app.theme != self.theme {
            store.app.language = self.lang.code().to_string();
            store.app.theme = self.theme.clone();
            let _ = store.save_app();
        }
    }

    /// 左侧图标导航
    fn nav_ui(&mut self, ui: &mut Ui) {
        let items: [(usize, &str, &str); 7] = [
            (SEC_STYLE, "🎯", "shape_style"),
            (SEC_DYNAMIC, "⚡", "dynamic"),
            (SEC_POSITION, "📍", "position"),
            (SEC_GAMEPAD, "🎮", "gamepad"),
            (SEC_HOTKEY, "⌨️", "hotkey"),
            (SEC_IMAGE, "🖼️", "custom_image"),
            (SEC_PRESETS, "📁", "presets"),
        ];
        for (idx, icon, key) in items {
            let selected = self.active_section == idx;
            let text = format!("{icon}  {}", t(self.lang, key));
            let resp = ui.selectable_label(selected, RichText::new(text).size(13.0));
            let rect = resp.rect;
            if selected {
                // 左侧强调条 + 底色
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(rect.min - egui::vec2(2.0, 2.0), rect.max + egui::vec2(2.0, 2.0)),
                    Rounding::same(6.0),
                    ACCENT_SOFT,
                );
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        rect.min + egui::vec2(2.0, 4.0),
                        egui::vec2(3.0, rect.height() - 8.0),
                    ),
                    Rounding::same(1.5),
                    ACCENT,
                );
            }
            if resp.hovered() && !selected {
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(rect.min - egui::vec2(2.0, 2.0), rect.max + egui::vec2(2.0, 2.0)),
                    Rounding::same(6.0),
                    Color32::from_rgba_unmultiplied(255, 255, 255, 8),
                );
            }
            if resp.clicked() {
                self.active_section = idx;
            }
            ui.add_space(2.0);
        }
    }

    /// 底部操作条：连接状态 + 应用 / 退出主程序
    fn bottom_bar(&mut self, ui: &mut Ui) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 44.0),
            egui::Sense::hover(),
        );
        let painter = ui.painter();
        // 卡片底色 + 上边框
        painter.rect_filled(rect, Rounding::same(10.0), CARD_BG);
        painter.hline(rect.left() + 12.0, rect.right() - 12.0, rect.top() + 12.0, egui::Stroke::new(1.0, BORDER));

        let inner = rect.shrink2(egui::vec2(12.0, 8.0));
        let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(Align::Center)));
        bar_ui.set_clip_rect(inner);
        // 状态文字
        if self.backend_ok {
            let color = if self.preset.dirty { WARN } else { OK };
            bar_ui.label(RichText::new(t(self.lang, "backend_connected")).size(11.0).color(color));
        } else {
            bar_ui.label(
                RichText::new(t(self.lang, "backend_file_mode")).size(11.0).color(WARN),
            );
        }
        bar_ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            // 状态 flash（临时提示）
            if let Some((text, at)) = &self.status {
                if at.elapsed() < STATUS_TTL {
                    ui.label(RichText::new(text).size(11.0).color(OK));
                } else {
                    self.status = None;
                }
            }
            ui.add_space(4.0);
            if ui
                .add(
                    egui::Button::new(RichText::new(t(self.lang, "quit_backend")).size(12.5))
                        .min_size(egui::vec2(96.0, 28.0)),
                )
                .clicked()
            {
                if let Some(appdata) = crate::appdata_dir().ok() {
                    let _ = std::fs::write(appdata.join("cmd.json"), "{\"cmd\":\"quit\"}");
                    self.flash(t(self.lang, "quit_backend_sent").to_string());
                }
            }
            ui.add_space(6.0);
            if ui
                .add(
                    egui::Button::new(RichText::new(t(self.lang, "push_apply")).size(13.5).strong())
                        .fill(ACCENT)
                        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(60, 140, 255)))
                        .min_size(egui::vec2(150.0, 30.0)),
                )
                .clicked()
            {
                self.push_to_backend();
            }
        });
    }

    /// 卡片容器
    fn card(ui: &mut Ui, title: Option<&str>, add_contents: impl FnOnce(&mut Ui)) {
        let frame = egui::Frame::none()
            .fill(CARD_BG)
            .rounding(Rounding::same(10.0))
            .inner_margin(Margin::same(14.0));
        frame.show(ui, |ui| {
            if let Some(t) = title {
                ui.label(RichText::new(t).size(13.5).strong().color(ACCENT));
                ui.add_space(8.0);
            }
            add_contents(ui);
        });
        ui.add_space(8.0);
    }

    // ================================================================
    // Section 内容
    // ================================================================

    fn section_style(&mut self, ui: &mut Ui) {
        Self::card(ui, None, |ui| {
            // 预览画布
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 210.0), egui::Sense::hover());
            preview::paint_preview(ui, rect, &self.preset);
            ui.add_space(6.0);
            ui.label(RichText::new(t(self.lang, "preview")).size(10.5).color(TEXT_DIM));
        });

        Self::card(ui, Some(t(self.lang, "templates")), |ui| {
            ui.horizontal(|ui| {
                ComboBox::from_id_salt("tpl_new")
                    .width(180.0)
                    .selected_text(t(self.lang, self.template_name))
                    .show_ui(ui, |ui| {
                        for (key, _apply) in TEMPLATES {
                            if ui
                                .selectable_label(self.template_name == key, t(self.lang, key))
                                .clicked()
                            {
                                self.template_name = key;
                            }
                        }
                    });
                if ui
                    .add(egui::Button::new(t(self.lang, "template_apply")).min_size(egui::vec2(72.0, 24.0)))
                    .clicked()
                {
                    if let Some((_key, apply)) =
                        TEMPLATES.iter().find(|(k, _)| *k == self.template_name)
                    {
                        apply(&mut self.preset);
                        self.sync_hex_buffers();
                        self.dirty = true;
                        self.flash(t(self.lang, "template_applied").to_string());
                    }
                }
            });
        });

        Self::card(ui, Some(t(self.lang, "shape_style")), |ui| {
            egui::Grid::new("g_shape").num_columns(3).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label(t(self.lang, "shape"));
                ComboBox::from_id_salt("shape_new")
                    .width(190.0)
                    .selected_text(shape_name(self.lang, self.preset.shape))
                    .show_ui(ui, |ui| {
                        for s in Shape::ALL {
                            if ui
                                .selectable_value(&mut self.preset.shape, s, shape_name(self.lang, s))
                                .changed()
                            {
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

            ui.add_space(6.0);
            if self.preset.multicolor {
                color_row_ui(ui, t(self.lang, "color_top"), &mut self.hex_top, &mut self.preset.colors.top, &mut self.dirty);
                color_row_ui(ui, t(self.lang, "color_bottom"), &mut self.hex_bottom, &mut self.preset.colors.bottom, &mut self.dirty);
                color_row_ui(ui, t(self.lang, "color_left"), &mut self.hex_left, &mut self.preset.colors.left, &mut self.dirty);
                color_row_ui(ui, t(self.lang, "color_right"), &mut self.hex_right, &mut self.preset.colors.right, &mut self.dirty);
            } else {
                color_row_ui(ui, t(self.lang, "main_color"), &mut self.hex_main, &mut self.preset.color, &mut self.dirty);
            }

            let has_gap = matches!(
                self.preset.shape,
                Shape::HollowCross | Shape::HollowSquare | Shape::HollowCrossDot | Shape::GapHair
            );
            if has_gap {
                ui.add_space(6.0);
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

            ui.add_space(6.0);
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
        });
    }

    fn section_dynamic(&mut self, ui: &mut Ui) {
        Self::card(ui, Some(t(self.lang, "dynamic")), |ui| {
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
        });
    }

    fn section_position(&mut self, ui: &mut Ui) {
        Self::card(ui, Some(t(self.lang, "position")), |ui| {
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(t(self.lang, "center_btn")).min_size(egui::vec2(96.0, 24.0))).clicked() {
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
        });
    }

    fn section_gamepad(&mut self, ui: &mut Ui) {
        Self::card(ui, Some(t(self.lang, "gamepad")), |ui| {
            ui.horizontal(|ui| {
                ui.label(t(self.lang, "ads_mode"));
                ComboBox::from_id_salt("ads_mode_new")
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
                ComboBox::from_id_salt("ads_button_new")
                    .width(170.0)
                    .selected_text(match self.preset.gamepad.ads_button {
                        AdsButton::LeftTrigger => t(self.lang, "ads_left_trigger"),
                        AdsButton::RightTrigger => t(self.lang, "ads_right_trigger"),
                        AdsButton::LeftBumper => t(self.lang, "ads_left_bumper"),
                        AdsButton::RightBumper => t(self.lang, "ads_right_bumper"),
                    })
                    .show_ui(ui, |ui| {
                        if ui.selectable_value(&mut self.preset.gamepad.ads_button, AdsButton::LeftTrigger, t(self.lang, "ads_left_trigger")).changed() {
                            self.dirty = true;
                        }
                        if ui.selectable_value(&mut self.preset.gamepad.ads_button, AdsButton::RightTrigger, t(self.lang, "ads_right_trigger")).changed() {
                            self.dirty = true;
                        }
                        if ui.selectable_value(&mut self.preset.gamepad.ads_button, AdsButton::LeftBumper, t(self.lang, "ads_left_bumper")).changed() {
                            self.dirty = true;
                        }
                        if ui.selectable_value(&mut self.preset.gamepad.ads_button, AdsButton::RightBumper, t(self.lang, "ads_right_bumper")).changed() {
                            self.dirty = true;
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label(t(self.lang, "trigger_threshold"));
                let mut tr = self.preset.gamepad.trigger_threshold as u32;
                if ui.add(egui::Slider::new(&mut tr, 0..=255).show_value(true)).changed() {
                    self.preset.gamepad.trigger_threshold = tr as u8;
                    self.dirty = true;
                }
            });
            if ui.checkbox(&mut self.preset.gamepad.fire_expand, t(self.lang, "gamepad_fire_expand")).changed() {
                self.dirty = true;
            }
            ui.label(RichText::new(t(self.lang, "gamepad_note")).size(10.5).color(TEXT_DIM));
        });
    }

    fn section_hotkey(&mut self, ui: &mut Ui) {
        Self::card(ui, Some(t(self.lang, "hotkey")), |ui| {
            ui.horizontal(|ui| {
                ui.label(t(self.lang, "hotkey_toggle"));
                if ui.add(TextEdit::singleline(&mut self.hotkey_buf).desired_width(140.0)).changed() {
                    self.preset.hotkey_toggle = Hotkey::parse(&self.hotkey_buf).unwrap_or_default();
                    self.dirty = true;
                }
            });
            ui.label(RichText::new(t(self.lang, "hotkey_note")).size(10.5).color(TEXT_DIM));
            ui.add_space(6.0);
            if ui.checkbox(&mut self.preset.right_click_toggle, t(self.lang, "right_click")).changed() {
                self.dirty = true;
            }
            if self.preset.right_click_toggle {
                ui.horizontal(|ui| {
                    ui.label(t(self.lang, "right_click_mode"));
                    ComboBox::from_id_salt("rc_mode_new")
                        .width(130.0)
                        .selected_text(match self.preset.right_click_mode {
                            RightClickMode::Click => t(self.lang, "rc_click"),
                            RightClickMode::HoldShow => t(self.lang, "rc_hold_show"),
                            RightClickMode::HoldHide => t(self.lang, "rc_hold_hide"),
                        })
                        .show_ui(ui, |ui| {
                            if ui.selectable_value(&mut self.preset.right_click_mode, RightClickMode::Click, t(self.lang, "rc_click")).changed() {
                                self.dirty = true;
                            }
                            if ui.selectable_value(&mut self.preset.right_click_mode, RightClickMode::HoldShow, t(self.lang, "rc_hold_show")).changed() {
                                self.dirty = true;
                            }
                            if ui.selectable_value(&mut self.preset.right_click_mode, RightClickMode::HoldHide, t(self.lang, "rc_hold_hide")).changed() {
                                self.dirty = true;
                            }
                        });
                });
            }
        });
    }

    fn section_image(&mut self, ui: &mut Ui) {
        Self::card(ui, Some(t(self.lang, "custom_image")), |ui| {
            ui.horizontal(|ui| {
                ui.label(t(self.lang, "image_path"));
                if ui.add(TextEdit::singleline(&mut self.preset.image.path).desired_width(280.0)).changed() {
                    self.dirty = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label(t(self.lang, "image_scale"));
                if ui.add(egui::Slider::new(&mut self.preset.image.scale, 0.1..=5.0).show_value(true)).changed() {
                    self.dirty = true;
                }
            });
        });
    }

    fn section_presets(&mut self, ui: &mut Ui) {
        Self::card(ui, Some(t(self.lang, "presets")), |ui| {
            ui.horizontal(|ui| {
                ui.label(t(self.lang, "preset_current"));
                let names = { self.store.lock().unwrap().preset_names() };
                ComboBox::from_id_salt("preset_new")
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
                if ui.add(egui::Button::new(t(self.lang, "save")).min_size(egui::vec2(64.0, 24.0))).clicked() {
                    self.save_current();
                }
                if ui.add(egui::Button::new(t(self.lang, "delete")).min_size(egui::vec2(64.0, 24.0))).clicked() {
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
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(t(self.lang, "new_name"));
                ui.add(TextEdit::singleline(&mut self.new_preset_name).desired_width(140.0));
                if ui.add(egui::Button::new(t(self.lang, "create")).min_size(egui::vec2(72.0, 24.0))).clicked() {
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
            ui.label(RichText::new(t(self.lang, "new_preset")).size(10.5).color(TEXT_DIM));
        });
    }
}

impl eframe::App for AcajaApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // 主题（自动/明/暗）
        let dark = match self.theme.as_str() {
            "light" => false,
            "dark" => true,
            _ => true,
        };
        // 仅切换时重设（Visuals 在 new 已定制 dark；light 用默认）
        if !dark {
            ctx.set_visuals(Visuals::light());
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(Margin::same(10.0)))
            .show(ctx, |ui| {
                self.top_bar(ui);
                ui.add_space(8.0);

                // ---- 主体：左导航 + 右内容 ----
                let full = ui.available_rect_before_wrap();
                let nav_w = 148.0;
                let nav_rect = egui::Rect::from_min_size(full.min, egui::vec2(nav_w, full.height() - 52.0));
                let mut nav_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(nav_rect)
                        .layout(egui::Layout::top_down(Align::Min)),
                );
                nav_ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                self.nav_ui(&mut nav_ui);

                // 导航与内容的分隔线
                ui.painter().vline(
                    nav_rect.right() + 6.0,
                    nav_rect.y_range(),
                    egui::Stroke::new(1.0, BORDER),
                );

                let content_rect = egui::Rect::from_min_max(
                    egui::pos2(nav_rect.right() + 14.0, full.top()),
                    egui::pos2(full.right(), full.bottom() - 52.0),
                );
                let mut content_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(content_rect)
                        .layout(egui::Layout::top_down(Align::Min)),
                );
                content_ui.set_clip_rect(content_rect);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(&mut content_ui, |ui| {
                        match self.active_section {
                            SEC_STYLE => self.section_style(ui),
                            SEC_DYNAMIC => self.section_dynamic(ui),
                            SEC_POSITION => self.section_position(ui),
                            SEC_GAMEPAD => self.section_gamepad(ui),
                            SEC_HOTKEY => self.section_hotkey(ui),
                            SEC_IMAGE => self.section_image(ui),
                            _ => self.section_presets(ui),
                        }
                        ui.add_space(6.0);
                    });

                ui.add_space(8.0);
                self.bottom_bar(ui);
            });

        // ---- 帧末：仅标记（推送模式：由「应用」按钮触发） ----
        if self.dirty {
            self.dirty = false;
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

/// 风格模板：借鉴 Crosshair X 的常见预设风格，一键套用参数。
const TEMPLATES: [(&str, fn(&mut Preset)); 8] = [
    ("tpl_apex", |p: &mut Preset| {
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
        p.shape = Shape::Gate;
        p.size = 16.0;
        p.thickness = 1.0;
        p.hollow.gap = 2.0;
        p.color = "#3DFF6E".into();
        p.multicolor = false;
        p.opacity = 0.9;
    }),
    ("tpl_cs2", |p: &mut Preset| {
        p.shape = Shape::Cross;
        p.size = 18.0;
        p.thickness = 1.5;
        p.hollow.gap = 0.0;
        p.color = "#00FF00".into();
        p.multicolor = false;
        p.opacity = 0.9;
    }),
    ("tpl_sniper", |p: &mut Preset| {
        p.shape = Shape::RingDot;
        p.size = 26.0;
        p.thickness = 2.0;
        p.color = "#FFFFFF".into();
        p.multicolor = false;
        p.opacity = 0.7;
        p.hollow.center_dot_size = 2.5;
    }),
    ("tpl_classic", |p: &mut Preset| {
        p.shape = Shape::Cross;
        p.size = 20.0;
        p.thickness = 2.0;
        p.hollow.gap = 0.0;
        p.color = "#FF0000".into();
        p.multicolor = false;
        p.opacity = 0.8;
    }),
    ("tpl_thick_gate", |p: &mut Preset| {
        p.shape = Shape::Gate;
        p.size = 24.0;
        p.thickness = 4.0;
        p.hollow.gap = 3.0;
        p.color = "#FFFFFF".into();
        p.multicolor = false;
        p.opacity = 1.0;
    }),
    ("tpl_dot", |p: &mut Preset| {
        p.shape = Shape::Dot;
        p.size = 6.0;
        p.color = "#FF2D2D".into();
        p.opacity = 0.9;
    }),
    ("tpl_cross_x", |p: &mut Preset| {
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
        ui.painter().rect_filled(rect, Rounding::same(6.0), Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8));
        ui.painter().rect_stroke(rect, Rounding::same(6.0), egui::Stroke::new(1.0, BORDER));
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