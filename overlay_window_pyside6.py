#!/usr/bin/env python3
# -*- coding: utf-8 -*-

import os
import ctypes
from ctypes import wintypes
from PySide6.QtWidgets import QWidget, QApplication
from PySide6.QtCore import Qt, QTimer, QPoint
from PySide6.QtGui import QPainter, QColor, QBrush, QPen, QImage, QPixmap

# Windows API 钩子相关
WH_MOUSE_LL = 14
WM_RBUTTONDOWN = 0x0204
HC_ACTION = 0

# 定义鼠标钩子结构
class MSLLHOOKSTRUCT(ctypes.Structure):
    _fields_ = [
        ("pt", wintypes.POINT),
        ("mouseData", wintypes.DWORD),
        ("flags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.c_size_t)
    ]

# 全局鼠标钩子变量
mouse_hook = None
MOUSE_HOOK_CALLBACK = ctypes.CFUNCTYPE(
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.POINTER(MSLLHOOKSTRUCT)
)

# 全局右键回调函数
global_right_click_callback = None

# 保存回调函数引用，防止被垃圾回收
mouse_hook_callback_ref = None

def mouse_hook_proc(nCode, wParam, lParam):
    """鼠标钩子回调函数"""
    global global_right_click_callback
    
    try:
        # 检查是否是右键按下事件
        if wParam == WM_RBUTTONDOWN and nCode >= 0:
            if global_right_click_callback:
                # 使用 QTimer 延迟执行，避免在钩子回调中直接调用
                from PySide6.QtCore import QTimer
                QTimer.singleShot(0, global_right_click_callback)
        
        # 继续传递事件（不拦截）
        return ctypes.windll.user32.CallNextHookEx(mouse_hook, nCode, wParam, lParam)
    except:
        # 出错时继续传递事件
        return ctypes.windll.user32.CallNextHookEx(mouse_hook, nCode, wParam, lParam)


class OverlayWindow(QWidget):
    def __init__(self, config):
        super().__init__()
        self.config = config
        
        # 拖动相关变量
        self.is_drag_mode = False
        self.is_dragging = False
        self.drag_start_pos = QPoint()
        self.crosshair_pos = None  # 准星的当前位置
        
        # 右键回调函数
        self.right_click_callback = None
        
        # 设置窗口属性
        self.setWindowFlags(
            Qt.FramelessWindowHint |  # 无边框
            Qt.WindowStaysOnTopHint |  # 置顶
            Qt.Tool  # 工具窗口
        )
        
        # 设置窗口透明
        self.setAttribute(Qt.WA_TranslucentBackground)
        self.setAttribute(Qt.WA_ShowWithoutActivating)
        
        # 设置全屏
        self.showFullScreen()
        
        # 初始化鼠标穿透
        self.setMouseTracking(False)
        self.setWindowFlag(Qt.WindowTransparentForInput, True)
        
        # 定时器用于重绘
        self.timer = QTimer()
        self.timer.timeout.connect(self.update)
        self.timer.start(50)  # 20 FPS
        
        # 全屏自动置顶检测
        self.auto_topmost_on_fullscreen = False
        self.last_fullscreen_hwnd = 0
        self.fullscreen_timer = QTimer()
        self.fullscreen_timer.timeout.connect(self._check_and_topmost_on_fullscreen)
    
    def set_auto_topmost_on_fullscreen(self, enabled):
        """启用或禁用全屏自动置顶检测。

        当检测到前台切换到一个覆盖整个屏幕的窗口(例如无边框全屏游戏)时,
        通过 SetWindowPos 强制把准星窗口重新提到最顶层,避免被游戏遮挡。

        注意:对 D3D 独占全屏无效(硬件级独占显示器)。
        """
        self.auto_topmost_on_fullscreen = enabled
        if enabled:
            self.fullscreen_timer.start(1000)  # 每秒检查一次,开销极小
        else:
            self.fullscreen_timer.stop()
            self.last_fullscreen_hwnd = 0
    
    def _check_and_topmost_on_fullscreen(self):
        """检测前台全屏窗口并自动重新置顶准星。"""
        if not self.auto_topmost_on_fullscreen or not self.isVisible():
            return
        try:
            user32 = ctypes.windll.user32
            hwnd = user32.GetForegroundWindow()
            if not hwnd:
                return
            
            # 排除准星窗口自身
            try:
                if hwnd == int(self.winId()):
                    return
            except Exception:
                pass
            
            # 获取前台窗口矩形
            rect = wintypes.RECT()
            if not user32.GetWindowRect(hwnd, ctypes.byref(rect)):
                return
            
            # 主屏幕尺寸
            sw = user32.GetSystemMetrics(0)  # SM_CXSCREEN
            sh = user32.GetSystemMetrics(1)  # SM_CYSCREEN
            
            # 判断是否覆盖整个主屏幕(允许少量边距误差)
            is_fullscreen = (rect.left <= 0 and rect.top <= 0 and
                             rect.right >= sw and rect.bottom >= sh)
            
            if is_fullscreen and hwnd != self.last_fullscreen_hwnd:
                # 前台切换到一个新的全屏窗口,强制把准星提到最顶层
                my_hwnd = int(self.winId())
                # HWND_TOPMOST = -1
                # SWP_NOMOVE=0x0002 | SWP_NOSIZE=0x0001 | SWP_NOACTIVATE=0x0010 | SWP_SHOWWINDOW=0x0040
                user32.SetWindowPos(my_hwnd, -1, 0, 0, 0, 0,
                                    0x0002 | 0x0001 | 0x0010 | 0x0040)
                self.last_fullscreen_hwnd = hwnd
            elif not is_fullscreen:
                # 退出全屏,重置记录,下次进入全屏会再次触发
                self.last_fullscreen_hwnd = 0
        except Exception as e:
            print(f"全屏置顶检测出错: {e}")
    
    def updateConfig(self, config):
        """更新配置"""
        self.config = config
        # 重置crosshair_pos，让准星位置跟随配置
        self.crosshair_pos = None
        self.update()
    
    def center_crosshair(self):
        """将准星居中"""
        screen_size = QApplication.primaryScreen().size()
        self.crosshair_pos = QPoint(screen_size.width() // 2, screen_size.height() // 2)
        self.config["position"] = {"x": "center", "y": "center"}
        self.update()
    
    def draw_cross(self, painter, center, size, thickness, color):
        """绘制十字准星"""
        # 如果启用描边，先绘制描边
        if self.config.get("enable_border", False):
            border_color = QColor(self.config.get("border_color", "#000000"))
            border_opacity = self.config.get("border_opacity", 1.0)
            border_thickness = self.config.get("border_thickness", 1)
            border_color.setAlphaF(border_opacity)
            painter.setPen(QPen(border_color, thickness + border_thickness * 2))
            painter.drawLine(center[0] - size, center[1], center[0] + size, center[1])  # 水平线
            painter.drawLine(center[0], center[1] - size, center[0], center[1] + size)  # 垂直线
        
        # 绘制主体
        painter.setPen(QPen(color, thickness))
        painter.drawLine(center[0] - size, center[1], center[0] + size, center[1])  # 水平线
        painter.drawLine(center[0], center[1] - size, center[0], center[1] + size)  # 垂直线
    
    def draw_dot(self, painter, center, size, color):
        """绘制圆点准星"""
        # 如果启用描边，先绘制描边
        if self.config.get("enable_border", False):
            border_color = QColor(self.config.get("border_color", "#000000"))
            border_opacity = self.config.get("border_opacity", 1.0)
            border_thickness = self.config.get("border_thickness", 1)
            border_color.setAlphaF(border_opacity)
            border_size = size + border_thickness * 2
            painter.setPen(QPen(border_color, 1))
            painter.setBrush(QBrush(border_color))
            painter.drawEllipse(center[0] - border_size//2, center[1] - border_size//2, border_size, border_size)
        
        # 绘制主体
        painter.setPen(QPen(color, 1))
        painter.setBrush(QBrush(color))
        # 修正：drawEllipse的参数是左上角坐标，需要减去一半的尺寸让中心点对齐
        painter.drawEllipse(center[0] - size//2, center[1] - size//2, size, size)
    
    def draw_square(self, painter, center, size, color):
        """绘制方块准星"""
        # 如果启用描边，先绘制描边
        if self.config.get("enable_border", False):
            border_color = QColor(self.config.get("border_color", "#000000"))
            border_opacity = self.config.get("border_opacity", 1.0)
            border_thickness = self.config.get("border_thickness", 1)
            border_color.setAlphaF(border_opacity)
            border_size = size + border_thickness * 2
            painter.setPen(QPen(border_color, 1))
            painter.setBrush(QBrush(border_color))
            painter.drawRect(center[0] - border_size//2, center[1] - border_size//2, border_size, border_size)
        
        # 绘制主体
        painter.setPen(QPen(color, 2))
        painter.setBrush(QBrush(color))
        painter.drawRect(center[0] - size//2, center[1] - size//2, size, size)
    
    def draw_circle(self, painter, center, size, thickness, color):
        """绘制圆圈准星"""
        # 如果启用描边，先绘制描边
        if self.config.get("enable_border", False):
            border_color = QColor(self.config.get("border_color", "#000000"))
            border_opacity = self.config.get("border_opacity", 1.0)
            border_thickness = self.config.get("border_thickness", 1)
            border_color.setAlphaF(border_opacity)
            painter.setPen(QPen(border_color, thickness + border_thickness * 2))
            painter.setBrush(QBrush(Qt.transparent))
            painter.drawEllipse(center[0] - size//2, center[1] - size//2, size, size)
        
        # 绘制主体
        painter.setPen(QPen(color, thickness))
        painter.setBrush(QBrush(Qt.transparent))
        # 修正：drawEllipse的参数是左上角坐标，需要减去一半的尺寸让中心点对齐
        painter.drawEllipse(center[0] - size//2, center[1] - size//2, size, size)
    
    def draw_hollow_cross(self, painter, center, gap_size, line_length, line_thickness, color):
        """绘制空心十字准星"""
        # 如果启用描边，先绘制描边
        if self.config.get("enable_border", False):
            border_color = QColor(self.config.get("border_color", "#000000"))
            border_opacity = self.config.get("border_opacity", 1.0)
            border_thickness = self.config.get("border_thickness", 1)
            border_color.setAlphaF(border_opacity)
            painter.setPen(QPen(border_color, line_thickness + border_thickness * 2))
            
            # 绘制四段分离的直线（描边）
            painter.drawLine(center[0], center[1] - gap_size, center[0], center[1] - gap_size - line_length)
            painter.drawLine(center[0], center[1] + gap_size, center[0], center[1] + gap_size + line_length)
            painter.drawLine(center[0] - gap_size, center[1], center[0] - gap_size - line_length, center[1])
            painter.drawLine(center[0] + gap_size, center[1], center[0] + gap_size + line_length, center[1])
        
        # 绘制主体
        painter.setPen(QPen(color, line_thickness))
        
        # 绘制四段分离的直线
        # 上半部分
        painter.drawLine(center[0], center[1] - gap_size, center[0], center[1] - gap_size - line_length)
        # 下半部分
        painter.drawLine(center[0], center[1] + gap_size, center[0], center[1] + gap_size + line_length)
        # 左半部分
        painter.drawLine(center[0] - gap_size, center[1], center[0] - gap_size - line_length, center[1])
        # 右半部分
        painter.drawLine(center[0] + gap_size, center[1], center[0] + gap_size + line_length, center[1])
    
    def draw_hollow_square(self, painter, center, size, thickness, color):
        """绘制空心方框准星"""
        # 如果启用描边，先绘制描边
        if self.config.get("enable_border", False):
            border_color = QColor(self.config.get("border_color", "#000000"))
            border_opacity = self.config.get("border_opacity", 1.0)
            border_thickness = self.config.get("border_thickness", 1)
            border_color.setAlphaF(border_opacity)
            painter.setPen(QPen(border_color, thickness + border_thickness * 2))
            painter.setBrush(QBrush(Qt.transparent))
            painter.drawRect(center[0] - size//2, center[1] - size//2, size, size)
        
        # 绘制主体
        painter.setPen(QPen(color, thickness))
        painter.setBrush(QBrush(Qt.transparent))
        painter.drawRect(center[0] - size//2, center[1] - size//2, size, size)
    
    def draw_hollow_cross_dot(self, painter, center, gap_size, line_length, line_thickness, dot_size, color):
        """绘制空心十字加中心点准星"""
        # 先绘制空心十字（已包含描边支持）
        self.draw_hollow_cross(painter, center, gap_size, line_length, line_thickness, color)
        
        # 绘制中心点描边
        if self.config.get("enable_border", False):
            border_color = QColor(self.config.get("border_color", "#000000"))
            border_opacity = self.config.get("border_opacity", 1.0)
            border_thickness = self.config.get("border_thickness", 1)
            border_color.setAlphaF(border_opacity)
            border_dot_size = dot_size + border_thickness * 2
            painter.setPen(QPen(border_color, 1))
            painter.setBrush(QBrush(border_color))
            painter.drawEllipse(center[0] - border_dot_size//2, center[1] - border_dot_size//2, border_dot_size, border_dot_size)
        
        # 绘制中心点主体
        painter.setPen(QPen(color, 1))
        painter.setBrush(QBrush(color))
        # drawEllipse的参数是左上角坐标，所以要减去一半的尺寸让中心点对齐
        painter.drawEllipse(center[0] - dot_size//2, center[1] - dot_size//2, dot_size, dot_size)
    
    def draw_custom_image(self, painter, center, opacity):
        """绘制自定义图片准星"""
        image_path = self.config.get("custom_image_path", "")
        image_scale = self.config.get("custom_image_scale", 1.0)
        
        if not image_path or not os.path.exists(image_path):
            return
        
        try:
            # 加载图片
            pixmap = QPixmap(image_path)
            
            if pixmap.isNull():
                return
            
            # 应用透明度
            scaled_pixmap = pixmap.scaled(
                int(pixmap.width() * image_scale),
                int(pixmap.height() * image_scale),
                Qt.KeepAspectRatio,
                Qt.SmoothTransformation
            )
            
            # 创建带透明度的图片
            opacity_image = QImage(scaled_pixmap.size(), QImage.Format_ARGB32)
            opacity_image.fill(Qt.transparent)
            
            image_painter = QPainter(opacity_image)
            image_painter.setOpacity(opacity)
            image_painter.drawPixmap(0, 0, scaled_pixmap)
            image_painter.end()
            
            # 绘制图片，中心对齐
            x = center[0] - scaled_pixmap.width() // 2
            y = center[1] - scaled_pixmap.height() // 2
            
            painter.setPen(Qt.NoPen)
            painter.drawImage(x, y, opacity_image)
            
        except Exception as e:
            print(f"绘制自定义图片失败: {e}")
    
    def toggleDragMode(self):
        """切换拖动模式"""
        self.is_drag_mode = not self.is_drag_mode
        
        if self.is_drag_mode:
            # 进入拖动模式：禁用鼠标穿透，启用鼠标跟踪
            self.setWindowFlag(Qt.WindowTransparentForInput, False)
            self.setMouseTracking(True)
            self.setCursor(Qt.OpenHandCursor)
            
            # 初始化准星位置
            if self.crosshair_pos is None:
                screen_size = QApplication.primaryScreen().size()
                position = self.config.get("position", {"x": "center", "y": "center"})
                if position["x"] == "center":
                    center_x = screen_size.width() // 2
                else:
                    center_x = int(position["x"])
                
                if position["y"] == "center":
                    center_y = screen_size.height() // 2
                else:
                    center_y = int(position["y"])
                
                self.crosshair_pos = QPoint(center_x, center_y)
        else:
            # 退出拖动模式：启用鼠标穿透，禁用鼠标跟踪
            self.setWindowFlag(Qt.WindowTransparentForInput, True)
            self.setMouseTracking(False)
            self.setCursor(Qt.ArrowCursor)
        
        # 重新显示窗口以应用窗口标志更改
        self.hide()
        self.showFullScreen()
        
        return self.is_drag_mode
    
    def get_crosshair_position(self):
        """获取准星当前位置"""
        if self.crosshair_pos:
            return (self.crosshair_pos.x(), self.crosshair_pos.y())
        else:
            # 如果没有拖动过，返回配置中的位置
            position = self.config.get("position", {"x": "center", "y": "center"})
            if position["x"] == "center" and position["y"] == "center":
                screen_size = QApplication.primaryScreen().size()
                return (screen_size.width() // 2, screen_size.height() // 2)
            else:
                return (int(position["x"]), int(position["y"]))
    
    def mousePressEvent(self, event):
        """鼠标按下事件"""
        if self.is_drag_mode and event.button() == Qt.LeftButton:
            self.is_dragging = True
            self.drag_start_pos = event.position().toPoint()
            self.setCursor(Qt.ClosedHandCursor)
    
    def mouseReleaseEvent(self, event):
        """鼠标释放事件"""
        if self.is_drag_mode and event.button() == Qt.LeftButton:
            self.is_dragging = False
            self.setCursor(Qt.OpenHandCursor)
        elif event.button() == Qt.RightButton:
            # 右键点击触发回调
            if self.right_click_callback:
                self.right_click_callback()
    
    def mouseMoveEvent(self, event):
        """鼠标移动事件"""
        if self.is_drag_mode and self.is_dragging:
            # 计算移动距离
            current_pos = event.position().toPoint()
            delta = current_pos - self.drag_start_pos
            
            # 更新准星位置
            if self.crosshair_pos:
                self.crosshair_pos += delta
            
            # 更新拖动起始位置
            self.drag_start_pos = current_pos
            
            # 更新配置中的位置
            if self.crosshair_pos:
                self.config["position"] = {
                    "x": self.crosshair_pos.x(), 
                    "y": self.crosshair_pos.y()
                }
            
            # 触发重绘
            self.update()
    
    def paintEvent(self, event):
        """绘制事件"""
        painter = QPainter(self)
        painter.setRenderHint(QPainter.Antialiasing)
        
        # 获取配置参数
        shape = self.config.get("shape", "cross")
        size = self.config.get("size", 20)
        thickness = self.config.get("thickness", 2)
        opacity = self.config.get("opacity", 0.8)
        color = self.config.get("color", "#FF0000")
        position = self.config.get("position", {"x": "center", "y": "center"})
        
        # 设置颜色和透明度
        qcolor = QColor(color)
        qcolor.setAlphaF(opacity)
        
        # 计算中心位置
        if self.is_drag_mode and self.crosshair_pos:
            # 在拖动模式下，使用拖动后的位置
            center_x = self.crosshair_pos.x()
            center_y = self.crosshair_pos.y()
        else:
            # 正常模式下，使用配置中的位置
            screen_size = QApplication.primaryScreen().size()
            if position["x"] == "center":
                center_x = screen_size.width() // 2
            else:
                center_x = int(position["x"])
            
            if position["y"] == "center":
                center_y = screen_size.height() // 2
            else:
                center_y = int(position["y"])
        
        center = (center_x, center_y)
        
        # 根据形状绘制准星
        if shape == "cross":
            self.draw_cross(painter, center, size, thickness, qcolor)
        elif shape == "dot":
            self.draw_dot(painter, center, size, qcolor)
        elif shape == "square":
            self.draw_square(painter, center, size, qcolor)
        elif shape == "circle":
            self.draw_circle(painter, center, size, thickness, qcolor)
        elif shape == "hollow_cross":
            gap_size = self.config.get("hollow_gap", size // 3)
            line_length = self.config.get("hollow_length", size)
            line_thickness = self.config.get("hollow_thickness", thickness)
            self.draw_hollow_cross(painter, center, gap_size, line_length, line_thickness, qcolor)
        elif shape == "hollow_square":
            self.draw_hollow_square(painter, center, size, thickness, qcolor)
        elif shape == "hollow_cross_dot":
            gap_size = self.config.get("hollow_gap", size // 3)
            line_length = self.config.get("hollow_length", size)
            line_thickness = self.config.get("hollow_thickness", thickness)
            dot_size = self.config.get("center_dot_size",3)
            self.draw_hollow_cross_dot(painter, center, gap_size, line_length, line_thickness, dot_size, qcolor)
        elif shape == "custom_image":
            self.draw_custom_image(painter, center, opacity)
        
        # 在拖动模式下绘制额外的提示信息
        if self.is_drag_mode:
            # 绘制拖动模式提示
            painter.setPen(QPen(QColor(255,255,255, 128), 1))
            hint_text = "Drag mode - drag the crosshair to the desired position" if self.config.get("language","zh")=="en" else "拖动模式 - 拖动准星到想要的位置"
            text_rect = painter.boundingRect(10, 10, 300, 30, Qt.AlignLeft, hint_text)
            painter.fillRect(text_rect.adjusted(-5, -5, 5, 5), QColor(0, 0, 0, 128))
            painter.drawText(10, 10, hint_text)
    
    def register_global_mouse_hook(self, callback):
        """注册全局鼠标钩子"""
        global global_right_click_callback, mouse_hook, mouse_hook_callback_ref
        try:
            # 设置全局回调
            global_right_click_callback = callback
            self.right_click_callback = callback
            
            # 创建回调函数并保存引用
            callback_func = MOUSE_HOOK_CALLBACK(mouse_hook_proc)
            mouse_hook_callback_ref = callback_func  # 保存引用防止被垃圾回收
            
            # 注册低级鼠标钩子
            mouse_hook = ctypes.windll.user32.SetWindowsHookExA(
                WH_MOUSE_LL,
                callback_func,
                None,
                0
            )
            
            if mouse_hook:
                print("全局右键钩子已注册")
            else:
                print("注册全局右键钩子失败")
                
        except Exception as e:
            print(f"注册全局鼠标钩子出错: {e}")
    
    
    def unregister_global_mouse_hook(self):
        """注销全局鼠标钩子"""
        global global_right_click_callback, mouse_hook
        try:
            if mouse_hook:
                ctypes.windll.user32.UnhookWindowsHookEx(mouse_hook)
                mouse_hook = None
                print("全局右键功能已注销")
            global_right_click_callback = None
            self.right_click_callback = None
        except Exception as e:
            print(f"注销全局鼠标钩子出错:{e}")