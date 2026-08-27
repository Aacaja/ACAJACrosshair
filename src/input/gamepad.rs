//! 手柄输入：XInput 轮询线程（零依赖，直接链接 xinput1_4）。
//!
//! 检测左扳机（瞄准/Apex ADS）与右扳机（开火），状态变化才发事件。
//! 无手柄时自动降频轮询（500ms），插入即恢复 4ms 高频。
//!
//! XInput 不在 windows-rs 中，手动链接：
//! `XInputGetState(dwUserIndex: u32, pState: *mut XINPUT_STATE) -> u32`（ERROR_SUCCESS=0，ERROR_DEVICE_NOT_CONNECTED=1167）

use std::sync::{atomic::{AtomicBool, Ordering}, Arc, RwLock};

use crossbeam_channel::{Receiver, Sender, unbounded};
use log::{info, warn};

const ERROR_SUCCESS: u32 = 0;
const ERROR_DEVICE_NOT_CONNECTED: u32 = 1167;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XINPUT_GAMEPAD {
    pub wButtons: u16,
    pub bLeftTrigger: u8,
    pub bRightTrigger: u8,
    pub sThumbLX: i16,
    pub sThumbLY: i16,
    pub sThumbRX: i16,
    pub sThumbRY: i16,
    pub dwPaddingReserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XINPUT_STATE {
    pub dwPacketNumber: u32,
    pub Gamepad: XINPUT_GAMEPAD,
}

// SDK 提供 XInput.lib（转发到 xinput1_4.dll），因此链接名必须是 XInput
#[link(name = "XInput")]
unsafe extern "system" {
    fn XInputGetState(dwUserIndex: u32, pState: *mut XINPUT_STATE) -> u32;
}

/// 手柄事件
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameEvent {
    /// 瞄准状态变化：true=按下瞄准键（左扳机）
    Ads(bool),
    /// 开火状态变化：true=按下开火（右扳机）
    Fire(bool),
}

pub struct GamepadWatcher {
    pub events: Receiver<GameEvent>,
    stop: Arc<AtomicBool>,
}

impl Drop for GamepadWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// 瞄准触发源：扳机（模拟量）或肩键（数字量）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdsSource {
    LeftTrigger,
    RightTrigger,
    LeftBumper,
    RightBumper,
}

impl AdsSource {
    /// 从预设的 AdsButton 映射（v1.0.4 起支持肩键）
    pub fn from_config(b: crate::config::AdsButton) -> Self {
        match b {
            crate::config::AdsButton::LeftTrigger => AdsSource::LeftTrigger,
            crate::config::AdsButton::RightTrigger => AdsSource::RightTrigger,
            crate::config::AdsButton::LeftBumper => AdsSource::LeftBumper,
            crate::config::AdsButton::RightBumper => AdsSource::RightBumper,
        }
    }
}

// XINPUT_GAMEPAD 肩键位（v1.0.6 修正：0x0004/0x0008 是方向键 LEFT/RIGHT！
// 正确值：LB = 0x0100，RB = 0x0200）
const XINPUT_GAMEPAD_LEFT_SHOULDER: u16 = 0x0100;
const XINPUT_GAMEPAD_RIGHT_SHOULDER: u16 = 0x0200;

/// 运行时手柄配置（动态共享：UI 改设置即时生效，无需重启线程）
#[derive(Clone, Copy, Debug)]
pub struct RuntimeGamepadCfg {
    /// 扳机模拟量阈值 0-255
    pub threshold: u8,
    /// 瞄准触发键（支持 LB/RB）
    pub ads_source: AdsSource,
}

impl RuntimeGamepadCfg {
    pub fn from_preset(p: &crate::config::Preset) -> Self {
        RuntimeGamepadCfg {
            threshold: p.gamepad.trigger_threshold,
            ads_source: AdsSource::from_config(p.gamepad.ads_button),
        }
    }
}

/// 启动手柄轮询线程。配置经 `Arc<RwLock<RuntimeGamepadCfg>>` 动态读取（UI 改动即时生效）。
/// 轮询 8ms（125Hz）：CPU 占用约 0.5% 单核，ADS 延迟 <8ms 不可感知。
pub fn start_gamepad(cfg: Arc<RwLock<RuntimeGamepadCfg>>) -> GamepadWatcher {
    let (tx, rx): (Sender<GameEvent>, Receiver<GameEvent>) = unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();

    std::thread::Builder::new()
        .name("acaja-gamepad".into())
        .spawn(move || {
            let mut prev_ads = false;
            let mut prev_fire = false;
            let mut connected = false;
            let mut state = XINPUT_STATE::default();

            while !stop2.load(Ordering::SeqCst) {
                // 动态读取配置（轻量读锁）
                let cfg_now = *cfg.read().unwrap();
                let threshold = cfg_now.threshold.max(1);
                let ads_source = cfg_now.ads_source;

                let r = unsafe { XInputGetState(0, &mut state) };
                if r == ERROR_SUCCESS {
                    if !connected {
                        connected = true;
                        info!("手柄已连接（XInput 控制器 0）");
                    }
                    let pad = &state.Gamepad;
                    let ads = match ads_source {
                        AdsSource::LeftTrigger => pad.bLeftTrigger >= threshold,
                        AdsSource::RightTrigger => pad.bRightTrigger >= threshold,
                        AdsSource::LeftBumper => (pad.wButtons & XINPUT_GAMEPAD_LEFT_SHOULDER) != 0,
                        AdsSource::RightBumper => (pad.wButtons & XINPUT_GAMEPAD_RIGHT_SHOULDER) != 0,
                    };
                    let fire = pad.bRightTrigger >= threshold;

                    if ads != prev_ads {
                        prev_ads = ads;
                        let _ = tx.send(GameEvent::Ads(ads));
                    }
                    if fire != prev_fire {
                        prev_fire = fire;
                        let _ = tx.send(GameEvent::Fire(fire));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(8));
                } else if r == ERROR_DEVICE_NOT_CONNECTED {
                    if connected {
                        connected = false;
                        prev_ads = false;
                        prev_fire = false;
                        info!("手柄已拔出");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                } else {
                    warn!("XInputGetState 错误: {r}");
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        })
        .expect("spawn gamepad thread");

    GamepadWatcher { events: rx, stop }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_layout() {
        // XINPUT_GAMEPAD: 2+1+1+2+2+2+2+4 = 16 字节（dwPaddingReserved 补到 4 对齐，无额外填充）
        assert_eq!(std::mem::size_of::<XINPUT_GAMEPAD>(), 16);
        assert_eq!(std::mem::size_of::<XINPUT_STATE>(), 20);
        assert_eq!(std::mem::align_of::<XINPUT_STATE>(), 4);
    }

    #[test]
    fn trigger_threshold_math() {
        // 模拟触发判定逻辑（与轮询循环一致）
        let f = |trigger: u8, threshold: u8| trigger >= threshold;
        assert!(f(30, 30));
        assert!(f(255, 30));
        assert!(!f(29, 30));
        assert!(!f(0, 1));
    }
}