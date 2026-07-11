#!/usr/bin/env python3
# -*- coding: utf-8 -*-

"""
准星程序 v3.0.0 打包脚本
使用 PyInstaller 打包成独立的可执行文件
"""

import os
import sys
import subprocess
import shutil
from datetime import datetime
import urllib.request
import tempfile

def check_upx():
    """检查并安装UPX压缩工具"""
    upx_path = "upx.exe"
    
    # 检查本地是否有UPX
    if os.path.exists(upx_path):
        print(f"[OK] UPX 已存在")
        # 返回绝对路径：subprocess 在 shell=False 时不会用当前目录搜索裸文件名，
        # 传相对路径 "upx.exe" 会导致后续 compress_with_upx 中 CreateProcess 报 WinError 2
        return os.path.abspath(upx_path)
    
    # 尝试从系统PATH中查找
    try:
        result = subprocess.run(["upx", "--version"], capture_output=True, text=True, timeout=5)
        if result.returncode == 0:
            print(f"[OK] UPX 已安装在系统PATH中")
            return "upx"
    except:
        pass
    
    # 下载UPX
    print("[INFO] UPX 未找到，正在下载...")
    upx_url = "https://github.com/upx/upx/releases/download/v4.2.2/upx-4.2.2-win64.zip"
    
    try:
        # 创建临时目录
        temp_dir = tempfile.mkdtemp()
        zip_path = os.path.join(temp_dir, "upx.zip")
        
        print(f"[INFO] 下载 UPX 从 {upx_url}")
        urllib.request.urlretrieve(upx_url, zip_path)
        
        # 解压UPX
        print("[INFO] 解压 UPX...")
        shutil.unpack_archive(zip_path, temp_dir)
        
        # 查找upx.exe
        for root, dirs, files in os.walk(temp_dir):
            if "upx.exe" in files:
                shutil.copy2(os.path.join(root, "upx.exe"), upx_path)
                print(f"[OK] UPX 下载并安装成功")
                return os.path.abspath(upx_path)
        
        print("[ERROR] 未找到 upx.exe")
        return None
        
    except Exception as e:
        print(f"[ERROR] UPX 下载失败: {e}")
        return None

def compress_with_upx(exe_path, upx_path):
    """使用UPX压缩exe文件"""
    if not upx_path or not os.path.exists(upx_path):
        print("[WARNING] UPX 不可用，跳过压缩")
        return False
    
    try:
        print("\n[INFO] 开始UPX压缩...")
        
        # 获取压缩前大小
        original_size = os.path.getsize(exe_path) / (1024 * 1024)
        print(f"[INFO] 原始文件大小: {original_size:.1f} MB")
        
        # 执行压缩
        # --best: 最佳压缩率
        # --lzma: 使用LZMA算法（压缩率更高）
        # --force: 强制压缩CFG保护的文件
        result = subprocess.run(
            [upx_path, "--best", "--lzma", "--force", exe_path],
            capture_output=True,
            text=True,
            timeout=120
        )
        
        if result.returncode == 0:
            # 获取压缩后大小
            compressed_size = os.path.getsize(exe_path) / (1024 * 1024)
            reduction = ((original_size - compressed_size) / original_size) * 100
            
            print(f"[OK] UPX压缩完成！")
            print(f"[INFO] 压缩后大小: {compressed_size:.1f} MB")
            print(f"[INFO] 压缩率: {reduction:.1f}%")
            print(f"[INFO] 节省空间: {original_size - compressed_size:.1f} MB")
            return True
        else:
            print(f"[ERROR] UPX压缩失败: {result.stderr}")
            return False
            
    except Exception as e:
        print(f"[ERROR] UPX压缩出错: {e}")
        return False

def build_executable():
    """打包可执行文件"""
    print("=" * 50)
    print("准星程序 v3.0.0 打包工具（含UPX压缩）")
    print("=" * 50)
    
    # 检查并安装UPX
    print("\n[INFO] 检查UPX压缩工具...")
    upx_path = check_upx()
    
    # 检查 PyInstaller
    try:
        import PyInstaller
        print(f"[OK] PyInstaller 已安装: {PyInstaller.__version__}")
    except ImportError:
        print("[ERROR] PyInstaller 未安装，正在安装...")
        subprocess.check_call([sys.executable, "-m", "pip", "install", "pyinstaller"])
        import PyInstaller
        print(f"[OK] PyInstaller 安装成功: {PyInstaller.__version__}")
    
    # 检查 PySide6
    try:
        import PySide6
        print(f"[OK] PySide6 已安装: {PySide6.__version__}")
    except ImportError:
        print("[ERROR] PySide6 未安装，正在安装...")
        subprocess.check_call([sys.executable, "-m", "pip", "install", "pyside6"])
        import PySide6
        print(f"[OK] PySide6 安装成功: {PySide6.__version__}")
    
    # 构建参数
    app_name = "小林の准星"
    main_script = "crosshair_pyside6.py"
    # 优先使用从 1024 图标生成的多尺寸 .ico，回退到 favicon.ico
    icon_path = os.path.join("FAV", "app.ico")
    if not os.path.exists(icon_path):
        icon_path = os.path.join("FAV", "favicon.ico")
    
    # PyInstaller 命令参数
    pyinstaller_args = [
        "--name", app_name,
        "--onefile",  # 打包成单个文件
        "--windowed",  # 无控制台窗口
        "--clean",  # 清理临时文件
        "--noconfirm",  # 不询问确认
        "--distpath", ".",  # 输出到当前目录
        "--workpath", "build",  # 临时构建目录
        # 打包 FAV 资源目录，供运行时加载窗口/托盘图标
        "--add-data", "FAV;FAV",
    ]
    
    # 如果有图标文件，添加图标参数
    if icon_path and os.path.exists(icon_path):
        pyinstaller_args.extend(["--icon", icon_path])
        print(f"[OK] 使用图标: {icon_path}")
    
    # 添加主脚本
    pyinstaller_args.append(main_script)
    
    # 执行打包
    print("\n[INFO] 开始打包...")
    try:
        subprocess.check_call([sys.executable, "-m", "PyInstaller"] + pyinstaller_args)
        print("[OK] 打包完成！")
    except subprocess.CalledProcessError as e:
        print(f"[ERROR] 打包失败: {e}")
        return False
    
    # 检查输出文件
    exe_name = f"{app_name}.exe"
    if os.path.exists(exe_name):
        file_size = os.path.getsize(exe_name) / (1024 * 1024)  # MB
        print(f"[OK] 生成文件: {exe_name} ({file_size:.1f} MB)")
    else:
        print("[ERROR] 未找到生成的可执行文件")
        return False
    
    # 使用UPX压缩
    compress_with_upx(exe_name, upx_path)
    
    # 清理构建文件
    if os.path.exists("build"):
        shutil.rmtree("build")
        print("[OK] 清理临时文件")
    
    # 创建发布包
    release_dir = f"准星程序_v3.0.0_发布包"
    if os.path.exists(release_dir):
        shutil.rmtree(release_dir)
    
    os.makedirs(release_dir)
    
    # 复制文件到发布包
    files_to_copy = [
        exe_name,
        "README.md",
        "安装使用说明.txt", 
        "拖动功能使用说明.md",
    ]
    
    print(f"\n[INFO] 创建发布包: {release_dir}")
    for file in files_to_copy:
        if os.path.exists(file):
            shutil.copy2(file, os.path.join(release_dir, file))
            print(f"[OK] 复制: {file}")
        else:
            print(f"[WARNING] 文件不存在: {file}")
    
    # 创建版本信息文件
    version_info = f"""准星程序 v3.0.0 版本信息

发布时间: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}

新功能:
- 新增"控件不透明度"滑块，可统一调节卡片/输入框/按钮的背景透明度，文字保持清晰
- 英文应用标题更新为 Roland's Crosshair

改进:
- 背景图裁剪选区拖到图片边缘时保持尺寸不再被压缩
- 形状下拉框选项名称随语言切换显示（不再显示变量名）
- 补全快捷键、位置、保存当前配置、主题等标签的英文本地化

文件说明:
- 准星程序.exe: 主程序文件
- README.md: 程序说明文档
- 安装使用说明.txt: 安装和使用指南
- 拖动功能使用说明.md: 拖动功能详细说明

作者: B站：林晓CCC
"""
    
    with open(os.path.join(release_dir, "版本说明_v3.0.0.txt"), "w", encoding="utf-8") as f:
        f.write(version_info)
    
    print("[OK] 创建版本说明文件")
    
    # 压缩发布包
    zip_name = "准星程序_v3.0.0.zip"
    if os.path.exists(zip_name):
        os.remove(zip_name)
    
    print(f"\n[INFO] 压缩发布包: {zip_name}")
    shutil.make_archive(release_dir[:-4], "zip", release_dir)
    
    if os.path.exists(zip_name):
        zip_size = os.path.getsize(zip_name) / (1024 * 1024)  # MB
        print(f"[OK] 压缩完成: {zip_name} ({zip_size:.1f} MB)")
    
    print(f"\n[SUCCESS] 准星程序 v3.0.0 打包完成！")
    print(f"[INFO] 发布包位置: {os.path.abspath(release_dir)}")
    print(f"[INFO] 压缩包位置: {os.path.abspath(zip_name)}")
    
    return True

if __name__ == "__main__":
    # 切换到脚本所在目录
    script_dir = os.path.dirname(os.path.abspath(__file__))
    os.chdir(script_dir)
    
    # 执行打包
    success = build_executable()
    
    if success:
        print("\n[SUCCESS] 打包成功！可以分发使用了。")
    else:
        print("\n[ERROR] 打包失败，请检查错误信息。")
        sys.exit(1)
