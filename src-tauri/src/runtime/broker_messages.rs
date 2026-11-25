//! broker 请求与钩子消息分发；掩码在控制线程收到消息之前完成。
use super::{paste_generation, BrokerRequest, Control};
use crate::{
    app_events, diag, hook, paster,
    settings::{self, Hotkey, HotkeyRegistrar},
};
use anyhow::Result;
use std::sync::{atomic::Ordering, mpsc::Sender};
use windows::Win32::UI::WindowsAndMessaging::MSG;

pub(super) fn request(
    request: BrokerRequest,
    registrar: &mut impl HotkeyRegistrar,
    registered: &mut Option<Hotkey>,
    probe: &mut impl FnMut(crate::settings::Shortcut) -> crate::hotkey_check::Finding,
) {
    match request {
        BrokerRequest::Replace {
            hotkey,
            reply,
            cancelled,
        } => {
            if cancelled.load(Ordering::Acquire) {
                return;
            }
            let result = settings::replace_hotkey(registrar, registered, hotkey)
                .map_err(|error| error.to_string());
            app_events::record("broker replace reply failed", reply.send(result));
        }
        BrokerRequest::Check { main, quick, reply } => {
            let result = crate::hotkey_check::check_pair(main, quick, *registered, probe)
                .map_err(|error| error.to_string());
            app_events::record("broker check reply failed", reply.send(result));
        }
    }
}

pub(super) fn recording(msg: &MSG) {
    let session = msg.lParam.0 as u64;
    if session != hook::RECORD_SESSION.load(Ordering::Acquire) || !hook::recording() {
        diag::write(format!("hotkey_record stage=discard session={session}"));
        return;
    }
    diag::write(format!("hotkey_record stage=captured session={session}"));
    let masked = paster::mask_shortcut();
    let packed = if masked.is_ok() {
        msg.wParam.0 as u32
    } else {
        0
    };
    send(Control::RecordedHotkey(packed, session));
    if let Err(error) = masked {
        send(Control::Report(error.to_string()));
    }
}

pub(super) fn action(msg: &MSG) {
    current_action(msg, paste_generation, paster::mask_shortcut, send);
}

pub(super) fn health(msg: &MSG) {
    let message = if msg.wParam.0 == 1 {
        "键盘钩子已自动恢复"
    } else {
        "键盘钩子恢复失败，请重启应用后重试"
    };
    send(Control::Report(message.into()));
}

fn current_action(
    msg: &MSG,
    current: impl FnOnce() -> u64,
    mask: impl FnOnce() -> Result<()>,
    send: impl FnMut(Control),
) {
    let Some(action) = hook::Action::from_raw(msg.wParam.0) else {
        return;
    };
    let generation = msg.lParam.0 as u64;
    if generation != current() {
        return;
    }
    diag::write(format!(
        "hotkey_action action={action:?} generation={generation}"
    ));
    dispatch_action(action, generation, mask, send);
}

fn dispatch_action(
    action: hook::Action,
    generation: u64,
    mask: impl FnOnce() -> Result<()>,
    mut send: impl FnMut(Control),
) {
    if action == hook::Action::Open {
        if let Err(error) = mask() {
            send(Control::Hook(hook::Action::Cancel, generation));
            send(Control::Report(error.to_string()));
            return;
        }
    }
    send(Control::Hook(action, generation));
}

fn send(control: Control) {
    app_events::record(
        "broker control send failed",
        send_to(super::CONTROL.get(), control),
    );
}

fn send_to(sender: Option<&Sender<Control>>, control: Control) -> Result<(), String> {
    let sender = sender.ok_or_else(|| "控制通道未启动".to_string())?;
    app_events::send_control(sender, control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::{
        cell::RefCell,
        sync::{atomic::AtomicBool, mpsc, Arc},
    };
    use windows::Win32::Foundation::{LPARAM, WPARAM};

    struct RejectRegistrar;
    impl HotkeyRegistrar for RejectRegistrar {
        fn unregister(&mut self) -> Result<()> {
            panic!("取消请求不得注销热键")
        }
        fn register(&mut self, _: Hotkey) -> Result<()> {
            panic!("取消请求不得注册热键")
        }
    }

    #[test]
    fn cancelled_replace_does_not_register_or_reply() {
        let (reply, wait) = mpsc::channel();
        let mut registered = Some(Hotkey::AltV);
        request(
            BrokerRequest::Replace {
                hotkey: Hotkey::AltV,
                reply,
                cancelled: Arc::new(AtomicBool::new(true)),
            },
            &mut RejectRegistrar,
            &mut registered,
            &mut |_| panic!("不得检测"),
        );
        assert!(matches!(
            wait.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        assert_eq!(registered, Some(Hotkey::AltV));
    }

    #[test]
    fn stale_or_unknown_action_never_masks_or_dispatches() {
        for (action, generation) in [(hook::Action::Open as usize, 8), (usize::MAX, 9)] {
            let msg = MSG {
                wParam: WPARAM(action),
                lParam: LPARAM(generation),
                ..Default::default()
            };
            current_action(
                &msg,
                || 9,
                || panic!("不得发送掩码"),
                |_| panic!("不得调度"),
            );
        }
    }

    #[test]
    fn open_masks_before_dispatch_and_failure_cancels_before_report() {
        let events = RefCell::new(Vec::new());
        dispatch_action(
            hook::Action::Open,
            9,
            || {
                events.borrow_mut().push("mask");
                Ok(())
            },
            |control| {
                assert!(matches!(control, Control::Hook(hook::Action::Open, 9)));
                events.borrow_mut().push("open");
            },
        );
        assert_eq!(*events.borrow(), ["mask", "open"]);
        let mut controls = Vec::new();
        dispatch_action(
            hook::Action::Open,
            10,
            || Err(anyhow!("mask failed")),
            |control| controls.push(control),
        );
        assert!(
            matches!(controls.as_slice(), [Control::Hook(hook::Action::Cancel, 10), Control::Report(error)] if error == "mask failed")
        );
        dispatch_action(
            hook::Action::Cancel,
            11,
            || panic!("取消不掩码"),
            |control| {
                assert!(matches!(control, Control::Hook(hook::Action::Cancel, 11)));
            },
        );
    }

    #[test]
    fn missing_or_disconnected_control_is_reported() {
        assert!(send_to(None, Control::Toggle).is_err());
        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        assert_eq!(
            send_to(Some(&sender), Control::Toggle).unwrap_err(),
            "控制线程已退出"
        );
    }
}
