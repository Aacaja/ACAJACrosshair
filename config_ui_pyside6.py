#!/usr/bin/env python3
# -*- coding: utf-8 -*-

import os
import sys
import json
import glob
import ctypes
from ctypes import wintypes
from PySide6.QtWidgets import (
    QMainWindow, QWidget, QVBoxLayout, QHBoxLayout, QGridLayout,
    QLabel, QPushButton, QComboBox, QSlider, QLineEdit,
    QFrame, QGroupBox, QFileDialog, QMessageBox, QInputDialog,
    QApplication, QDialog, QKeySequenceEdit, QSystemTrayIcon, QCheckBox
)
from PySide6.QtCore import Qt, QTimer
from PySide6.QtGui import QFont, QColor, QKeySequence, QAction, QIcon, QPen, QPainter, QPixmap, QImage
from theme import detect_system_theme, get_effective_theme, generate_qss
from background_dialog import BackgroundCropDialog

# Windows API imports
try:
    import win32con
    import win32api
    import win32gui
except ImportError:
    pass

# Windows API constants
WM_HOTKEY = 0x0312
MOD_ALT = 0x0001
MOD_CONTROL = 0x0002
MOD_SHIFT = 0x0004
MOD_WIN = 0x0008

# 应用版本号（窗口标题等处统一引用）
APP_VERSION = "3.1.0"


def resource_path(relative_path):
    """解析资源文件的绝对路径。

    兼容两种运行方式：
    - 源码运行：以本文件所在目录为基准。
    - PyInstaller 打包（onefile）：以解包临时目录 sys._MEIPASS 为基准。
    """
    base_path = getattr(sys, "_MEIPASS", os.path.dirname(os.path.abspath(__file__)))
    return os.path.join(base_path, relative_path)

# 控件不透明度默认值（配置键缺失或无效时回退）
DEFAULT_CONTROL_OPACITY = 1.0


def _clamp01(value: float) -> float:
    """将浮点值钳制到 [0.0, 1.0]：小于 0.0 映射为 0.0，大于 1.0 映射为 1.0，范围内保持不变。"""
    if value < 0.0:
        return 0.0
    if value > 1.0:
        return 1.0
    return value


def _sanitize_control_opacity(value) -> float:
    """校验并归一化控件不透明度配置值。

    对非数字（含 None、字符串、bool、NaN）或越界（<0.0 或 >1.0）的输入，
    丢弃并回退到默认值 1.0；对范围内的有效数字则原样返回其 float。
    """
    # 排除 bool（bool 是 int 的子类，不应被当作有效不透明度）
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return DEFAULT_CONTROL_OPACITY
    numeric = float(value)
    # 排除 NaN 与无穷
    if numeric != numeric or numeric in (float("inf"), float("-inf")):
        return DEFAULT_CONTROL_OPACITY
    # 越界值视为无效，回退默认
    if numeric < 0.0 or numeric > 1.0:
        return DEFAULT_CONTROL_OPACITY
    return numeric


def _is_valid_control_opacity(value) -> bool:
    """判断控件不透明度存储值是否为有效的范围内数字（非 bool/非数字/NaN/无穷/越界均为无效）。"""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return False
    numeric = float(value)
    if numeric != numeric or numeric in (float("inf"), float("-inf")):
        return False
    return 0.0 <= numeric <= 1.0


class ConfigUI(QMainWindow):
    def __init__(self):
        super().__init__()
        self.overlay_window = None
        self.is_shown = False
        
        # 语言配置
        self.language = "zh"
        self.strings = {
            "zh": {
                "title": "小林の准星",
                "author": "Bilibili：林晓CCC",
                "config_management": "配置管理",
                "preset_config": "预设配置:",
                "config_location": "设置配置文件位置:",
                "new_preset": "新建预设",
                "load_preset": "加载预设",
                "save_preset": "保存预设",
                "delete_preset": "删除预设",
                "open_folder": "打开文件夹",
                "show_crosshair": "显示准星",
                "hide_crosshair": "隐藏准星",
                "crosshair_settings": "准星设置",
                "shape": "形状:",
                "shape_cross": "十字",
                "shape_dot": "圆点",
                "shape_square": "方块",
                "shape_circle": "圆形",
                "shape_hollow_cross": "空心十字",
                "shape_hollow_square": "空心方块",
                "shape_hollow_cross_dot": "空心十字加点",
                "shape_custom_image": "自定义图片",
                "size": "大小:",
                "thickness": "粗细:",
                "opacity": "透明度:",
                "color": "颜色:",
                "choose_color": "选择颜色",
                "position": "位置:",
                "center": "居中",
                "drag_mode": "拖动模式",
                "normal_mode": "正常模式",
                "hollow_cross_settings": "空心十字设置:",
                "center_dot_size": "中心点大小:",
                "hollow_gap": "中心距离:",
                "save_current": "保存当前配置",
                "language": "语言:",
                "invalid_address": "地址无效！",
                "save_failed": "保存失败，已保留上次成功保存的值。",
                "control_opacity_reset": "控件不透明度存储值无效，已重置为默认。",
                "warning": "警告",
                "success": "成功",
                "error": "错误",
                "confirm": "确认",
                "preset_exists": "预设 '{name}' 已存在！",
                "preset_created": "预设 '{name}' 创建成功！",
                "select_preset": "请先选择一个预设！",
                "preset_loaded": "已加载预设：{name}",
                "config_saved": "配置已保存到：{path}",
                "cannot_delete_default": "默认配置不能删除！",
                "delete_confirm": "确定要删除预设 '{name}' 吗？",
                "preset_deleted": "预设已删除",
                "delete_failed": "删除失败：{error}",
                "cannot_open_folder": "无法打开文件夹：{error}",
                "new_preset_name": "请输入预设名称：",
                "program_error": "程序运行出错：{error}",
                "hotkey": "快捷键:",
                "hotkey_hint": "点击设置快捷键",
                "hotkey_undefined": "未设置",
                "clear_hotkey": "清除",
                "enable_border": "启用描边",
                "border_settings": "描边设置...",
                "border_thickness": "描边厚度",
                "border_color": "描边颜色",
                "border_opacity": "描边透明度",
                "custom_image": "自定义图片",
                "select_image": "选择图片",
                "image_path": "图片路径:",
                "image_scale": "图片缩放:",
                "no_image": "未选择图片",
                "right_click_toggle": "右键切换准星",
                "right_click_warning": "启用后将拦截全局右键",
                "right_click_mode_click": "点击切换",
                "right_click_mode_hold_show": "按住显示",
                "right_click_mode_hold_hide": "按住隐藏",
                "minimize_to_tray": "关闭到系统托盘",
                "info_text": "1. 选择预设并自定义参数\n2. 点击显示准星\n3. 记得保存！\n4. 目前暂不支持全屏游戏使用，建议使用无边框",
                "auto_topmost_on_fullscreen": "全屏时自动置顶",
                "auto_topmost_hint": "检测到游戏全屏窗口时自动重置准星到最顶层",
                "hotkey_dialog_label": "请按下快捷键组合:",
                "ok": "确定",
                "cancel": "取消",
                "show_crosshair_first": "请先显示准星！",
                "tray_title": "准星程序",
                "tray_minimized_msg": "程序已最小化到系统托盘",
                "tray_toggle": "切换准星",
                "tray_open_settings": "打开设置",
                "tray_quit": "退出",
                "tray_tooltip_show": "准星程序 - 准星显示",
                "tray_tooltip_hide": "准星程序 - 准星隐藏",
                                "drag_hint": "拖动模式 - 拖动准星到想要的位置",
                "theme": "主题:",
                "theme_auto": "自动",
                "theme_light": "浅色",
                "theme_dark": "深色",
                "theme_custom": "自定义",
                "ui_background": "界面背景",
                "select_bg_image": "选择背景图片",
                "clear_bg": "清除背景",
                "bg_opacity": "背景不透明度:",
                "control_opacity": "控件不透明度:",
            },
            "en": {
                "title": "Roland's Crosshair",
                "author": "Bilibili: 林晓CCC",
                "config_management": "Configuration Management",
                "preset_config": "Preset Config:",
                "config_location": "Set Config Location:",
                "new_preset": "New Preset",
                "load_preset": "Load Preset",
                "save_preset": "Save Preset",
                "delete_preset": "Delete Preset",
                "open_folder": "Open Folder",
                "show_crosshair": "Show Crosshair",
                "hide_crosshair": "Hide Crosshair",
                "crosshair_settings": "Crosshair Settings",
                "shape": "Shape:",
                "shape_cross": "Cross",
                "shape_dot": "Dot",
                "shape_square": "Square",
                "shape_circle": "Circle",
                "shape_hollow_cross": "Hollow Cross",
                "shape_hollow_square": "Hollow Square",
                "shape_hollow_cross_dot": "Hollow Cross with Dot",
                "shape_custom_image": "Custom Image",
                "size": "Size:",
                "thickness": "Thickness:",
                "opacity": "Opacity:",
                "color": "Color:",
                "choose_color": "Choose Color",
                "position": "Position:",
                "center": "Center",
                "drag_mode": "Drag Mode",
                "normal_mode": "Normal Mode",
                "hollow_cross_settings": "Hollow Cross Settings:",
                "center_dot_size": "Center Dot Size:",
                "hollow_gap": "Center Gap:",
                "save_current": "Save Current Config",
                "language": "Language:",
                "invalid_address": "Invalid Address!",
                "save_failed": "Save failed. The last successfully saved value has been kept.",
                "control_opacity_reset": "Invalid stored control opacity was reset to default.",
                "warning": "Warning",
                "success": "Success",
                "error": "Error",
                "confirm": "Confirm",
                "preset_exists": "Preset '{name}' already exists!",
                "preset_created": "Preset '{name}' created successfully!",
                "select_preset": "Please select a preset first!",
                "preset_loaded": "Preset loaded: {name}",
                "config_saved": "Configuration saved to: {path}",
                "cannot_delete_default": "Default configuration cannot be deleted!",
                "delete_confirm": "Are you sure you want to delete preset '{name}'?",
                "preset_deleted": "Preset deleted",
                "delete_failed": "Delete failed: {error}",
                "cannot_open_folder": "Cannot open folder: {error}",
                "new_preset_name": "Please enter preset name:",
                "program_error": "Program error: {error}",
                "hotkey": "Hotkey:",
                "hotkey_hint": "Click to set hotkey",
                "hotkey_undefined": "Not Set",
                "clear_hotkey": "Clear",
                "enable_border": "Enable Border",
                "border_settings": "Border Settings...",
                "border_thickness": "Border Thickness",
                "border_color": "Border Color",
                "border_opacity": "Border Opacity",
                "custom_image": "Custom Image",
                "select_image": "Select Image",
                "image_path": "Image Path:",
                "image_scale": "Image Scale:",
                "no_image": "No Image Selected",
                "right_click_toggle": "Right click toggle crosshair",
                "right_click_warning": "Will intercept global right click after enabling",
                "right_click_mode_click": "Click to toggle",
                "right_click_mode_hold_show": "Hold to show",
                "right_click_mode_hold_hide": "Hold to hide",
                "minimize_to_tray": "Minimize to system tray",
                "info_text": "1. Select preset and customize parameters\n2. Click Show Crosshair\n3. Remember to save!\n4. Not support fullscreen games currently, suggest using borderless window",
                "auto_topmost_on_fullscreen": "Auto topmost on fullscreen",
                "auto_topmost_hint": "Automatically bring crosshair to top when a fullscreen window is detected",
                "hotkey_dialog_label": "Press the key combination:",
                "ok": "OK",
                "cancel": "Cancel",
                "show_crosshair_first": "Please show the crosshair first!",
                "tray_title": "Crosshair",
                "tray_minimized_msg": "The app has been minimized to the system tray",
                "tray_toggle": "Toggle Crosshair",
                "tray_open_settings": "Open Settings",
                "tray_quit": "Quit",
                "tray_tooltip_show": "Crosshair - visible",
                "tray_tooltip_hide": "Crosshair - hidden",
                                "drag_hint": "Drag mode - drag the crosshair to the desired position",
                "theme": "Theme:",
                "theme_auto": "Auto",
                "theme_light": "Light",
                "theme_dark": "Dark",
                "theme_custom": "Custom",
                "ui_background": "UI Background",
                "select_bg_image": "Select Background Image",
                "clear_bg": "Clear Background",
                "bg_opacity": "Background Opacity:",
                "control_opacity": "Control Opacity:",
            }
        }
        
        # 配置文件管理
        self.config_dir = os.path.join(os.environ['APPDATA'], 'CrosshairApp')
        os.makedirs(self.config_dir, exist_ok=True)
        self.app_state_path = os.path.join(self.config_dir, "app_state.json")
        app_state = self.load_app_state()

        # 恢复上次关闭程序时使用的预设；若该预设文件已不存在则回退到默认预设
        last_preset = app_state.get("last_preset", "default")
        if not os.path.exists(self.get_config_path(last_preset)):
            last_preset = "default"
        self.current_config_file = last_preset + ".json"
        self.config_file_path = self.get_config_path(last_preset)
        
        # 默认配置
        self.config = {
            "size": 20,
            "color": "#FF0000",
            "shape": "cross",
            "thickness": 2,
            "opacity": 0.8,
            "position": {"x": "center", "y": "center"},
            "hotkey": "",
            "minimize_to_tray": False,
            "right_click_shortcut": False,
            "right_click_mode": "click",
            "enable_border": False,
            "border_thickness": 1,
            "border_color": "#000000",
            "border_opacity": 1.0,
            "custom_image_path": "",
            "custom_image_scale": 1.0,
            "auto_topmost_on_fullscreen": False,
            "theme": "auto",
            "ui_bg_enabled": False,
            "ui_bg_image": "",
            "ui_bg_opacity": 0.15,
            "ui_control_opacity": 1.0
        }
        
        # 全局快捷键相关
        self.global_hotkey_id = 1
        self.hotkey_registered = False
        
        # 系统托盘相关
        self.tray_icon = None
        self.tray_menu = None
        
        self.load_config()

        # 恢复上次关闭程序时使用的语言（应用级状态优先，兼容旧版存于预设内的语言字段）
        self.language = app_state.get("language", self.config.get("language", "zh"))

        self.setup_ui()
        
        # 程序启动后默认显示准星
        QTimer.singleShot(500, self.show_crosshair)
        
    def t(self, key):
        """获取当前语言的字符串"""
        return self.strings[self.language].get(key, key)

    def _window_title(self):
        """根据当前语言返回窗口标题（含版本号）。"""
        brand = "Roland's Crosshair" if self.language == "en" else "小林の准星"
        return f"{brand} v{APP_VERSION}"

    def _app_icon(self):
        """加载应用图标（FAV 中的高分辨率图标），找不到时返回 None。"""
        for name in ("favicon_1024.png", "app.ico", "favicon.ico"):
            path = resource_path(os.path.join("FAV", name))
            if os.path.exists(path):
                return QIcon(path)
        return None
    
    def format_text(self, key, **kwargs):
        """格式化字符串"""
        text = self.t(key)
        for k, v in kwargs.items():
            text = text.replace(f"{{{k}}}", str(v))
        return text
    
    def get_config_path(self, config_name):
        """获取指定配置文件的完整路径"""
        if not config_name.endswith('.json'):
            return os.path.join(self.config_dir, config_name + '.json')
        else:
            return os.path.join(self.config_dir, config_name)
    
    def load_config(self):
        """加载配置文件"""
        try:
            if os.path.exists(self.config_file_path):
                with open(self.config_file_path, 'r', encoding='utf-8') as f:
                    self.config.update(json.load(f))
        except Exception as e:
            print(f"加载配置文件失败: {e}")
    
    def save_config(self):
        """保存配置文件。

        返回值：持久化成功返回 True，发生异常返回 False。
        （返回布尔值向后兼容，既有调用方均忽略返回值。）
        """
        try:
            with open(self.config_file_path, 'w', encoding='utf-8') as f:
                json.dump(self.config, f, indent=2, ensure_ascii=False)
            return True
        except Exception as e:
            print(f"保存配置文件失败: {e}")
            return False

    def load_app_state(self):
        """加载应用级状态（记忆的语言与最近使用的预设名），与具体预设文件无关。"""
        try:
            if os.path.exists(self.app_state_path):
                with open(self.app_state_path, 'r', encoding='utf-8') as f:
                    return json.load(f)
        except Exception as e:
            print(f"加载应用状态失败: {e}")
        return {}

    def save_app_state(self):
        """保存应用级状态：当前语言与当前预设名，供下次启动时恢复。"""
        try:
            preset_name = self.current_config_file[:-5] if self.current_config_file.endswith('.json') else self.current_config_file
            state = {"language": self.language, "last_preset": preset_name}
            with open(self.app_state_path, 'w', encoding='utf-8') as f:
                json.dump(state, f, indent=2, ensure_ascii=False)
        except Exception as e:
            print(f"保存应用状态失败: {e}")
    
    def get_available_presets(self):
        """获取可用的预设配置列表"""
        preset_files = glob.glob(os.path.join(self.config_dir, '*.json'))
        presets = []
        for file in preset_files:
            preset_name = os.path.basename(file)[:-5]
            presets.append(preset_name)
        return sorted(presets)
    
    def setup_ui(self):
        """设置用户界面"""
        self.setWindowTitle(self._window_title())
        # 设置窗口图标（使用 FAV 中的高分辨率图标）
        app_icon = self._app_icon()
        if app_icon is not None:
            self.setWindowIcon(app_icon)
        self.setGeometry(100, 100, 570, 900)
        self.setFixedSize(570, 900)
        
        # 设置字体
        self.setup_fonts()
        
        # 中央部件
        central_widget = ThemedBackgroundWidget(self)
        central_widget.setObjectName("centralWidget")
        self.setCentralWidget(central_widget)
        main_layout = QVBoxLayout(central_widget)
        main_layout.setSpacing(8)
        main_layout.setContentsMargins(8, 8, 8, 8)
        
        # 标题
        self.title_label = QLabel(self.t("title"))
        self.title_label.setAlignment(Qt.AlignCenter)
        self.title_label.setObjectName("titleLabel")
        main_layout.addWidget(self.title_label)
        
        # 作者信息
        self.author_label = QLabel(self.t("author"))
        self.author_label.setAlignment(Qt.AlignCenter)
        self.author_label.setObjectName("authorLabel")
        main_layout.addWidget(self.author_label)
        
        # 语言切换
        lang_layout = QHBoxLayout()
        self.lang_label = QLabel(self.t("language"))
        lang_layout.addWidget(self.lang_label)
        self.language_var = self.language
        self.language_combo = QComboBox()
        self.language_combo.addItems(["zh", "en"])
        self.language_combo.setCurrentText(self.language)
        self.language_combo.currentTextChanged.connect(self.change_language)
        lang_layout.addWidget(self.language_combo)
        lang_layout.addStretch()
        main_layout.addLayout(lang_layout)
        
        # 配置文件管理
        self.config_group = QGroupBox(self.t("config_management"))
        config_layout = QVBoxLayout(self.config_group)
        
        # 预设配置
        preset_layout = QHBoxLayout()
        self.preset_config_label = QLabel(self.t("preset_config"))
        preset_layout.addWidget(self.preset_config_label)
        self.preset_var = self.current_config_file.replace('.json', '')
        self.preset_combo = QComboBox()
        self.update_preset_list()
        self.preset_combo.currentTextChanged.connect(self.on_preset_selected)
        preset_layout.addWidget(self.preset_combo)
        config_layout.addLayout(preset_layout)
        
        # 配置文件路径
        path_layout = QHBoxLayout()
        self.config_location_label = QLabel(self.t("config_location"))
        path_layout.addWidget(self.config_location_label)
        self.config_path_var = self.config_file_path
        self.config_path_entry = QLineEdit(self.config_file_path)
        self.config_path_entry.textChanged.connect(self.validate_config_path)
        path_layout.addWidget(self.config_path_entry)
        config_layout.addLayout(path_layout)
        
        # 错误提示（默认不显示，不占空间）
        self.error_label = QLabel()
        self.error_label.setObjectName("errorLabel")
        self.error_label.setVisible(False)
        config_layout.addWidget(self.error_label)
        
        # 按钮
        button_layout = QHBoxLayout()
        self.new_preset_button = QPushButton(self.t("new_preset"))
        self.load_preset_button = QPushButton(self.t("load_preset"))
        self.save_preset_button = QPushButton(self.t("save_preset"))
        self.delete_preset_button = QPushButton(self.t("delete_preset"))
        self.open_folder_button = QPushButton(self.t("open_folder"))
        
        button_layout.addWidget(self.new_preset_button)
        button_layout.addWidget(self.load_preset_button)
        button_layout.addWidget(self.save_preset_button)
        button_layout.addWidget(self.delete_preset_button)
        button_layout.addWidget(self.open_folder_button)
        
        # 连接按钮信号
        self.new_preset_button.clicked.connect(self.create_preset)
        self.load_preset_button.clicked.connect(self.load_preset)
        self.save_preset_button.clicked.connect(self.save_preset)
        self.delete_preset_button.clicked.connect(self.delete_preset)
        self.open_folder_button.clicked.connect(self.open_config_folder)
        
        config_layout.addLayout(button_layout)
        main_layout.addWidget(self.config_group)
        
        # 控制按钮
        self.show_button = QPushButton(self.t("show_crosshair"))
        self.show_button.clicked.connect(self.toggle_crosshair)
        self.show_button.setObjectName("primaryButton")
        main_layout.addWidget(self.show_button)
        
        # 设置区域
        self.settings_group = QGroupBox(self.t("crosshair_settings"))
        settings_layout = QGridLayout(self.settings_group)
        # 列拉伸: label(0)固定, slider(1)优先拉伸, entry(2)固定
        settings_layout.setColumnStretch(0, 0)
        settings_layout.setColumnStretch(1, 1)
        settings_layout.setColumnStretch(2, 0)
        settings_layout.setHorizontalSpacing(8)
        
        # 形状选择
        self.shape_label = QLabel(self.t("shape"))
        settings_layout.addWidget(self.shape_label, 0, 0)
        self.shape_var = self.config["shape"]
        self.shape_combo = QComboBox()
        # 使用 itemData 存储实际形状值，显示文本为随语言变化的翻译
        self._populate_shape_combo()
        self._set_shape_combo_value(self.shape_var)
        # 以 currentIndexChanged 触发处理，避免因翻译文本变化而误触发
        self.shape_combo.currentIndexChanged.connect(self.on_shape_changed)
        settings_layout.addWidget(self.shape_combo, 0, 1, 1, 2)
        
        # 大小设置
        self.size_label = QLabel(self.t("size"))
        settings_layout.addWidget(self.size_label, 1, 0)
        self.size_var = float(self.config["size"])
        self.size_slider = QSlider(Qt.Horizontal)
        self.size_slider.setRange(1, 100)
        self.size_slider.setValue(int(self.size_var))
        self.size_slider.valueChanged.connect(self.update_size_label)
        self.size_entry = QLineEdit(str(self.size_var))
        self.size_entry.setFixedWidth(60)
        self.size_entry.editingFinished.connect(lambda: self._commit_slider_entry(self.size_entry, self.size_slider))
        settings_layout.addWidget(self.size_slider, 1, 1)
        settings_layout.addWidget(self.size_entry, 1, 2)
        
        # 粗细设置
        self.thickness_label = QLabel(self.t("thickness"))
        settings_layout.addWidget(self.thickness_label, 2, 0)
        self.thickness_var = float(self.config["thickness"])
        self.thickness_slider = QSlider(Qt.Horizontal)
        # 滑块内部按 0.1 的粒度存储（真实粗细 = 滑块值 / 10），以支持小数点粗细
        self.thickness_slider.setRange(10, 200)
        self.thickness_slider.setValue(round(self.thickness_var * 10))
        self.thickness_slider.valueChanged.connect(self.update_thickness_label)
        self.thickness_entry = QLineEdit(str(self.thickness_var))
        self.thickness_entry.setFixedWidth(60)
        self.thickness_entry.editingFinished.connect(lambda: self._commit_slider_entry(self.thickness_entry, self.thickness_slider, divisor=10))
        settings_layout.addWidget(self.thickness_slider, 2, 1)
        settings_layout.addWidget(self.thickness_entry, 2, 2)
        
        # 透明度设置
        self.opacity_label = QLabel(self.t("opacity"))
        settings_layout.addWidget(self.opacity_label, 3, 0)
        self.opacity_var = self.config["opacity"]
        self.opacity_slider = QSlider(Qt.Horizontal)
        self.opacity_slider.setRange(10, 100)
        self.opacity_slider.setValue(int(self.opacity_var * 100))
        self.opacity_slider.valueChanged.connect(self.update_opacity_label)
        self.opacity_entry = QLineEdit(str(self.opacity_var))
        self.opacity_entry.setFixedWidth(60)
        self.opacity_entry.editingFinished.connect(lambda: self._commit_slider_entry(self.opacity_entry, self.opacity_slider, divisor=100))
        settings_layout.addWidget(self.opacity_slider, 3, 1)
        settings_layout.addWidget(self.opacity_entry, 3, 2)
        
        # 颜色选择
        self.color_label = QLabel(self.t("color"))
        settings_layout.addWidget(self.color_label, 4, 0)
        self.color_button = QPushButton(self.t("choose_color"))
        self.color_button.setObjectName("colorButton")
        self.color_button.setFixedSize(100, 24)
        self._update_color_button_bg()
        self.color_button.clicked.connect(self.choose_color)
        # 颜色按钮放在entry列，与滑块对齐
        settings_layout.addWidget(self.color_button, 4, 2)
        
        # 描边设置（复选框跨2列，避免与按钮互相挤压）
        self.enable_border_checkbox = QCheckBox(self.t("enable_border"))
        self.enable_border_checkbox.setChecked(self.config.get("enable_border", False))
        self.enable_border_checkbox.stateChanged.connect(self.on_enable_border_changed)
        settings_layout.addWidget(self.enable_border_checkbox, 5, 0, 1, 2)
        
        # 描边设置按钮（限制宽度，防止撑宽entry列）
        self.border_settings_button = QPushButton(self.t("border_settings"))
        self.border_settings_button.clicked.connect(self.open_border_settings)
        self.border_settings_button.setVisible(self.config.get("enable_border", False))
        self.border_settings_button.setMaximumWidth(120)
        settings_layout.addWidget(self.border_settings_button, 5, 2)
        
        # 自定义图片设置
        self.custom_image_group = QGroupBox(self.t("custom_image"))
        custom_image_layout = QVBoxLayout(self.custom_image_group)
        
        # 图片路径选择
        image_path_layout = QHBoxLayout()
        self.image_path_label = QLabel(self.t("image_path"))
        image_path_layout.addWidget(self.image_path_label)
        self.image_path_entry = QLineEdit(self.config.get("custom_image_path", ""))
        self.image_path_entry.setReadOnly(True)
        image_path_layout.addWidget(self.image_path_entry)
        
        self.select_image_button = QPushButton(self.t("select_image"))
        self.select_image_button.clicked.connect(self.select_custom_image)
        image_path_layout.addWidget(self.select_image_button)
        custom_image_layout.addLayout(image_path_layout)
        
        # 图片缩放设置
        scale_layout = QHBoxLayout()
        self.image_scale_label = QLabel(self.t("image_scale"))
        scale_layout.addWidget(self.image_scale_label)
        
        self.image_scale_var = self.config.get("custom_image_scale", 1.0)
        self.image_scale_slider = QSlider(Qt.Horizontal)
        self.image_scale_slider.setRange(1, 300)
        self.image_scale_slider.setValue(int(self.image_scale_var * 100))
        self.image_scale_slider.valueChanged.connect(self.update_image_scale_label)
        self.image_scale_entry = QLineEdit(str(self.image_scale_var))
        self.image_scale_entry.setFixedWidth(60)
        self.image_scale_entry.editingFinished.connect(lambda: self._commit_slider_entry(self.image_scale_entry, self.image_scale_slider, divisor=100))
        scale_layout.addWidget(self.image_scale_slider)
        scale_layout.addWidget(self.image_scale_entry)
        custom_image_layout.addLayout(scale_layout)
        
        # 图片预览标签
        self.image_preview_label = QLabel()
        self.image_preview_label.setFixedSize(100, 100)
        self.image_preview_label.setStyleSheet("border: 1px solid #ccc; background-color: white;")
        self.image_preview_label.setAlignment(Qt.AlignCenter)
        self.image_preview_label.setText(self.t("no_image"))
        self.image_preview_label.setScaledContents(True)
        
        preview_layout = QHBoxLayout()
        preview_layout.addStretch()
        preview_layout.addWidget(self.image_preview_label)
        preview_layout.addStretch()
        custom_image_layout.addLayout(preview_layout)
        
        settings_layout.addWidget(self.custom_image_group, 6, 0, 1, 3)
        
        # 更新图片预览
        self.update_image_preview()
        
        # 空心十字专用设置
        self.hollow_cross_group = QGroupBox(self.t("hollow_cross_settings"))
        hollow_layout = QGridLayout(self.hollow_cross_group)
        # 列拉伸: label(0)固定, slider(1)优先拉伸, entry(2)固定
        hollow_layout.setColumnStretch(0, 0)
        hollow_layout.setColumnStretch(1, 1)
        hollow_layout.setColumnStretch(2, 0)
        hollow_layout.setHorizontalSpacing(8)
        
        # 中心距离设置
        self.hollow_gap_label = QLabel(self.t("hollow_gap"))
        hollow_layout.addWidget(self.hollow_gap_label, 0, 0)
        self.hollow_gap_var = self.config.get("hollow_gap", 0)  # 默认值改为0
        self.hollow_gap_slider = QSlider(Qt.Horizontal)
        self.hollow_gap_slider.setRange(0, 50)  # 改为最小值0
        self.hollow_gap_slider.setValue(int(self.hollow_gap_var))
        self.hollow_gap_slider.valueChanged.connect(self.update_hollow_gap_label)
        self.hollow_gap_entry = QLineEdit(str(self.hollow_gap_var))
        self.hollow_gap_entry.setFixedWidth(60)
        self.hollow_gap_entry.editingFinished.connect(lambda: self._commit_slider_entry(self.hollow_gap_entry, self.hollow_gap_slider))
        hollow_layout.addWidget(self.hollow_gap_slider, 0, 1)
        hollow_layout.addWidget(self.hollow_gap_entry, 0, 2)
        
        # 中心点大小设置（用于空心十字加点；直线长度/粗细已去除，改用上方的"大小"/"粗细"滑块）
        self.center_dot_size_label = QLabel(self.t("center_dot_size"))
        hollow_layout.addWidget(self.center_dot_size_label, 1, 0)
        self.center_dot_size_var = self.config.get("center_dot_size", 3)
        self.center_dot_size_slider = QSlider(Qt.Horizontal)
        self.center_dot_size_slider.setRange(1, 10)
        self.center_dot_size_slider.setValue(int(self.center_dot_size_var))
        self.center_dot_size_slider.valueChanged.connect(self.update_center_dot_size_label)
        self.center_dot_size_entry = QLineEdit(str(self.center_dot_size_var))
        self.center_dot_size_entry.setFixedWidth(60)
        self.center_dot_size_entry.editingFinished.connect(lambda: self._commit_slider_entry(self.center_dot_size_entry, self.center_dot_size_slider))
        hollow_layout.addWidget(self.center_dot_size_slider, 1, 1)
        hollow_layout.addWidget(self.center_dot_size_entry, 1, 2)
        
        settings_layout.addWidget(self.hollow_cross_group, 9, 0, 1, 3)
        
        # 快捷键设置
        hotkey_layout = QHBoxLayout()
        self.hotkey_label = QLabel(self.t("hotkey"))
        hotkey_layout.addWidget(self.hotkey_label)
        self.hotkey_button = QPushButton(self.config.get("hotkey", self.t("hotkey_undefined")))
        self.hotkey_button.clicked.connect(self.set_hotkey)
        self.hotkey_button.setToolTip(self.t("hotkey_hint"))
        self.hotkey_button.setFixedHeight(24)
        self.hotkey_button.setMaximumWidth(100)
        hotkey_layout.addWidget(self.hotkey_button)
        
        self.clear_hotkey_button = QPushButton(self.t("clear_hotkey"))
        self.clear_hotkey_button.clicked.connect(self.clear_hotkey)
        self.clear_hotkey_button.setFixedHeight(24)
        self.clear_hotkey_button.setMaximumWidth(80)
        hotkey_layout.addWidget(self.clear_hotkey_button)
        
        hotkey_layout.addStretch()
        settings_layout.addLayout(hotkey_layout, 10, 0, 1, 3)
        
        # 位置设置
        position_layout = QHBoxLayout()
        self.position_label = QLabel(self.t("position"))
        position_layout.addWidget(self.position_label)
        self.center_button = QPushButton(self.t("center"))
        self.center_button.clicked.connect(self.center_crosshair)
        position_layout.addWidget(self.center_button)
        
        self.drag_button = QPushButton(self.t("drag_mode"))
        self.drag_button.clicked.connect(self.toggle_drag_mode)
        self.drag_button.setObjectName("dragButton")
        position_layout.addWidget(self.drag_button)
        
        position_layout.addStretch()
        settings_layout.addLayout(position_layout, 11, 0, 1, 3)
        
        main_layout.addWidget(self.settings_group)
        
        # 右键快捷键单独一行（勾选框 + 模式下拉框，避免和下面一行挤在一起导致文字被压缩）
        right_click_layout = QHBoxLayout()
        self.right_click_checkbox = QCheckBox(self.t("right_click_toggle"))
        self.right_click_checkbox.setChecked(self.config.get("right_click_shortcut", False))
        self.right_click_checkbox.stateChanged.connect(self.on_right_click_changed)
        self.right_click_checkbox.setToolTip(self.t("right_click_warning"))
        right_click_layout.addWidget(self.right_click_checkbox)

        # 右键模式选择：点击切换 / 按住显示 / 按住隐藏
        self.right_click_mode_combo = QComboBox()
        self._populate_right_click_mode_combo()
        self._set_right_click_mode_combo_value(self.config.get("right_click_mode", "click"))
        self.right_click_mode_combo.currentIndexChanged.connect(self.on_right_click_mode_changed)
        self.right_click_mode_combo.setVisible(self.config.get("right_click_shortcut", False))
        right_click_layout.addWidget(self.right_click_mode_combo)

        right_click_layout.addStretch()
        main_layout.addLayout(right_click_layout)

        # 关闭到托盘 / 全屏置顶 两个选项同一行
        options_layout = QHBoxLayout()
        self.minimize_to_tray_checkbox = QCheckBox(self.t("minimize_to_tray"))
        self.minimize_to_tray_checkbox.setChecked(self.config.get("minimize_to_tray", False))
        self.minimize_to_tray_checkbox.stateChanged.connect(self.on_minimize_to_tray_changed)
        options_layout.addWidget(self.minimize_to_tray_checkbox)

        self.auto_topmost_checkbox = QCheckBox(self.t("auto_topmost_on_fullscreen"))
        self.auto_topmost_checkbox.setChecked(self.config.get("auto_topmost_on_fullscreen", False))
        self.auto_topmost_checkbox.setToolTip(self.t("auto_topmost_hint"))
        self.auto_topmost_checkbox.stateChanged.connect(self.on_auto_topmost_changed)
        options_layout.addWidget(self.auto_topmost_checkbox)

        options_layout.addStretch()
        main_layout.addLayout(options_layout)
        
        # 保留 warning_label 对象（语言切换需要），但不在主界面显示
        self.warning_label = QLabel(self.t("right_click_warning"))
        self.warning_label.setObjectName("warningLabel")
        self.warning_label.setVisible(False)
        
        # 初始状态设置控件可见性
        self.update_control_visibility()
        
        # 保存配置按钮
        self.save_button = QPushButton(self.t("save_current"))
        self.save_button.clicked.connect(self.save_settings)
        main_layout.addWidget(self.save_button)
        
        main_layout.addStretch()
        
        # 主题选择
        theme_layout = QHBoxLayout()
        self.theme_label = QLabel(self.t("theme"))
        theme_layout.addWidget(self.theme_label)
        self.theme_combo = QComboBox()
        self.theme_combo.addItems([self.t("theme_auto"), self.t("theme_light"), self.t("theme_dark"), self.t("theme_custom")])
        cur_theme = self.config.get("theme", "auto")
        theme_map = {"auto": 0, "light": 1, "dark": 2, "custom": 3}
        self.theme_combo.setCurrentIndex(theme_map.get(cur_theme, 0))
        self.theme_combo.currentIndexChanged.connect(self.on_theme_changed)
        theme_layout.addWidget(self.theme_combo)
        theme_layout.addStretch()
        main_layout.addLayout(theme_layout)

        # 界面背景（可展开/折叠）
        self.bg_toggle_button = QPushButton(self.t("ui_background") + " ▶")
        self.bg_toggle_button.setCheckable(True)
        self.bg_toggle_button.setObjectName("bgToggleButton")
        self.bg_toggle_button.toggled.connect(self.on_bg_toggle)
        main_layout.addWidget(self.bg_toggle_button)
        
        self.bg_group = QGroupBox(self.t("ui_background"))
        bg_layout = QGridLayout(self.bg_group)
        self.bg_opacity_label = QLabel(self.t("bg_opacity"))
        bg_layout.addWidget(self.bg_opacity_label, 0, 0)
        self.bg_opacity_slider = QSlider(Qt.Horizontal)
        self.bg_opacity_slider.setRange(0, 100)
        self.bg_opacity_slider.setValue(int(self.config.get("ui_bg_opacity", 0.15) * 100))
        self.bg_opacity_slider.valueChanged.connect(self.on_bg_opacity_changed)
        bg_layout.addWidget(self.bg_opacity_slider, 0, 1)
        # 控件不透明度滑块（紧跟背景不透明度滑块之后）
        self.control_opacity_label = QLabel(self.t("control_opacity"))
        bg_layout.addWidget(self.control_opacity_label, 1, 0)
        self.control_opacity_slider = QSlider(Qt.Horizontal)
        self.control_opacity_slider.setRange(0, 100)
        self.control_opacity_slider.setSingleStep(1)
        self.control_opacity_slider.setValue(int(self.config.get("ui_control_opacity", 1.0) * 100))
        self.control_opacity_slider.valueChanged.connect(self.on_control_opacity_changed)
        bg_layout.addWidget(self.control_opacity_slider, 1, 1)
        # 选择/清除按钮行相应下移到第 2 行
        self.select_bg_button = QPushButton(self.t("select_bg_image"))
        self.select_bg_button.clicked.connect(self.select_bg_image)
        bg_layout.addWidget(self.select_bg_button, 2, 0)
        self.clear_bg_button = QPushButton(self.t("clear_bg"))
        self.clear_bg_button.clicked.connect(self.clear_bg_image)
        bg_layout.addWidget(self.clear_bg_button, 2, 1)
        main_layout.addWidget(self.bg_group)
        self.bg_group.setVisible(False)  # 默认折叠

        # 应用主题
        self.apply_theme()

        # 初始化快捷键
        self.setup_hotkey_shortcut()
        
        # 如果配置启用，初始化系统托盘
        if self.config.get("minimize_to_tray", False):
            self.setup_system_tray()
        
    def setup_fonts(self):
        """设置字体"""
        if self.language == "zh":
            font = QFont("Microsoft YaHei", 9)
        else:
            font = QFont("Times New Roman", 9)
        
        self.setFont(font)
        
    def change_language(self, lang):
        """切换语言"""
        self.language = lang
        
        # 更新字体
        self.setup_fonts()
        
        # 保存语言配置（同时写入应用级状态，确保跨预设也能记住语言）
        self.config["language"] = lang
        self.save_config()
        self.save_app_state()

        # 刷新所有界面文本
        self.update_ui_from_config()
        
        # 刷新窗口标题
        self.setWindowTitle(self._window_title())
        
        # 如果系统托盘已存在,重建菜单以刷新文本
        if self.tray_icon is not None:
            self.tray_menu = self.create_tray_menu()
            self.tray_icon.setContextMenu(self.tray_menu)
            self.update_tray_tooltip()
        
    def validate_config_path(self, path):
        """验证配置文件路径"""
        if not path.strip():
            self.error_label.setText("")
            return
        
        if os.path.isdir(path):
            self.error_label.setText("")
            old_config_dir = self.config_dir
            self.config_dir = path
            
            if old_config_dir != self.config_dir:
                self.config_file_path = self.get_config_path(self.preset_var)
                self.update_config_from_ui()
                self.save_config()
                self.update_preset_list()
        else:
            self.error_label.setText(self.t("invalid_address"))
    
    def on_preset_selected(self, preset_name):
        """预设选择事件"""
        if preset_name:
            self.load_preset()
    
    def update_preset_list(self):
        """更新预设列表"""
        presets = self.get_available_presets()
        self.preset_combo.clear()
        self.preset_combo.addItems(presets)
        if self.preset_var in presets:
            self.preset_combo.setCurrentText(self.preset_var)
    
    def create_preset(self):
        """创建新的预设配置"""
        preset_name, ok = QInputDialog.getText(self, self.t("new_preset"), self.t("new_preset_name"))
        if ok and preset_name.strip():
            preset_name = preset_name.strip()
            if preset_name in self.get_available_presets():
                QMessageBox.warning(self, self.t("warning"), self.format_text("preset_exists", name=preset_name))
                return
            
            self.current_config_file = preset_name + '.json'
            self.config_file_path = self.get_config_path(preset_name)
            
            self.config = {
                "size": 20,
                "color": "#FF0000",
                "shape": "cross",
                "thickness": 2,
                "opacity": 0.8,
                "position": {"x": "center", "y": "center"}
            }
            
            self.save_config()
            self.update_ui_from_config()
            # 新预设默认无快捷键，重新注册以清除旧预设遗留的快捷键
            self.setup_hotkey_shortcut()
            self.save_app_state()
            self.update_preset_list()
            self.preset_combo.setCurrentText(preset_name)
            self.config_path_entry.setText(self.config_file_path)

            QMessageBox.information(self, self.t("success"), self.format_text("preset_created", name=preset_name))
    
    def load_preset(self):
        """加载预设配置"""
        preset_name = self.preset_combo.currentText()
        if not preset_name:
            QMessageBox.warning(self, self.t("warning"), self.t("select_preset"))
            return
        
        self.current_config_file = preset_name + '.json'
        self.config_file_path = self.get_config_path(preset_name)
        
        # 先加载配置文件内容
        try:
            config_path = self.get_config_path(preset_name)
            
            if os.path.exists(config_path):
                with open(config_path, 'r', encoding='utf-8') as f:
                    loaded_config = json.load(f)
            else:
                loaded_config = {}
        except Exception as e:
            QMessageBox.critical(self, self.t("error"), f"加载预设失败: {e}")
            return
        
        # 设置默认配置，然后用加载的配置覆盖
        self.config = {
            "size": 20,
            "color": "#FF0000",
            "shape": "cross",
            "thickness": 2,
            "opacity": 0.8,
            "position": {"x": "center", "y": "center"},
            "hollow_gap": 0,
            "center_dot_size": 3
        }
        
        # 用加载的配置覆盖默认配置
        self.config.update(loaded_config)
        
        # 更新UI
        self.update_ui_from_config()
        self.config_path_entry.setText(self.config_file_path)

        # 重新注册全局快捷键，确保切换预设后快捷键立即生效（而不必手动重新设置）
        self.setup_hotkey_shortcut()

        # 记录本次使用的预设，供下次启动时恢复
        self.save_app_state()

        # 更新准星显示
        if self.overlay_window:
            self.overlay_window.updateConfig(self.config)

        # 如果准星已显示，重新显示以应用新配置
        if self.is_shown:
            self.hide_crosshair()
            self.show_crosshair()

    def save_preset(self):
        """保存预设配置"""
        self.update_config_from_ui()
        
        # 确保保存准星位置信息
        if self.overlay_window and hasattr(self.overlay_window, 'get_crosshair_position'):
            pos = self.overlay_window.get_crosshair_position()
            # 只有在非居中位置时才保存具体坐标
            if pos[0] != QApplication.primaryScreen().size().width() // 2 or \
               pos[1] != QApplication.primaryScreen().size().height() // 2:
                self.config["position"] = {"x": pos[0], "y": pos[1]}
            else:
                self.config["position"] = {"x": "center", "y": "center"}
        
        self.save_config()
        QMessageBox.information(self, self.t("success"), self.format_text("config_saved", path=self.config_file_path))
    
    def delete_preset(self):
        """删除预设配置"""
        preset_name = self.preset_combo.currentText()
        if not preset_name:
            QMessageBox.warning(self, self.t("warning"), self.t("select_preset"))
            return
        
        if preset_name == 'default':
            QMessageBox.warning(self, self.t("warning"), self.t("cannot_delete_default"))
            return
        
        reply = QMessageBox.question(self, self.t("confirm"), self.format_text("delete_confirm", name=preset_name))
        if reply == QMessageBox.Yes:
            preset_file = self.get_config_path(preset_name)
            try:
                if os.path.exists(preset_file):
                    os.remove(preset_file)
                    QMessageBox.information(self, self.t("success"), self.t("preset_deleted"))
                    self.update_preset_list()
                    # setCurrentText 会触发 currentTextChanged -> on_preset_selected('default')，
                    # 从而自动加载 default 预设，无需在此重复调用
                    self.preset_combo.setCurrentText('default')
            except Exception as e:
                QMessageBox.critical(self, self.t("error"), self.format_text("delete_failed", error=str(e)))
    
    def open_config_folder(self):
        """打开配置文件夹"""
        try:
            os.startfile(self.config_dir)
        except Exception as e:
            QMessageBox.critical(self, self.t("error"), self.format_text("cannot_open_folder", error=str(e)))
    
    
    def _commit_slider_entry(self, entry, slider, divisor=1):
        """把输入框中手动填写的数值提交给对应滑块，让数值真正生效。

        divisor: 滑块整数值换算成输入框显示值的比例（透明度、图片缩放为100，其余为1）。
        非法输入、或数值超出滑块范围被钳制后与当前值相同（不会触发 valueChanged）时，
        都会把输入框重置为滑块当前实际生效的显示值，避免输入框显示和实际配置不一致。
        """
        text = entry.text().strip()
        try:
            display_value = float(text)
        except ValueError:
            entry.setText(str(slider.value() / divisor) if divisor != 1 else str(slider.value()))
            return
        raw_value = round(display_value * divisor)
        clamped = max(slider.minimum(), min(slider.maximum(), raw_value))
        if clamped == slider.value():
            entry.setText(str(clamped / divisor) if divisor != 1 else str(clamped))
        else:
            slider.setValue(clamped)  # 触发 valueChanged -> update_*_label -> update_crosshair()

    def update_size_label(self, value):
        """更新大小标签"""
        self.size_entry.setText(str(value))
        self.update_crosshair()
    
    def update_thickness_label(self, value):
        """更新粗细标签"""
        self.thickness_entry.setText(str(value / 10.0))
        self.update_crosshair()
    
    def update_opacity_label(self, value):
        """更新透明度标签"""
        self.opacity_entry.setText(str(value / 100.0))
        self.update_crosshair()
    
    def choose_color(self):
        """选择颜色"""
        from PySide6.QtWidgets import QColorDialog
        color = QColorDialog.getColor()
        if color.isValid():
            self.config["color"] = color.name()
            self.color_button.setStyleSheet(f"background-color: {color.name()};")
            self.update_crosshair()
    
    def toggle_crosshair(self):
        """切换准星显示/隐藏"""
        if not self.is_shown:
            self.show_crosshair()
        else:
            self.hide_crosshair()
    
    def show_crosshair(self):
        """显示准星"""
        is_new_overlay = self.overlay_window is None
        if is_new_overlay:
            from overlay_window_pyside6 import OverlayWindow
            self.overlay_window = OverlayWindow(self.config)

        # 检查窗口是否已经可见，避免重复操作和激活窗口
        if self.overlay_window.isVisible():
            self.overlay_window.updateConfig(self.config)
        else:
            # 确保窗口设置了 WA_ShowWithoutActivating 属性
            self.overlay_window.setAttribute(Qt.WA_ShowWithoutActivating)
            # 使用 show() 而不是 showFullScreen() 来避免激活窗口
            self.overlay_window.show()
            self.overlay_window.updateConfig(self.config)

        # 仅在 overlay 首次创建时（即程序刚启动）按配置注册右键钩子，确保重启后
        # 已保存的右键设置立即生效。之后不能再无条件重新注册：在"按住显示"模式下，
        # show_crosshair 本身就是按下时的回调，若每次调用都重新卸载/安装底层鼠标钩子，
        # 会在右键仍被按住期间触发系统级联回调，导致钩子反复重装并最终使程序崩溃。
        if is_new_overlay:
            if self.config.get("right_click_shortcut", False):
                self._apply_right_click_hook()
            else:
                self.overlay_window.unregister_global_mouse_hook()

        # 应用全屏自动置顶设置
        if hasattr(self.overlay_window, 'set_auto_topmost_on_fullscreen'):
            self.overlay_window.set_auto_topmost_on_fullscreen(self.config.get("auto_topmost_on_fullscreen", False))
        
        self.show_button.setText(self.t("hide_crosshair"))
        self.is_shown = True
    
    def hide_crosshair(self):
        """隐藏准星"""
        if self.overlay_window:
            self.overlay_window.hide()
        self.show_button.setText(self.t("show_crosshair"))
        self.is_shown = False
    
    def update_crosshair(self):
        """更新准星"""
        self.update_config_from_ui()
        if self.overlay_window and self.is_shown:
            self.overlay_window.updateConfig(self.config)
    
    def refresh_crosshair_display(self):
        """刷新准星显示"""
        self.update_crosshair()
    
    def center_crosshair(self):
        """将准星居中"""
        self.config["position"] = {"x": "center", "y": "center"}
        if self.overlay_window:
            self.overlay_window.center_crosshair()
            # 同时更新配置到UI
            self.update_config_from_ui()
            self.save_config()
    
    def toggle_drag_mode(self):
        """切换拖动模式"""
        if not self.is_shown:
            QMessageBox.warning(self, self.t("warning"), self.t("show_crosshair_first"))
            return
        
        if self.overlay_window:
            is_drag_mode = self.overlay_window.toggleDragMode()
            if is_drag_mode:
                self.drag_button.setText(self.t("normal_mode"))
                self.drag_button.setObjectName("dragButton")
            else:
                self.drag_button.setText(self.t("drag_mode"))
                self.drag_button.setObjectName("dragButton")
                # 退出拖动模式时保存位置
                if hasattr(self.overlay_window, 'get_crosshair_position'):
                    pos = self.overlay_window.get_crosshair_position()
                    self.config["position"] = {"x": pos[0], "y": pos[1]}
                    self.save_config()
    
    def save_settings(self):
        """保存设置"""
        self.update_config_from_ui()
        self.save_config()
        QMessageBox.information(self, self.t("success"), self.format_text("config_saved", path=self.config_file_path))
    
    # 形状值列表（顺序即下拉框显示顺序）；显示文本由语言字符串 shape_<value> 提供
    SHAPE_VALUES = ["cross", "dot", "square", "circle",
                    "hollow_cross", "hollow_square", "hollow_cross_dot", "custom_image"]

    def _populate_shape_combo(self):
        """按当前语言填充形状下拉框：显示翻译文本，itemData 存储实际形状值。"""
        for value in self.SHAPE_VALUES:
            self.shape_combo.addItem(self.t(f"shape_{value}"), value)

    def _refresh_shape_combo_texts(self):
        """语言切换时刷新形状下拉框各项的显示文本，保持选中值不变。"""
        for i in range(self.shape_combo.count()):
            value = self.shape_combo.itemData(i)
            self.shape_combo.setItemText(i, self.t(f"shape_{value}"))

    def _set_shape_combo_value(self, value):
        """根据实际形状值选中对应下拉项。"""
        index = self.shape_combo.findData(value)
        if index < 0:
            index = 0
        self.shape_combo.setCurrentIndex(index)

    def _get_shape_value(self):
        """获取当前选中的实际形状值（而非显示文本）。"""
        value = self.shape_combo.currentData()
        return value if value is not None else "cross"

    # 右键模式值列表（顺序即下拉框显示顺序）；显示文本由语言字符串 right_click_mode_<value> 提供
    RIGHT_CLICK_MODE_VALUES = ["click", "hold_show", "hold_hide"]

    def _populate_right_click_mode_combo(self):
        """按当前语言填充右键模式下拉框：显示翻译文本，itemData 存储实际模式值。"""
        for value in self.RIGHT_CLICK_MODE_VALUES:
            self.right_click_mode_combo.addItem(self.t(f"right_click_mode_{value}"), value)

    def _refresh_right_click_mode_combo_texts(self):
        """语言切换时刷新右键模式下拉框各项的显示文本，保持选中值不变。"""
        for i in range(self.right_click_mode_combo.count()):
            value = self.right_click_mode_combo.itemData(i)
            self.right_click_mode_combo.setItemText(i, self.t(f"right_click_mode_{value}"))

    def _set_right_click_mode_combo_value(self, value):
        """根据实际模式值选中对应下拉项。"""
        index = self.right_click_mode_combo.findData(value)
        if index < 0:
            index = 0
        self.right_click_mode_combo.setCurrentIndex(index)

    def _get_right_click_mode_value(self):
        """获取当前选中的实际右键模式值（而非显示文本）。"""
        value = self.right_click_mode_combo.currentData()
        return value if value is not None else "click"

    def _apply_right_click_hook(self):
        """根据当前右键模式，向 overlay 注册对应的全局鼠标钩子回调（自身具备幂等性，可安全重复调用）。"""
        if not self.overlay_window:
            return
        mode = self.config.get("right_click_mode", "click")
        if mode == "hold_show":
            self.overlay_window.register_global_mouse_hook(self.show_crosshair, self.hide_crosshair)
        elif mode == "hold_hide":
            self.overlay_window.register_global_mouse_hook(self.hide_crosshair, self.show_crosshair)
        else:
            self.overlay_window.register_global_mouse_hook(self.toggle_crosshair)

    def on_shape_changed(self, shape):
        """形状改变事件"""
        self.update_control_visibility()
        self.update_crosshair()
    
    def update_control_visibility(self):
        """更新控件可见性"""
        # 空心十字控件
        is_hollow_cross = self._get_shape_value() in ["hollow_cross", "hollow_cross_dot"]
        self.hollow_cross_group.setVisible(is_hollow_cross)
        
        # 中心点大小控件只在空心十字加点时显示
        is_hollow_cross_dot = self._get_shape_value() == "hollow_cross_dot"
        
        # 控制中心点大小相关控件的可见性（在hollow_layout的第1行）
        # 第1行包含：label(0), slider(1), entry(2)
        for col in [0, 1, 2]:
            item = self.hollow_cross_group.layout().itemAtPosition(1, col)
            if item:
                widget = item.widget()
                if widget:
                    widget.setVisible(is_hollow_cross_dot)
        
        # 自定义图片控件
        is_custom_image = self._get_shape_value() == "custom_image"
        self.custom_image_group.setVisible(is_custom_image)
    
    def update_center_dot_size_label(self, value):
        """更新中心点大小标签"""
        self.center_dot_size_entry.setText(str(value))
        self.update_crosshair()
    
    def update_hollow_gap_label(self, value):
        """更新空心十字中心距离标签"""
        self.hollow_gap_entry.setText(str(value))
        self.update_crosshair()
    
    def select_custom_image(self):
        """选择自定义图片"""
        file_path, _ = QFileDialog.getOpenFileName(
            self,
            self.t("select_image"),
            "",
            "Images (*.png *.jpg *.jpeg *.bmp);;All Files (*)"
        )
        
        if file_path:
            self.config["custom_image_path"] = file_path
            self.image_path_entry.setText(file_path)
            self.update_image_preview()
            self.update_crosshair()
    
    def update_image_scale_label(self, value):
        """更新图片缩放标签"""
        scale = value / 100.0
        self.image_scale_entry.setText(str(scale))
        self.update_image_preview()
        self.update_crosshair()
    
    def update_image_preview(self):
        """更新图片预览"""
        image_path = self.config.get("custom_image_path", "")
        if image_path and os.path.exists(image_path):
            try:
                pixmap = QPixmap(image_path)
                scale = self.image_scale_slider.value() / 100.0
                
                # 缩放图片
                scaled_pixmap = pixmap.scaled(
                    int(pixmap.width() * scale),
                    int(pixmap.height() * scale),
                    Qt.KeepAspectRatio,
                    Qt.SmoothTransformation
                )
                
                self.image_preview_label.setPixmap(scaled_pixmap)
                self.image_preview_label.setText("")
            except Exception as e:
                self.image_preview_label.setText("Error")
                print(f"加载图片预览失败: {e}")
        else:
            self.image_preview_label.setPixmap(QPixmap())
            self.image_preview_label.setText(self.t("no_image"))
    
    def on_enable_border_changed(self, state):
        """启用描边复选框改变"""
        self.config["enable_border"] = (state == Qt.CheckState.Checked.value)
        self.border_settings_button.setVisible(state == Qt.CheckState.Checked.value)
        self.update_crosshair()
    
    def open_border_settings(self):
        """打开描边设置对话框"""
        from border_settings_dialog import BorderSettingsDialog
        
        dialog = BorderSettingsDialog(self.config, self, self.language)
        result = dialog.exec()
        
        if result == QDialog.Accepted:
            self.update_crosshair()
    
    def set_hotkey(self):
        """设置快捷键"""
        from PySide6.QtWidgets import QKeySequenceEdit, QDialog, QVBoxLayout, QLabel, QPushButton
        
        # 创建自定义对话框
        dialog = QDialog(self)
        dialog.setWindowTitle(self.t("hotkey"))
        dialog.setModal(True)
        
        # 创建布局
        layout = QVBoxLayout(dialog)
        
        # 添加提示标签
        label = QLabel(self.t("hotkey_dialog_label"))
        layout.addWidget(label)
        
        # 添加快捷键编辑器
        key_edit = QKeySequenceEdit()
        key_edit.setKeySequence(QKeySequence(self.config.get("hotkey", "")))
        layout.addWidget(key_edit)
        
        # 添加确认按钮
        button_layout = QHBoxLayout()
        ok_button = QPushButton(self.t("ok"))
        cancel_button = QPushButton(self.t("cancel"))
        
        ok_button.clicked.connect(dialog.accept)
        cancel_button.clicked.connect(dialog.reject)
        
        button_layout.addStretch()
        button_layout.addWidget(ok_button)
        button_layout.addWidget(cancel_button)
        layout.addLayout(button_layout)
        
        # 显示对话框
        if dialog.exec() == QDialog.Accepted:
            key_sequence = key_edit.keySequence()
            self.config["hotkey"] = key_sequence.toString()
            self.hotkey_button.setText(key_sequence.toString() if key_sequence.toString() else self.t("hotkey_undefined"))
            self.setup_hotkey_shortcut()
            self.save_config()
    
    def clear_hotkey(self):
        """清除快捷键"""
        self.config["hotkey"] = ""
        self.hotkey_button.setText(self.t("hotkey_undefined"))
        self.unregister_global_hotkey()
        self.save_config()
    
    def setup_hotkey_shortcut(self):
        """设置全局快捷键（使用Windows API）"""
        try:
            # 先注销旧的快捷键
            self.unregister_global_hotkey()
            
            # 注册新的全局快捷键
            hotkey_str = self.config.get("hotkey", "")
            if hotkey_str:
                key_sequence = QKeySequence(hotkey_str)
                if not key_sequence.isEmpty():
                    # 解析快捷键
                    modifiers = 0
                    key_code = 0
                    
                    # 获取按键组合
                    key_combo = key_sequence[0]
                    
                    # 识别修饰键
                    if key_combo.keyboardModifiers() & Qt.KeyboardModifier.ControlModifier:
                        modifiers |= MOD_CONTROL
                    if key_combo.keyboardModifiers() & Qt.KeyboardModifier.AltModifier:
                        modifiers |= MOD_ALT
                    if key_combo.keyboardModifiers() & Qt.KeyboardModifier.ShiftModifier:
                        modifiers |= MOD_SHIFT
                    
                    # 获取虚拟键码
                    key = key_combo.key()
                    key_int = key & 0x01FFFFFF  # 移除修饰键位
                    
                    # 转换为Windows虚拟键码
                    if key_int >= Qt.Key.Key_A and key_int <= Qt.Key.Key_Z:
                        key_code = 0x41 + (key_int - Qt.Key.Key_A)
                    elif key_int >= Qt.Key.Key_0 and key_int <= Qt.Key.Key_9:
                        key_code = 0x30 + (key_int - Qt.Key.Key_0)
                    elif key_int == Qt.Key.Key_F1:
                        key_code = 0x70
                    elif key_int == Qt.Key.Key_F2:
                        key_code = 0x71
                    elif key_int == Qt.Key.Key_F3:
                        key_code = 0x72
                    elif key_int == Qt.Key.Key_F4:
                        key_code = 0x73
                    elif key_int == Qt.Key.Key_F5:
                        key_code = 0x74
                    elif key_int == Qt.Key.Key_F6:
                        key_code = 0x75
                    elif key_int == Qt.Key.Key_F7:
                        key_code = 0x76
                    elif key_int == Qt.Key.Key_F8:
                        key_code = 0x77
                    elif key_int == Qt.Key.Key_F9:
                        key_code = 0x78
                    elif key_int == Qt.Key.Key_F10:
                        key_code = 0x79
                    elif key_int == Qt.Key.Key_F11:
                        key_code = 0x7A
                    elif key_int == Qt.Key.Key_F12:
                        key_code = 0x7B
                    elif key_int == Qt.Key.Key_Space:
                        key_code = 0x20
                    elif key_int == Qt.Key.Key_Tab:
                        key_code = 0x09
                    elif key_int == Qt.Key.Key_Return:
                        key_code = 0x0D
                    elif key_int == Qt.Key.Key_Escape:
                        key_code = 0x1B
                    
                    # 注册全局快捷键
                    if key_code != 0:
                        hwnd = int(self.winId())
                        if ctypes.windll.user32.RegisterHotKey(hwnd, self.global_hotkey_id, modifiers, key_code):
                            self.hotkey_registered = True
                            print(f"全局快捷键已注册: {hotkey_str}")
                        else:
                            print(f"注册全局快捷键失败: {hotkey_str}")
        except Exception as e:
            print(f"设置全局快捷键出错: {e}")
    
    def unregister_global_hotkey(self):
        """注销全局快捷键"""
        try:
            if self.hotkey_registered:
                hwnd = int(self.winId())
                ctypes.windll.user32.UnregisterHotKey(hwnd, self.global_hotkey_id)
                self.hotkey_registered = False
        except Exception as e:
            print(f"注销全局快捷键出错: {e}")
    
    def nativeEvent(self, eventType, message):
        """处理原生Windows事件（用于全局快捷键）"""
        if eventType == "windows_generic_MSG":
            msg = ctypes.wintypes.MSG.from_address(message.__int__())
            if msg.message == WM_HOTKEY:
                # 触发快捷键
                self.toggle_crosshair()
                return True, 0
        return False, 0
    
    def setup_system_tray(self):
        """设置系统托盘"""
        try:
            # 创建托盘图标
            self.tray_icon = QSystemTrayIcon(self)

            # 优先使用 FAV 中的应用图标；找不到时回退为绘制的红色十字
            tray_icon = self._app_icon()
            if tray_icon is None:
                icon_pixmap = QPixmap(32, 32)
                icon_pixmap.fill(Qt.transparent)
                painter = QPainter(icon_pixmap)
                painter.setRenderHint(QPainter.Antialiasing)
                # 绘制一个十字作为图标
                painter.setPen(QPen(QColor("#FF0000"), 4))
                painter.drawLine(8, 16, 24, 16)  # 水平线
                painter.drawLine(16, 8, 16, 24)  # 垂直线
                painter.end()
                tray_icon = QIcon(icon_pixmap)
            self.tray_icon.setIcon(tray_icon)
            
            # 创建托盘菜单
            self.tray_menu = self.create_tray_menu()
            self.tray_icon.setContextMenu(self.tray_menu)
            
            # 双击托盘图标切换准星显示
            self.tray_icon.activated.connect(self.on_tray_icon_activated)
            
            # 设置工具提示
            self.update_tray_tooltip()
            
            # 显示托盘图标
            self.tray_icon.show()
            
            # 显示提示消息
            self.tray_icon.showMessage(
                self.t("tray_title"),
                self.t("tray_minimized_msg"),
                QSystemTrayIcon.MessageIcon.Information,
                3000
            )
            
        except Exception as e:
            print(f"设置系统托盘失败: {e}")
    
    def create_tray_menu(self):
        """创建托盘菜单"""
        from PySide6.QtWidgets import QMenu
        
        menu = QMenu(self)
        
        # 显示/隐藏准星
        toggle_action = QAction(self.t("tray_toggle"), self)
        toggle_action.triggered.connect(self.toggle_crosshair)
        menu.addAction(toggle_action)
        
        menu.addSeparator()
        
        # 打开设置
        show_action = QAction(self.t("tray_open_settings"), self)
        show_action.triggered.connect(self.show_main_window)
        menu.addAction(show_action)
        
        menu.addSeparator()
        
        # 退出
        quit_action = QAction(self.t("tray_quit"), self)
        quit_action.triggered.connect(self.quit_application)
        menu.addAction(quit_action)
        
        return menu
    
    def on_tray_icon_activated(self, reason):
        """托盘图标激活事件"""
        if reason == QSystemTrayIcon.ActivationReason.DoubleClick:
            # 双击切换准星显示
            self.toggle_crosshair()
    
    def on_right_click_changed(self, state):
        """右键快捷键选项改变"""
        self.config["right_click_shortcut"] = (state == Qt.CheckState.Checked.value)
        self.save_config()
        self.right_click_mode_combo.setVisible(self.config["right_click_shortcut"])

        # 注册或注销全局鼠标钩子
        if self.overlay_window:
            if self.config["right_click_shortcut"]:
                self._apply_right_click_hook()
            else:
                self.overlay_window.unregister_global_mouse_hook()

    def on_right_click_mode_changed(self, index):
        """右键模式改变（点击切换 / 按住显示 / 按住隐藏）"""
        self.config["right_click_mode"] = self._get_right_click_mode_value()
        self.save_config()
        # 若右键功能当前已启用，立即用新模式重新注册钩子
        if self.config.get("right_click_shortcut", False):
            self._apply_right_click_hook()

    def on_minimize_to_tray_changed(self, state):
        """关闭到托盘选项改变"""
        self.config["minimize_to_tray"] = (state == Qt.CheckState.Checked.value)
        self.save_config()
        
        # 根据设置初始化或移除系统托盘
        if self.config["minimize_to_tray"]:
            if self.tray_icon is None:
                self.setup_system_tray()
        else:
            if self.tray_icon is not None:
                self.tray_icon.hide()
                self.tray_icon = None
    
    def on_auto_topmost_changed(self, state):
        """全屏自动置顶选项改变"""
        self.config["auto_topmost_on_fullscreen"] = (state == Qt.CheckState.Checked.value)
        self.save_config()
        
        # 立即应用到覆盖层
        if self.overlay_window and hasattr(self.overlay_window, 'set_auto_topmost_on_fullscreen'):
            self.overlay_window.set_auto_topmost_on_fullscreen(self.config["auto_topmost_on_fullscreen"])
    
    def update_tray_tooltip(self):
        """更新托盘图标提示"""
        if self.tray_icon:
            key = "tray_tooltip_show" if self.is_shown else "tray_tooltip_hide"
            self.tray_icon.setToolTip(self.t(key))
    
    def show_main_window(self):
        """显示主窗口"""
        self.show()
        self.activateWindow()
    
    def quit_application(self):
        """退出应用程序"""
        self.save_app_state()
        self.unregister_global_hotkey()
        if self.overlay_window:
            self.overlay_window.unregister_global_mouse_hook()
            self.overlay_window.close()
        if self.tray_icon:
            self.tray_icon.hide()
        QApplication.quit()
    
    def closeEvent(self, event):
        """关闭事件"""
        # 如果启用了关闭到托盘，则最小化而不是关闭
        if self.config.get("minimize_to_tray", False) and self.tray_icon:
            event.ignore()
            self.hide()
        else:
            # 正常关闭
            self.save_app_state()
            self.unregister_global_hotkey()
            if self.overlay_window:
                self.overlay_window.close()
            if self.tray_icon:
                self.tray_icon.hide()
            event.accept()
    
    def update_config_from_ui(self):
        """从UI更新配置"""
        self.config["shape"] = self._get_shape_value()
        self.config["size"] = self.size_slider.value()
        self.config["thickness"] = self.thickness_slider.value() / 10.0
        self.config["opacity"] = self.opacity_slider.value() / 100.0
        
        # 保存自定义图片相关参数
        if self._get_shape_value() == "custom_image":
            self.config["custom_image_path"] = self.image_path_entry.text()
            self.config["custom_image_scale"] = self.image_scale_slider.value() / 100.0
        
        # 保存空心十字专用参数（直线长度/粗细已去除，统一使用上方的"大小"/"粗细"）
        if self._get_shape_value() in ["hollow_cross", "hollow_cross_dot"]:
            self.config["hollow_gap"] = self.hollow_gap_slider.value()
        
        # 保存中心点大小参数
        if self._get_shape_value() == "hollow_cross_dot":
            self.config["center_dot_size"] = self.center_dot_size_slider.value()
        
        # 保存描边参数
        self.config["enable_border"] = self.enable_border_checkbox.isChecked()
        # 描边参数在对话框中保存，这里不需要再次保存
        
        # 修复颜色解析
        style_sheet = self.color_button.styleSheet()
        if "background-color:" in style_sheet:
            self.config["color"] = style_sheet.split("background-color:")[1].split(";")[0].strip()
        else:
            self.config["color"] = "#FF0000"
    
    def update_ui_from_config(self):
        """从配置更新UI"""
        # 暂时断开信号连接，避免触发不必要的更新
        self.shape_combo.blockSignals(True)
        self.size_slider.blockSignals(True)
        self.thickness_slider.blockSignals(True)
        self.opacity_slider.blockSignals(True)
        self.hollow_gap_slider.blockSignals(True)
        self.center_dot_size_slider.blockSignals(True)
        self.enable_border_checkbox.blockSignals(True)
        self.language_combo.blockSignals(True)
        self.control_opacity_slider.blockSignals(True)
        self.right_click_mode_combo.blockSignals(True)
        
        try:
            # 更新静态标签和标题
            self.title_label.setText(self.t("title"))
            self.author_label.setText(self.t("author"))
            self.lang_label.setText(self.t("language"))
            
            # 更新 GroupBox 标题
            self.config_group.setTitle(self.t("config_management"))
            self.settings_group.setTitle(self.t("crosshair_settings"))
            self.custom_image_group.setTitle(self.t("custom_image"))
            self.hollow_cross_group.setTitle(self.t("hollow_cross_settings"))
            
            # 更新配置管理区域的标签
            self.preset_config_label.setText(self.t("preset_config"))
            self.config_location_label.setText(self.t("config_location"))
            
            # 更新按钮文本
            self.new_preset_button.setText(self.t("new_preset"))
            self.load_preset_button.setText(self.t("load_preset"))
            self.save_preset_button.setText(self.t("save_preset"))
            self.delete_preset_button.setText(self.t("delete_preset"))
            self.open_folder_button.setText(self.t("open_folder"))
            
            # 更新设置区域的所有标签
            self.shape_label.setText(self.t("shape"))
            self.size_label.setText(self.t("size"))
            self.thickness_label.setText(self.t("thickness"))
            self.opacity_label.setText(self.t("opacity"))
            self.color_label.setText(self.t("color"))
            self.enable_border_checkbox.setText(self.t("enable_border"))
            
            # 更新自定义图片区域标签
            self.image_path_label.setText(self.t("image_path"))
            self.image_scale_label.setText(self.t("image_scale"))
            
            # 更新其他按钮文本
            self.select_image_button.setText(self.t("select_image"))
            self.color_button.setText(self.t("choose_color"))
            self.hotkey_button.setToolTip(self.t("hotkey_hint"))
            self.clear_hotkey_button.setText(self.t("clear_hotkey"))
            
            # 更新图片预览标签
            if not self.config.get("custom_image_path", ""):
                self.image_preview_label.setText(self.t("no_image"))
            
            # 更新按钮和复选框文本
            self.right_click_checkbox.setText(self.t("right_click_toggle"))
            self.right_click_checkbox.setChecked(self.config.get("right_click_shortcut", False))
            self._refresh_right_click_mode_combo_texts()
            self._set_right_click_mode_combo_value(self.config.get("right_click_mode", "click"))
            self.right_click_mode_combo.setVisible(self.config.get("right_click_shortcut", False))
            self.minimize_to_tray_checkbox.setText(self.t("minimize_to_tray"))
            self.auto_topmost_checkbox.setText(self.t("auto_topmost_on_fullscreen"))
            self.auto_topmost_checkbox.setToolTip(self.t("auto_topmost_hint"))
            self.auto_topmost_checkbox.setChecked(self.config.get("auto_topmost_on_fullscreen", False))
            self.border_settings_button.setText(self.t("border_settings"))
            self.warning_label.setText(self.t("right_click_warning"))
            # 更新界面背景展开框文本
            arrow = "▼" if self.bg_toggle_button.isChecked() else "▶"
            self.bg_toggle_button.setText(f"{self.t('ui_background')} {arrow}")
            self.bg_group.setTitle(self.t("ui_background"))
            self.bg_opacity_label.setText(self.t("bg_opacity"))
            self.select_bg_button.setText(self.t("select_bg_image"))
            self.clear_bg_button.setText(self.t("clear_bg"))
            
            # 更新显示/隐藏按钮文本
            if self.is_shown:
                self.show_button.setText(self.t("hide_crosshair"))
            else:
                self.show_button.setText(self.t("show_crosshair"))
            
            # 更新位置按钮文本
            self.center_button.setText(self.t("center"))
            if hasattr(self, 'is_drag_mode') and self.is_drag_mode:
                self.drag_button.setText(self.t("normal_mode"))
            else:
                self.drag_button.setText(self.t("drag_mode"))
            
            # 更新形状：刷新各项翻译文本并按实际值选中
            self._refresh_shape_combo_texts()
            shape = self.config.get("shape", "cross")
            self._set_shape_combo_value(shape)
            
            # 更新大小
            size = int(self.config.get("size", 20))
            self.size_slider.setValue(size)
            self.size_entry.setText(str(size))
            
            # 更新粗细（支持小数点，滑块内部按 0.1 粒度存储）
            thickness = float(self.config.get("thickness", 2))
            self.thickness_slider.setValue(round(thickness * 10))
            self.thickness_entry.setText(str(thickness))
            
            # 更新透明度
            opacity = float(self.config.get("opacity", 0.8))
            self.opacity_slider.setValue(int(opacity * 100))
            self.opacity_entry.setText(str(opacity))
            
            # 更新颜色
            color = self.config.get("color", "#FF0000")
            self.color_button.setStyleSheet(f"background-color: {color};")
            
            # 更新描边设置
            self.enable_border_checkbox.setChecked(self.config.get("enable_border", False))
            self.border_settings_button.setVisible(self.config.get("enable_border", False))
            
            # 更新空心十字专用设置（直线长度/粗细已去除，统一使用上方的"大小"/"粗细"）
            hollow_gap = self.config.get("hollow_gap", 0)
            self.hollow_gap_slider.setValue(hollow_gap)
            self.hollow_gap_entry.setText(str(hollow_gap))
            self.hollow_gap_label.setText(self.t("hollow_gap"))

            # 更新中心点大小设置
            center_dot_size = self.config.get("center_dot_size", 3)
            self.center_dot_size_slider.setValue(center_dot_size)
            self.center_dot_size_entry.setText(str(center_dot_size))
            self.center_dot_size_label.setText(self.t("center_dot_size"))
            
            # 更新快捷键设置
            hotkey = self.config.get("hotkey", "")
            self.hotkey_label.setText(self.t("hotkey"))
            self.hotkey_button.setText(hotkey if hotkey else self.t("hotkey_undefined"))

            # 更新位置、保存按钮与主题相关文本
            self.position_label.setText(self.t("position"))
            self.save_button.setText(self.t("save_current"))
            self.theme_label.setText(self.t("theme"))
            # 主题下拉框各项文本随语言刷新（保持当前选中项不变）
            theme_item_keys = ["theme_auto", "theme_light", "theme_dark", "theme_custom"]
            for i, key in enumerate(theme_item_keys):
                if i < self.theme_combo.count():
                    self.theme_combo.setItemText(i, self.t(key))

            # 更新语言下拉框
            self.language_combo.setCurrentText(self.language)
            
            # 更新控件可见性
            self.update_control_visibility()
            
            # 更新自定义图片设置
            self.image_path_entry.setText(self.config.get("custom_image_path", ""))
            image_scale = self.config.get("custom_image_scale", 1.0)
            self.image_scale_slider.setValue(int(image_scale * 100))
            self.image_scale_entry.setText(str(image_scale))
            self.update_image_preview()

            # 更新控件不透明度滑块与标签
            self.control_opacity_label.setText(self.t("control_opacity"))
            if "ui_control_opacity" in self.config:
                raw_control_opacity = self.config["ui_control_opacity"]
                control_opacity = _sanitize_control_opacity(raw_control_opacity)
                # 存储值无效（非数字/NaN/越界）时回退默认并提示"无效值已重置"
                if not _is_valid_control_opacity(raw_control_opacity):
                    self.config["ui_control_opacity"] = control_opacity
                    self.error_label.setText(self.t("control_opacity_reset"))
                    self.error_label.setVisible(True)
            else:
                # 配置缺失时使用默认值（不视为无效，无需提示）
                control_opacity = DEFAULT_CONTROL_OPACITY
            self.control_opacity_slider.setValue(int(control_opacity * 100))
            
        finally:
            # 重新连接信号
            self.shape_combo.blockSignals(False)
            self.size_slider.blockSignals(False)
            self.thickness_slider.blockSignals(False)
            self.opacity_slider.blockSignals(False)
            self.hollow_gap_slider.blockSignals(False)
            self.center_dot_size_slider.blockSignals(False)
            self.enable_border_checkbox.blockSignals(False)
            self.language_combo.blockSignals(False)
            self.control_opacity_slider.blockSignals(False)
            self.right_click_mode_combo.blockSignals(False)

    def apply_theme(self):
        """应用当前主题到整个界面。"""
        effective = get_effective_theme(self.config)
        # 从 config 读取控件不透明度并钳制后传入 QSS 生成器
        control_opacity = _sanitize_control_opacity(
            self.config.get("ui_control_opacity", DEFAULT_CONTROL_OPACITY)
        )
        qss = generate_qss(effective, control_opacity=control_opacity)
        # 应用到中央部件及其子部件
        central = self.centralWidget()
        if central is not None:
            central.setStyleSheet(qss)
        # 同步背景图给 central widget
        if hasattr(central, 'set_background'):
            central.set_background(
                self.config.get("ui_bg_enabled", False),
                self.config.get("ui_bg_image", ""),
                self.config.get("ui_bg_opacity", 0.15)
            )

    def on_theme_changed(self, index):
        """主题下拉框改变。"""
        themes = ["auto", "light", "dark", "custom"]
        self.config["theme"] = themes[index] if 0 <= index < len(themes) else "auto"
        self.save_config()
        # 选择"自定义"时：展开界面背景区并启用背景图
        if self.config["theme"] == "custom":
            self.bg_toggle_button.setChecked(True)
            self.config["ui_bg_enabled"] = True
            self.save_config()
            # 如果还没有背景图，提示用户选择
            if not self.config.get("ui_bg_image") or not os.path.exists(self.config["ui_bg_image"]):
                QMessageBox.information(self, self.t("ui_background"), self.t("select_bg_image"))
        self.apply_theme()

    def on_bg_toggle(self, checked):
        """界面背景展开/折叠。"""
        self.bg_group.setVisible(checked)
        arrow = "▼" if checked else "▶"
        self.bg_toggle_button.setText(f"{self.t('ui_background')} {arrow}")

    def on_bg_opacity_changed(self, value):
        """背景不透明度改变。"""
        self.config["ui_bg_opacity"] = value / 100.0
        self.save_config()
        self.apply_theme()

    def on_control_opacity_changed(self, value):
        """控件不透明度改变：value/100 → 钳制 [0,1] → 写入 config → 持久化 → 重新应用主题。

        持久化失败时（Req 2.5）：保留上次成功持久化的值，通过 error_label 显示保存失败提示，
        不弹模态框、不使 UI 崩溃。
        """
        # 记录上次成功持久化的值，便于持久化失败时回滚
        previous_value = self.config.get("ui_control_opacity", DEFAULT_CONTROL_OPACITY)
        new_value = _clamp01(value / 100.0)
        self.config["ui_control_opacity"] = new_value

        if self.save_config():
            # 持久化成功：清除历史错误提示并应用主题
            self.error_label.setText("")
            self.error_label.setVisible(False)
            self.apply_theme()
        else:
            # 持久化失败：回滚为上次成功持久化的值，并显示非阻塞错误提示
            self.config["ui_control_opacity"] = previous_value
            self.error_label.setText(self.t("save_failed"))
            self.error_label.setVisible(True)

    def select_bg_image(self):
        """选择背景图片,弹出裁剪对话框。"""
        from PySide6.QtWidgets import QFileDialog
        file_path, _ = QFileDialog.getOpenFileName(
            self, self.t("select_bg_image"), "",
            "Images (*.png *.jpg *.jpeg *.bmp);;All Files (*)"
        )
        if not file_path:
            return
        pixmap = QPixmap(file_path)
        if pixmap.isNull():
            return
        dlg = BackgroundCropDialog(pixmap, self, self.language)
        if dlg.exec() == QDialog.Accepted:
            cropped = dlg.get_cropped_pixmap()
            if cropped is not None and not cropped.isNull():
                # 保存到 APPDATA
                bg_path = os.path.join(self.config_dir, "bg.png")
                cropped.save(bg_path, "PNG")
                self.config["ui_bg_image"] = bg_path
                self.config["ui_bg_enabled"] = True
                self.save_config()
                self.apply_theme()

    def clear_bg_image(self):
        """清除背景图片。"""
        self.config["ui_bg_enabled"] = False
        self.save_config()
        self.apply_theme()

    def _update_color_button_bg(self):
        """更新颜色按钮的背景色(保留主题边框)。"""
        c = self.config.get('color', '#FF0000')
        self.color_button.setText(c)
        # 仅设置背景色,QSS 提供 border
        self.color_button.setStyleSheet(
            f"QPushButton#colorButton {{ background-color: {c}; }}"
        )



class ThemedBackgroundWidget(QWidget):
    """带背景图片绘制能力的中央部件。"""

    def __init__(self, main_window=None):
        super().__init__()
        self.main_window = main_window
        self._bg_enabled = False
        self._bg_image_path = ""
        self._bg_opacity = 0.15
        self._bg_pixmap = None
        # 让 QWidget 绘制 QSS 样式表中的背景色（否则只有子控件变色）
        self.setAttribute(Qt.WA_StyledBackground, True)

    def set_background(self, enabled, image_path, opacity):
        """设置背景图。"""
        self._bg_enabled = enabled
        self._bg_image_path = image_path
        self._bg_opacity = opacity
        self._bg_pixmap = None
        if enabled and image_path and os.path.exists(image_path):
            self._bg_pixmap = QPixmap(image_path)
        self.update()

    def paintEvent(self, event):
        """先画背景图(带透明度),再交给父类画控件。"""
        from PySide6.QtGui import QPainter
        if self._bg_enabled and self._bg_pixmap is not None and not self._bg_pixmap.isNull():
            painter = QPainter(self)
            painter.setOpacity(self._bg_opacity)
            # 拉伸铺满
            scaled = self._bg_pixmap.scaled(
                self.width(), self.height(),
                Qt.KeepAspectRatioByExpanding,
                Qt.SmoothTransformation
            )
            x = (self.width() - scaled.width()) // 2
            y = (self.height() - scaled.height()) // 2
            painter.drawPixmap(x, y, scaled)
            painter.end()
        super().paintEvent(event)
