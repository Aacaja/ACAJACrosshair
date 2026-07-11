#!/usr/bin/env python3
# -*- coding: utf-8 -*-

"""
背景图片框选裁剪对话框:
选区为固定比例(与主界面570:900同比例),可拖动改变位置和缩放大小。
"""

from PySide6.QtWidgets import (
    QDialog, QVBoxLayout, QHBoxLayout, QPushButton, QLabel, QSizePolicy
)
from PySide6.QtCore import Qt, QRect, QPoint, QSize, Signal
from PySide6.QtGui import QPixmap, QPainter, QColor, QPen, QBrush, QCursor


# 选区固定宽高比 = 主界面 570:900
ASPECT_W = 570
ASPECT_H = 900
ASPECT_RATIO = ASPECT_W / ASPECT_H  # ≈ 0.633


class BackgroundCropDialog(QDialog):
    """背景图片框选裁剪对话框(固定比例选区)。

    用法:
        dlg = BackgroundCropDialog(pixmap, parent)
        if dlg.exec() == QDialog.Accepted:
            cropped = dlg.get_cropped_pixmap()  # 裁剪后的 QPixmap
    """

    def __init__(self, pixmap, parent=None, language="zh"):
        super().__init__(parent)
        self.language = language
        self.source_pixmap = pixmap
        self.cropped_pixmap = None

        self.setWindowTitle(self.t("crop_title"))
        self.setModal(True)
        self.resize(640, 640)

        self._setup_ui()

    def t(self, key):
        """简易翻译。"""
        tr = {
            "crop_title": {"zh": "框选背景区域", "en": "Select Background Region"},
            "crop_hint": {
                "zh": "拖动方框选择背景区域\n· 拖动方框内部 = 移动位置\n· 拖动边角/边线 = 缩放大小\n· 双击 = 全选整张图\n选区比例与界面固定一致",
                "en": "Drag the box to select background region\n· Drag inside = move\n· Drag edge/corner = resize\n· Double-click = select all\nAspect ratio is fixed to the UI"
            },
            "ok": {"zh": "确定", "en": "OK"},
            "cancel": {"zh": "取消", "en": "Cancel"},
        }
        d = tr.get(key, {})
        return d.get(self.language, d.get("zh", key))

    def _setup_ui(self):
        layout = QVBoxLayout(self)

        # 提示
        hint = QLabel(self.t("crop_hint"))
        hint.setWordWrap(True)
        layout.addWidget(hint)

        # 图片预览标签(支持固定比例框选)
        self.image_label = AspectRatioCropLabel(self.source_pixmap)
        self.image_label.setAlignment(Qt.AlignCenter)
        self.image_label.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)
        layout.addWidget(self.image_label, 1)

        # 按钮
        btn_layout = QHBoxLayout()
        btn_layout.addStretch()
        ok_btn = QPushButton(self.t("ok"))
        cancel_btn = QPushButton(self.t("cancel"))
        ok_btn.clicked.connect(self._accept)
        cancel_btn.clicked.connect(self.reject)
        btn_layout.addWidget(ok_btn)
        btn_layout.addWidget(cancel_btn)
        layout.addLayout(btn_layout)

    def _accept(self):
        """确定:根据选区裁剪图片。"""
        # 获取图片坐标系下的选区(实际像素)
        src_rect = self.image_label.get_source_selection_rect()
        if src_rect is not None and src_rect.width() > 10 and src_rect.height() > 10:
            src_rect = src_rect.intersected(self.source_pixmap.rect())
            if not src_rect.isEmpty():
                self.cropped_pixmap = self.source_pixmap.copy(src_rect)
        else:
            # 没有有效选区,用整张图
            self.cropped_pixmap = self.source_pixmap
        self.accept()

    def get_cropped_pixmap(self):
        """返回裁剪后的 QPixmap,可能为 None。"""
        return self.cropped_pixmap


class AspectRatioCropLabel(QLabel):
    """可拖拽固定比例框选的图片标签。

    - 拖动方框内部:移动位置
    - 拖动边角/边线:缩放大小(保持固定比例)
    - 双击:全选整张图
    发出 selectionChanged() 信号。
    """

    selectionChanged = Signal()

    # 操作模式
    NONE = 0
    MOVE = 1
    RESIZE = 2

    # 缩放手柄判定阈值(像素)
    HANDLE_SIZE = 12

    def __init__(self, pixmap, parent=None):
        super().__init__(parent)
        self._source_pixmap = pixmap  # 原图
        self._display_pixmap = None   # 缩放后用于显示的图
        self._img_rect = QRect()      # 图片在label中的实际显示区域

        # 选区(display 坐标系)
        self._selection = QRect()
        self._mode = self.NONE
        self._drag_offset = QPoint()
        self._handle = ""  # "tl","tr","bl","br","t","b","l","r",""

        self.setScaledPixmap(pixmap)
        self.setMouseTracking(True)
        self.setMinimumSize(400, 400)

    def setScaledPixmap(self, pixmap):
        """按标签大小缩放显示图片。"""
        if pixmap is None or pixmap.isNull():
            return
        self._source_pixmap = pixmap

    def _update_display(self):
        """根据当前size重新生成显示用pixmap并初始化选区。"""
        if self._source_pixmap is None or self._source_pixmap.isNull():
            return
        lw = max(self.width() - 20, 100)
        lh = max(self.height() - 20, 100)
        scaled = self._source_pixmap.scaled(
            lw, lh,
            Qt.KeepAspectRatio,
            Qt.SmoothTransformation
        )
        self._display_pixmap = scaled
        self.setPixmap(scaled)
        # 计算图片在label中的居中位置
        px = (self.width() - scaled.width()) // 2
        py = (self.height() - scaled.height()) // 2
        self._img_rect = QRect(px, py, scaled.width(), scaled.height())
        # 默认选区:尽量大,居中
        self._selection = self._fit_default_selection()

    def _fit_default_selection(self):
        """计算默认选区:在图片内尽量大、居中、保持固定比例。"""
        if self._img_rect.isNull():
            return QRect()
        iw = self._img_rect.width()
        ih = self._img_rect.height()
        # 在保持比例的前提下取最大尺寸
        if iw / ih > ASPECT_RATIO:
            # 图片更宽,以高度为基准
            sh = ih
            sw = int(sh * ASPECT_RATIO)
        else:
            # 图片更高/等比,以宽度为基准
            sw = iw
            sh = int(sw / ASPECT_RATIO)
        sx = self._img_rect.x() + (iw - sw) // 2
        sy = self._img_rect.y() + (ih - sh) // 2
        return QRect(sx, sy, sw, sh)

    def resizeEvent(self, event):
        super().resizeEvent(event)
        self._update_display()
        self.update()

    def paintEvent(self, event):
        super().paintEvent(event)
        if self._display_pixmap is None:
            return
        painter = QPainter(self)
        # 画半透明遮罩(选区外变暗)
        overlay = QColor(0, 0, 0, 120)
        painter.setBrush(overlay)
        painter.setPen(Qt.NoPen)
        img = self._img_rect
        sel = self._selection
        # 上下左右四块遮罩
        painter.drawRect(QRect(img.x(), img.y(), img.width(), sel.y() - img.y()))
        bottom = sel.bottom() + 1
        painter.drawRect(QRect(img.x(), bottom, img.width(), img.bottom() - bottom + 1))
        painter.drawRect(QRect(img.x(), sel.y(), sel.x() - img.x(), sel.height()))
        right = sel.right() + 1
        painter.drawRect(QRect(right, sel.y(), img.right() - right + 1, sel.height()))
        # 画选区边框
        pen = QPen(QColor("#0a84ff"), 2)
        painter.setPen(pen)
        painter.setBrush(Qt.NoBrush)
        painter.drawRect(sel)
        # 画缩放手柄(四角和四边中点)
        painter.setBrush(QColor("#0a84ff"))
        painter.setPen(Qt.NoPen)
        for h in ["tl", "tr", "bl", "br", "t", "b", "l", "r"]:
            hr = self._handle_rect(h)
            if hr:
                painter.drawRect(hr)

    def _handle_rect(self, handle):
        """返回某个手柄的方块矩形(display坐标)。"""
        s = self._selection
        hs = self.HANDLE_SIZE
        if handle == "tl":
            return QRect(s.x() - hs//2, s.y() - hs//2, hs, hs)
        if handle == "tr":
            return QRect(s.right() - hs//2, s.y() - hs//2, hs, hs)
        if handle == "bl":
            return QRect(s.x() - hs//2, s.bottom() - hs//2, hs, hs)
        if handle == "br":
            return QRect(s.right() - hs//2, s.bottom() - hs//2, hs, hs)
        return None

    def _hit_handle(self, pos):
        """判断鼠标是否点在某手柄上,返回handle名或''。"""
        for h in ["tl", "tr", "bl", "br"]:
            hr = self._handle_rect(h)
            if hr and hr.contains(pos):
                return h
        return ""

    def _constrain_selection(self, rect):
        """将选区限制在图片范围内,并保持固定比例(以rect中心为基准)。"""
        rect = rect.intersected(self._img_rect)
        if rect.width() < 20 or rect.height() < 20:
            return self._selection
        # 修正为固定比例(以当前宽高为准,较小的那个)
        w = rect.width()
        h = rect.height()
        if w / h > ASPECT_RATIO:
            # 太宽,收窄宽度
            w = int(h * ASPECT_RATIO)
        else:
            # 太高,收窄高度
            h = int(w / ASPECT_RATIO)
        cx = rect.center().x()
        cy = rect.center().y()
        new_rect = QRect(cx - w//2, cy - h//2, w, h)
        # 边界限制:如果超出图片范围,平移
        if new_rect.x() < self._img_rect.x():
            new_rect.moveLeft(self._img_rect.x())
        if new_rect.right() > self._img_rect.right():
            new_rect.moveRight(self._img_rect.right())
        if new_rect.y() < self._img_rect.y():
            new_rect.moveTop(self._img_rect.y())
        if new_rect.bottom() > self._img_rect.bottom():
            new_rect.moveBottom(self._img_rect.bottom())
        return new_rect

    def mousePressEvent(self, event):
        if event.button() == Qt.LeftButton:
            pos = event.position().toPoint()
            # 优先检查手柄
            handle = self._hit_handle(pos)
            if handle:
                self._mode = self.RESIZE
                self._handle = handle
            elif self._selection.contains(pos):
                self._mode = self.MOVE
                self._drag_offset = pos - self._selection.topLeft()
            else:
                # 点击空白:移动选区中心到该点
                self._mode = self.MOVE
                new_sel = self._selection
                new_sel.moveCenter(pos)
                self._selection = self._constrain_selection(new_sel)
                self._drag_offset = pos - self._selection.topLeft()
                self.update()
                self.selectionChanged.emit()
        super().mousePressEvent(event)

    def mouseMoveEvent(self, event):
        pos = event.position().toPoint()
        if self._mode == self.MOVE:
            new_top_left = pos - self._drag_offset
            new_sel = QRect(new_top_left, self._selection.size())
            new_sel = self._constrain_selection(new_sel)
            self._selection = new_sel
            self.update()
            self.selectionChanged.emit()
        elif self._mode == self.RESIZE:
            self._resize_selection(pos)
            self.update()
            self.selectionChanged.emit()
        else:
            # 悬停时改变鼠标光标
            handle = self._hit_handle(pos)
            if handle in ("tl", "br"):
                self.setCursor(Qt.SizeFDiagCursor)
            elif handle in ("tr", "bl"):
                self.setCursor(Qt.SizeBDiagCursor)
            elif self._selection.contains(pos):
                self.setCursor(Qt.SizeAllCursor)
            else:
                self.setCursor(Qt.ArrowCursor)
        super().mouseMoveEvent(event)

    def mouseReleaseEvent(self, event):
        self._mode = self.NONE
        self._handle = ""
        super().mouseReleaseEvent(event)

    def mouseDoubleClickEvent(self, event):
        """双击:全选整张图(保持比例的居中最大选区)。"""
        self._selection = self._fit_default_selection()
        self.update()
        self.selectionChanged.emit()
        super().mouseDoubleClickEvent(event)

    def _resize_selection(self, pos):
        """根据手柄和鼠标位置调整选区大小(保持固定比例 570:900)。

        - 若按比例修正后的目标矩形四条边都在 _img_rect 内(含边界),更新 _selection。
        - 若任一边越界(超出 >=1px),放弃本次缩放,保持 _selection 的尺寸与左上角位置不变。
        """
        s = self._selection.normalized()
        img = self._img_rect
        h = self._handle
        # 根据拖动的手柄计算原始边界。
        # 注意:此处不向图片边界(img)钳制,否则"已贴边继续放大"会被钳制成收缩;
        # 边界判定统一交由下方的越界检查处理,仅保留最小尺寸约束以防止选区翻转。
        left = s.left()
        top = s.top()
        right = s.right()
        bottom = s.bottom()
        if "l" in h:
            left = min(pos.x(), right - 20)
        if "r" in h:
            right = max(pos.x(), left + 20)
        if "t" in h:
            top = min(pos.y(), bottom - 20)
        if "b" in h:
            bottom = max(pos.y(), top + 20)
        new_w = right - left + 1
        new_h = bottom - top + 1
        if new_w <= 0 or new_h <= 0:
            return
        # 根据拖动方向调整以保持固定比例(保持未拖动的边不动)
        cur_ratio = new_w / new_h
        if cur_ratio > ASPECT_RATIO:
            # 偏宽,依据高度调整宽度
            target_w = int(new_h * ASPECT_RATIO)
            if "l" in h and "r" not in h:
                # 左边拖动,右边不动
                left = right - target_w + 1
            else:
                right = left + target_w - 1
        else:
            # 偏高,依据宽度调整高度
            target_h = int(new_w / ASPECT_RATIO)
            if "t" in h and "b" not in h:
                top = bottom - target_h + 1
            else:
                bottom = top + target_h - 1
        new_rect = QRect(left, top, right - left + 1, bottom - top + 1).normalized()
        # 边界判定:任一边越界(超出 >=1px)则放弃本次缩放,保持 _selection 的
        # 尺寸与左上角位置完全不变(既不放大越界,也不收缩任何维度)。
        # 显式覆盖"已贴边继续放大":目标越界时直接 return,不改变当前选区。
        if new_rect.left() < img.left() or new_rect.right() > img.right() or \
           new_rect.top() < img.top() or new_rect.bottom() > img.bottom():
            return
        # 目标矩形四边均在 _img_rect 内(含边界)时才更新选区
        self._selection = new_rect

    def get_source_selection_rect(self):
        """返回原图坐标系下的选区QRect(实际像素),无选区返回None。"""
        if self._display_pixmap is None or self._selection.isNull():
            return None
        sx = self._source_pixmap.width() / self._display_pixmap.width()
        sy = self._source_pixmap.height() / self._display_pixmap.height()
        return QRect(
            int((self._selection.x() - self._img_rect.x()) * sx),
            int((self._selection.y() - self._img_rect.y()) * sy),
            int(self._selection.width() * sx),
            int(self._selection.height() * sy),
        )
