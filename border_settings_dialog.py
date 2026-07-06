#!/usr/bin/env python3
# -*- coding: utf-8 -*-

from PySide6.QtWidgets import QDialog, QVBoxLayout, QHBoxLayout, QGridLayout, QLabel, QPushButton, QSlider, QLineEdit, QGroupBox, QColorDialog
from PySide6.QtCore import Qt


class BorderSettingsDialog(QDialog):
    def __init__(self, config, parent=None, language="zh"):
        super().__init__(parent)
        self.config = config
        self.language = language
        
        self.setWindowTitle(self.t("border_settings_title"))
        self.setModal(True)
        self.setFixedSize(350, 200)
        
        self.setup_ui()
    
    def t(self, key):
        """获取翻译字符串"""
        translations = {
            "border_settings_title": {"zh": "描边设置", "en": "Border Settings"},
            "border_thickness": {"zh": "描边厚度", "en": "Border Thickness"},
            "border_color": {"zh": "描边颜色", "en": "Border Color"},
            "border_opacity": {"zh": "描边透明度", "en": "Border Opacity"},
            "choose_color": {"zh": "选择颜色", "en": "Choose Color"},
            "ok": {"zh": "确定", "en": "OK"},
            "cancel": {"zh": "取消", "en": "Cancel"}
        }
        
        # 使用传入的语言设置（默认中文），跟随主界面语言
        return translations.get(key, {}).get(self.language, translations.get(key, {}).get("zh", key))
    
    def setup_ui(self):
        """设置UI"""
        layout = QVBoxLayout(self)
        
        # 描边设置组
        settings_group = QGroupBox(self.t("border_settings_title"))
        settings_layout = QGridLayout(settings_group)
        
        # 描边厚度
        settings_layout.addWidget(QLabel(self.t("border_thickness")), 0, 0)
        self.border_thickness_var = self.config.get("border_thickness", 1)
        self.border_thickness_slider = QSlider(Qt.Horizontal)
        self.border_thickness_slider.setRange(1, 10)
        self.border_thickness_slider.setValue(int(self.border_thickness_var))
        self.border_thickness_slider.valueChanged.connect(self.update_border_thickness_label)
        self.border_thickness_entry = QLineEdit(str(self.border_thickness_var))
        self.border_thickness_entry.setFixedWidth(60)
        self.border_thickness_entry.textChanged.connect(self.on_border_thickness_text_changed)
        settings_layout.addWidget(self.border_thickness_slider, 0, 1)
        settings_layout.addWidget(self.border_thickness_entry, 0, 2)
        
        # 描边颜色
        settings_layout.addWidget(QLabel(self.t("border_color")), 1, 0)
        self.border_color_button = QPushButton(self.t("choose_color"))
        self.border_color_button.setStyleSheet(f"background-color: {self.config.get('border_color', '#000000')};")
        self.border_color_button.clicked.connect(self.choose_border_color)
        settings_layout.addWidget(self.border_color_button, 1, 1, 1, 2)
        
        # 描边透明度
        settings_layout.addWidget(QLabel(self.t("border_opacity")), 2, 0)
        self.border_opacity_var = self.config.get("border_opacity", 1.0)
        self.border_opacity_slider = QSlider(Qt.Horizontal)
        self.border_opacity_slider.setRange(0, 100)
        self.border_opacity_slider.setValue(int(self.border_opacity_var * 100))
        self.border_opacity_slider.valueChanged.connect(self.update_border_opacity_label)
        self.border_opacity_entry = QLineEdit(str(self.border_opacity_var))
        self.border_opacity_entry.setFixedWidth(60)
        self.border_opacity_entry.textChanged.connect(self.on_border_opacity_text_changed)
        settings_layout.addWidget(self.border_opacity_slider, 2, 1)
        settings_layout.addWidget(self.border_opacity_entry, 2, 2)
        
        layout.addWidget(settings_group)
        
        # 按钮
        button_layout = QHBoxLayout()
        self.ok_button = QPushButton(self.t("ok"))
        self.ok_button.clicked.connect(self.accept)
        self.cancel_button = QPushButton(self.t("cancel"))
        self.cancel_button.clicked.connect(self.reject)
        
        button_layout.addStretch()
        button_layout.addWidget(self.ok_button)
        button_layout.addWidget(self.cancel_button)
        layout.addLayout(button_layout)
    
    def update_border_thickness_label(self, value):
        """更新描边厚度标签"""
        self.border_thickness_entry.setText(str(value))
    
    def on_border_thickness_text_changed(self, text):
        """描边厚度文本改变"""
        try:
            value = int(text)
            if 1 <= value <= 10:
                self.border_thickness_slider.setValue(value)
        except ValueError:
            pass
    
    def choose_border_color(self):
        """选择描边颜色"""
        color = QColorDialog.getColor()
        if color.isValid():
            self.config["border_color"] = color.name()
            self.border_color_button.setStyleSheet(f"background-color: {color.name()};")
    
    def update_border_opacity_label(self, value):
        """更新描边透明度标签"""
        self.border_opacity_entry.setText(str(value / 100.0))
    
    def on_border_opacity_text_changed(self, text):
        """描边透明度文本改变"""
        try:
            value = float(text)
            if 0.0 <= value <= 1.0:
                self.border_opacity_slider.setValue(int(value * 100))
        except ValueError:
            pass
    
    def get_border_settings(self):
        """获取描边设置"""
        return {
            "border_thickness": self.border_thickness_slider.value(),
            "border_color": self.config.get("border_color", "#000000"),
            "border_opacity": self.border_opacity_slider.value() / 100.0
        }
    
    def accept(self):
        """确定按钮"""
        # 更新配置
        self.config["border_thickness"] = self.border_thickness_slider.value()
        self.config["border_opacity"] = self.border_opacity_slider.value() / 100.0
        super().accept()
