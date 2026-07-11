#!/usr/bin/env python3
# -*- coding: utf-8 -*-

"""
主题系统:系统主题检测 + 浅色/深色 QSS 样式生成
"""

import ctypes
from PySide6.QtGui import QGuiApplication


def detect_system_theme():
    """检测 Windows 系统当前是浅色还是深色主题。

    优先用 Qt 6.5+ 的 styleHints().colorScheme(),
    失败则读注册表 HKCU\\...\\AppsUseLightTheme。

    返回 'light' 或 'dark'。
    """
    # 方式1:Qt 6.5+ (最简洁)
    try:
        sh = QGuiApplication.styleHints()
        if hasattr(sh, "colorScheme"):
            # Qt.ColorScheme.Dark = 2, Light = 1
            scheme = sh.colorScheme()
            # 用字符串比较避免版本差异
            if str(scheme).endswith("Dark") or int(getattr(scheme, "value", 0)) == 2:
                return "dark"
            return "light"
    except Exception:
        pass

    # 方式2:读注册表
    try:
        HKCU = 0x80000001
        KEY_READ = 0x20019
        HKEY_CURRENT_USER = ctypes.wintypes.HKEY()
        subkey = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"
        advapi32 = ctypes.windll.advapi32

        ret = advapi32.RegOpenKeyExW(HKCU, subkey, 0, KEY_READ, ctypes.byref(HKEY_CURRENT_USER))
        if ret == 0:
            value = ctypes.wintypes.DWORD()
            size = ctypes.wintypes.DWORD(ctypes.sizeof(value))
            ret = advapi32.RegQueryValueExW(HKEY_CURRENT_USER, "AppsUseLightTheme", None,
                                            None, ctypes.byref(value), ctypes.byref(size))
            advapi32.RegCloseKey(HKEY_CURRENT_USER)
            if ret == 0:
                # 0 = 深色, 1 = 浅色
                return "dark" if value.value == 0 else "light"
    except Exception:
        pass

    # 默认浅色
    return "light"


def get_effective_theme(config):
    """根据配置返回实际生效的主题。

    config["theme"] 可以是 'auto'/'light'/'dark'/'custom'。
    auto 时调用 detect_system_theme()。
    custom 时使用 detect_system_theme() 作为基础配色（再叠加自定义背景图）。
    """
    t = config.get("theme", "auto")
    if t == "auto":
        return detect_system_theme()
    if t == "custom":
        return detect_system_theme()
    return t


# ---------------- QSS 生成 ----------------

def _palette(theme):
    """返回主题配色字典。"""
    if theme == "dark":
        return {
            "bg": "#1e1e1e",          # 主背景
            "card": "#2d2d2d",        # 卡片/分组背景
            "card_alt": "#383838",    # 卡片悬停
            "text": "#e5e5e7",        # 主文字
            "text_dim": "#9a9a9e",    # 次要文字
            "accent": "#0a84ff",      # 强调色
            "accent_hover": "#3a9eff",
            "border": "#3a3a3c",
            "input_bg": "#3a3a3c",
            "danger": "#ff453a",
            "success": "#30d158",
            "warning": "#ffd60a",
        }
    # light
    return {
        "bg": "#f5f5f7",
        "card": "#ffffff",
        "card_alt": "#ececee",
        "text": "#1d1d1f",
        "text_dim": "#6e6e73",
        "accent": "#007aff",
        "accent_hover": "#0066d6",
        "border": "#d2d2d7",
        "input_bg": "#ffffff",
        "danger": "#ff3b30",
        "success": "#34c759",
        "warning": "#ff9500",
    }


def _hex_to_rgba(hex_color, alpha):
    """将 '#RRGGBB' 十六进制颜色与 alpha 组合为 'rgba(r, g, b, a)' 字符串。

    alpha 先钳制到 [0.0, 1.0]。
    """
    # 钳制 alpha 到 [0.0, 1.0]
    a = max(0.0, min(1.0, alpha))
    # 去除前导 '#' 并解析 RGB 分量
    h = hex_color.lstrip("#")
    r = int(h[0:2], 16)
    g = int(h[2:4], 16)
    b = int(h[4:6], 16)
    return f"rgba({r}, {g}, {b}, {a})"


def generate_qss(theme, control_opacity=1.0):
    """根据主题与控件不透明度生成完整 QSS 样式表字符串。

    theme: 'light' 或 'dark'
    control_opacity: 0.0-1.0，越界自动钳制；默认 1.0（完全不透明）。
    受影响控件(QGroupBox、QLineEdit、QComboBox、QSpinBox、QPushButton)的
    background-color 使用 rgba(...)，alpha = 钳制后的 control_opacity；文字 color 保持不变。
    """
    p = _palette(theme)

    # 入口处钳制不透明度
    alpha = max(0.0, min(1.0, control_opacity))
    # 预计算各 Styled_Control 背景色的 rgba 表达
    card_rgba = _hex_to_rgba(p['card'], alpha)
    card_alt_rgba = _hex_to_rgba(p['card_alt'], alpha)
    input_bg_rgba = _hex_to_rgba(p['input_bg'], alpha)

    qss = f"""
    /* ===== 全局 ===== */
    QWidget#centralWidget {{
        background-color: {p['bg']};
        color: {p['text']};
    }}
    QWidget {{
        color: {p['text']};
        font-family: "Microsoft YaHei", "Segoe UI", sans-serif;
    }}
    QLabel {{
        color: {p['text']};
        background: transparent;
    }}
    QLabel#titleLabel {{
        font-size: 18px;
        font-weight: bold;
        color: {p['accent']};
        padding: 2px;
    }}
    QLabel#authorLabel {{
        font-size: 10px;
        color: {p['text_dim']};
    }}
    QLabel#dimLabel {{
        color: {p['text_dim']};
        font-size: 9px;
    }}
    QLabel#warningLabel {{
        color: {p['warning']};
        font-size: 9px;
    }}
    QLabel#errorLabel {{
        color: {p['danger']};
        font-size: 9px;
    }}

    /* ===== 分组卡片 ===== */
    QGroupBox {{
        background-color: {card_rgba};
        border: 1px solid {p['border']};
        border-radius: 8px;
        margin-top: 14px;
        padding: 14px 10px 10px 10px;
        font-weight: bold;
        color: {p['text']};
    }}
    QGroupBox::title {{
        subcontrol-origin: margin;
        subcontrol-position: top left;
        left: 12px;
        padding: 0 6px;
        background-color: {card_rgba};
        color: {p['accent']};
    }}

    /* ===== 按钮 ===== */
    QPushButton {{
        background-color: {card_alt_rgba};
        color: {p['text']};
        border: 1px solid {p['border']};
        border-radius: 6px;
        padding: 4px 10px;
        min-height: 18px;
    }}
    QPushButton:hover {{
        background-color: {p['border']};
        border-color: {p['accent']};
    }}
    QPushButton:pressed {{
        background-color: {p['accent']};
        color: white;
    }}
    QPushButton#primaryButton {{
        background-color: {p['accent']};
        color: white;
        border: none;
        font-size: 14px;
        font-weight: bold;
        padding: 10px;
        border-radius: 8px;
    }}
    QPushButton#primaryButton:hover {{
        background-color: {p['accent_hover']};
    }}
    QPushButton#primaryButton:pressed {{
        background-color: {p['accent']};
    }}
    QPushButton#dragButton {{
        background-color: {p['danger']};
        color: white;
        border: none;
        border-radius: 6px;
        font-weight: bold;
    }}
    QPushButton#dragButton:hover {{
        background-color: #ff6b6b;
    }}
    QPushButton#colorButton {{
        border: 2px solid {p['border']};
        border-radius: 6px;
        padding: 5px 8px;
    }}

    /* ===== 输入控件 ===== */
    QLineEdit, QComboBox, QSpinBox {{
        background-color: {input_bg_rgba};
        color: {p['text']};
        border: 1px solid {p['border']};
        border-radius: 5px;
        padding: 3px 6px;
        min-height: 18px;
    }}
    QLineEdit:focus, QComboBox:focus {{
        border: 1px solid {p['accent']};
    }}
    QComboBox::drop-down {{
        border: none;
        width: 22px;
    }}
    QComboBox QAbstractItemView {{
        background-color: {p['card']};
        color: {p['text']};
        selection-background-color: {p['accent']};
        selection-color: white;
        border: 1px solid {p['border']};
        border-radius: 4px;
        outline: none;
    }}

    /* ===== 滑块 ===== */
    QSlider::groove:horizontal {{
        height: 6px;
        background: {p['border']};
        border-radius: 3px;
    }}
    QSlider::sub-page:horizontal {{
        background: {p['accent']};
        border-radius: 3px;
    }}
    QSlider::handle:horizontal {{
        background: {p['card']};
        border: 2px solid {p['accent']};
        width: 14px;
        height: 14px;
        margin: -5px 0;
        border-radius: 8px;
    }}
    QSlider::handle:horizontal:hover {{
        background: {p['accent']};
    }}

    /* ===== 复选框 ===== */
    QCheckBox {{
        color: {p['text']};
        spacing: 6px;
        background: transparent;
    }}
    QCheckBox::indicator {{
        width: 16px;
        height: 16px;
        border: 2px solid {p['border']};
        border-radius: 4px;
        background: {p['input_bg']};
    }}
    QCheckBox::indicator:hover {{
        border-color: {p['accent']};
    }}
    QCheckBox::indicator:checked {{
        background: {p['accent']};
        border-color: {p['accent']};
        image: none;
    }}

    /* ===== 对话框 ===== */
    QDialog {{
        background-color: {p['bg']};
    }}

    /* ===== 滚动条 ===== */
    QScrollBar:vertical {{
        background: transparent;
        width: 10px;
        margin: 0;
    }}
    QScrollBar::handle:vertical {{
        background: {p['border']};
        min-height: 30px;
        border-radius: 5px;
    }}
    QScrollBar::handle:vertical:hover {{
        background: {p['text_dim']};
    }}
    QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {{
        height: 0;
    }}
    QScrollBar::add-page:vertical, QScrollBar::sub-page:vertical {{
        background: transparent;
    }}

    /* ===== 菜单(托盘) ===== */
    QMenu {{
        background-color: {p['card']};
        color: {p['text']};
        border: 1px solid {p['border']};
        border-radius: 6px;
        padding: 4px;
    }}
    QMenu::item {{
        padding: 6px 24px;
        border-radius: 4px;
    }}
    QMenu::item:selected {{
        background-color: {p['accent']};
        color: white;
    }}
    QMenu::separator {{
        height: 1px;
        background: {p['border']};
        margin: 4px 8px;
    }}

    /* ===== Tooltip ===== */
    QToolTip {{
        background-color: {p['card']};
        color: {p['text']};
        border: 1px solid {p['border']};
        border-radius: 4px;
        padding: 4px;
    }}
    """
    return qss