//! 检查系统热键登记；不释放当前主热键，不保证识别其他软件的钩子或局部热键。
use crate::settings::{self, Hotkey, Shortcut};
use anyhow::{anyhow, Result};
use serde::Serialize;
use windows::Win32::{Foundation::HWND, UI::Input::KeyboardAndMouse::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Owned,
    Available,
    Conflict,
    Unavailable,
}
#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub status: Status,
    pub message: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub main: Finding,
    pub quick: Finding,
}
impl Report {
    pub fn validate(&self) -> Result<()> {
        let errors: Vec<_> = [("主窗口", &self.main), ("快速面板", &self.quick)]
            .into_iter()
            .filter(|(_, f)| matches!(f.status, Status::Conflict | Status::Unavailable))
            .map(|(name, f)| format!("{name}快捷键：{}", f.message))
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow!(errors.join("；")))
        }
    }
}
pub fn check_pair(
    main: Hotkey,
    quick: Hotkey,
    registered: Option<Hotkey>,
    mut probe: impl FnMut(Shortcut) -> Finding,
) -> Result<Report> {
    settings::validate_shortcut(main.shortcut())?;
    settings::validate_shortcut(quick.shortcut())?;
    settings::validate_hotkeys(main, quick)?;
    let mut check = |key: Hotkey| {
        if registered.is_some_and(|current| current.shortcut() == key.shortcut()) {
            Finding {
                status: Status::Owned,
                message: "当前由本应用注册".into(),
            }
        } else {
            probe(key.shortcut())
        }
    };
    Ok(Report {
        main: check(main),
        quick: check(quick),
    })
}
pub fn flags(shortcut: Shortcut) -> HOT_KEY_MODIFIERS {
    let mut flags = MOD_NOREPEAT.0;
    for (bit, flag) in [(1, MOD_CONTROL), (2, MOD_SHIFT), (4, MOD_ALT), (8, MOD_WIN)] {
        if shortcut.modifiers & bit != 0 {
            flags |= flag.0;
        }
    }
    HOT_KEY_MODIFIERS(flags)
}
pub enum ProbeError {
    Taken,
    System(u32),
}
pub trait ProbeApi {
    fn register(&mut self, key: Shortcut) -> std::result::Result<(), ProbeError>;
    fn unregister(&mut self) -> std::result::Result<(), u32>;
}
pub struct Probe<A: ProbeApi> {
    api: A,
    held: bool,
}
impl<A: ProbeApi> Probe<A> {
    pub fn check(&mut self, key: Shortcut) -> Finding {
        if self.held && !self.release() {
            return Finding {
                status: Status::Unavailable,
                message: "临时检测注册尚未释放，请重试检测".into(),
            };
        }
        match self.api.register(key) {
            Ok(()) => {
                self.held = true;
                if self.release() {
                    Finding {
                        status: Status::Available,
                        message: "未发现系统注册冲突".into(),
                    }
                } else {
                    Finding {
                        status: Status::Unavailable,
                        message: "临时检测注册未能释放，请重试检测".into(),
                    }
                }
            }
            Err(ProbeError::Taken) => Finding {
                status: Status::Conflict,
                message: "此组合已被系统或其他程序注册".into(),
            },
            Err(ProbeError::System(code)) => Finding {
                status: Status::Unavailable,
                message: format!("系统拒绝注册此组合（错误 0x{code:X}）"),
            },
        }
    }
    fn release(&mut self) -> bool {
        if self.held && self.api.unregister().is_err() {
            return false;
        }
        self.held = false;
        true
    }
}
impl<A: ProbeApi> Drop for Probe<A> {
    fn drop(&mut self) {
        let _ = self.release();
    }
}
pub struct NativeApi(HWND);
impl ProbeApi for NativeApi {
    fn register(&mut self, key: Shortcut) -> std::result::Result<(), ProbeError> {
        unsafe { RegisterHotKey(Some(self.0), 2, flags(key), key.key) }.map_err(|e| {
            if e.code() == windows::core::HRESULT::from_win32(1409) {
                ProbeError::Taken
            } else {
                ProbeError::System(e.code().0 as u32)
            }
        })
    }
    fn unregister(&mut self) -> std::result::Result<(), u32> {
        unsafe { UnregisterHotKey(Some(self.0), 2) }.map_err(|e| e.code().0 as u32)
    }
}
pub fn native(window: HWND) -> Probe<NativeApi> {
    Probe {
        api: NativeApi(window),
        held: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};
    #[derive(Default)]
    struct State {
        calls: Vec<&'static str>,
        register: VecDeque<std::result::Result<(), ProbeError>>,
        release: VecDeque<std::result::Result<(), u32>>,
    }
    struct Fake(Rc<RefCell<State>>);
    impl ProbeApi for Fake {
        fn register(&mut self, _key: Shortcut) -> std::result::Result<(), ProbeError> {
            let mut s = self.0.borrow_mut();
            s.calls.push("register");
            s.register.pop_front().unwrap_or(Ok(()))
        }
        fn unregister(&mut self) -> std::result::Result<(), u32> {
            let mut s = self.0.borrow_mut();
            s.calls.push("release");
            s.release.pop_front().unwrap_or(Ok(()))
        }
    }
    fn fixture() -> (Probe<Fake>, Rc<RefCell<State>>) {
        let s = Rc::new(RefCell::new(State::default()));
        (
            Probe {
                api: Fake(s.clone()),
                held: false,
            },
            s,
        )
    }
    #[test]
    fn own_registration_is_not_mistaken_for_conflict_or_released() {
        let mut seen = vec![];
        let owned = Hotkey::Custom {
            key: 86,
            modifiers: 4,
        };
        let report = check_pair(owned, Hotkey::CtrlShiftV, Some(Hotkey::AltV), |key| {
            seen.push(key);
            Finding {
                status: Status::Available,
                message: String::new(),
            }
        })
        .unwrap();
        assert_eq!(report.main.status, Status::Owned);
        assert_eq!(seen, vec![Hotkey::CtrlShiftV.shortcut()]);
        report.validate().unwrap();
    }
    #[test]
    fn occupied_and_system_failure_are_distinct_and_do_not_unregister_others() {
        let (mut probe, state) = fixture();
        state
            .borrow_mut()
            .register
            .extend([Err(ProbeError::Taken), Err(ProbeError::System(5))]);
        assert_eq!(
            probe.check(Hotkey::AltV.shortcut()).status,
            Status::Conflict
        );
        assert_eq!(
            probe.check(Hotkey::AltV.shortcut()).status,
            Status::Unavailable
        );
        drop(probe);
        assert_eq!(state.borrow().calls, vec!["register", "register"]);
    }
    #[test]
    fn successful_probe_releases_immediately() {
        let (mut probe, state) = fixture();
        assert_eq!(
            probe.check(Hotkey::AltV.shortcut()).status,
            Status::Available
        );
        drop(probe);
        assert_eq!(state.borrow().calls, vec!["register", "release"]);
    }
    #[test]
    fn cleanup_failure_retries_before_any_new_registration() {
        let (mut probe, state) = fixture();
        state.borrow_mut().release.extend([Err(5), Err(5), Ok(())]);
        assert_eq!(
            probe.check(Hotkey::AltV.shortcut()).status,
            Status::Unavailable
        );
        assert_eq!(
            probe.check(Hotkey::CtrlShiftV.shortcut()).status,
            Status::Unavailable
        );
        drop(probe);
        assert_eq!(
            state.borrow().calls,
            vec!["register", "release", "release", "release"]
        );
    }
    #[test]
    fn identical_modifiers_are_rejected_before_system_probe() {
        assert!(check_pair(
            Hotkey::AltV,
            Hotkey::Custom {
                key: 67,
                modifiers: 4
            },
            None,
            |_| panic!("内部冲突不应调用系统注册")
        )
        .is_err());
    }
    #[test]
    fn quick_conflict_blocks_saving_even_when_main_is_owned() {
        let report = check_pair(Hotkey::AltV, Hotkey::CtrlShiftV, Some(Hotkey::AltV), |_| {
            Finding {
                status: Status::Conflict,
                message: "已占用".into(),
            }
        })
        .unwrap();
        assert!(report
            .validate()
            .unwrap_err()
            .to_string()
            .contains("快速面板"));
    }
}
