//! 监听线程与原生资源生命周期；部分初始化失败也按已取得资源清理。
use super::{
    broker_messages, wndproc, BrokerRequest, BROKER, HOTKEY_REQUESTS, LISTENER, SETTINGS_MESSAGE,
};
use crate::{
    app_events::record,
    diag, hook,
    settings::{self, Hotkey, HotkeyRegistrar},
};
use anyhow::{anyhow, Context, Result};
use std::{
    sync::{
        atomic::Ordering,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};
use windows::{
    core::w,
    Win32::{
        Foundation::{HINSTANCE, HWND},
        System::{
            DataExchange::{AddClipboardFormatListener, RemoveClipboardFormatListener},
            LibraryLoader::GetModuleHandleW,
            Threading::GetCurrentThreadId,
        },
        UI::{
            Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey},
            WindowsAndMessaging::*,
        },
    },
};

struct NativeRegistrar(HWND);
impl HotkeyRegistrar for NativeRegistrar {
    fn unregister(&mut self) -> Result<()> {
        unsafe {
            UnregisterHotKey(Some(self.0), 1).map_err(|e| anyhow!("旧快捷键注销失败：{e}"))?;
            Ok(())
        }
    }
    fn register(&mut self, hotkey: Hotkey) -> Result<()> {
        let shortcut = hotkey.shortcut();
        unsafe {
            RegisterHotKey(
                Some(self.0),
                1,
                crate::hotkey_check::flags(shortcut),
                shortcut.key,
            )?;
        }
        Ok(())
    }
}

const WINDOW_CLASS: windows::core::PCWSTR = w!("ClipboardM0Listener");
const TIMER_ID: usize = 1;
const TIMER_INTERVAL_MS: u32 = 250;

struct Listener {
    instance: HINSTANCE,
    window: HWND,
    listening: bool,
    timer: bool,
    registered: Option<Hotkey>,
}

pub(super) fn start(
    initial: Hotkey,
    startup: crate::capture_policy::StartupCapture,
) -> Result<(u32, Option<String>)> {
    let (requests, pending) = mpsc::channel();
    HOTKEY_REQUESTS
        .set(requests)
        .map_err(|_| anyhow!("热键控制通道已启动"))?;
    let (reply, wait) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("clipboard-listener".into())
        .spawn(move || run(initial, pending, reply, startup))
        .context("剪贴板监听线程创建失败")?;
    wait.recv().map_err(|_| anyhow!("剪贴板监听线程启动失败"))?
}

fn run(
    initial: Hotkey,
    pending: Receiver<BrokerRequest>,
    reply: SyncSender<Result<(u32, Option<String>)>>,
    startup: crate::capture_policy::StartupCapture,
) {
    let mut listener = match Listener::create(startup) {
        Ok(listener) => listener,
        Err(error) => {
            record("broker startup error reply failed", reply.send(Err(error)));
            return;
        }
    };
    let startup = listener.initialize(initial);
    let id = unsafe { GetCurrentThreadId() };
    if let Err(error) = reply.send(Ok((id, startup))) {
        diag::write(format!("broker startup reply failed: {error}"));
        return;
    }
    if let Err(error) = listener.messages(pending) {
        diag::write(format!("broker message loop failed: {error}"));
    }
}

impl Listener {
    fn create(startup: crate::capture_policy::StartupCapture) -> Result<Self> {
        let instance: HINSTANCE = unsafe { GetModuleHandleW(None)? }.into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: WINDOW_CLASS,
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err(windows::core::Error::from_win32()).context("监听窗口类注册失败");
        }
        let mut listener = Self {
            instance,
            window: HWND::default(),
            listening: false,
            timer: false,
            registered: None,
        };
        listener.open_window(startup)?;
        Ok(listener)
    }

    fn open_window(&mut self, startup: crate::capture_policy::StartupCapture) -> Result<()> {
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                WINDOW_CLASS,
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(self.instance),
                None,
            )
        }
        .context("监听窗口创建失败")?;
        self.window = window;
        startup.initialize_sequence(&crate::clipboard::LAST_SEQUENCE, || unsafe {
            windows::Win32::System::DataExchange::GetClipboardSequenceNumber()
        });
        if startup == crate::capture_policy::StartupCapture::ChangesOnly {
            diag::write("restored_startup skip_existing_clipboard=true".into());
        }
        unsafe { AddClipboardFormatListener(window) }.context("剪贴板监听注册失败")?;
        self.listening = true;
        if unsafe { SetTimer(Some(window), TIMER_ID, TIMER_INTERVAL_MS, None) } == 0 {
            return Err(anyhow!("剪贴板监听定时器创建失败"));
        }
        self.timer = true;
        LISTENER.store(window.0 as isize, Ordering::Release);
        BROKER.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
        Ok(())
    }

    fn initialize(&mut self, initial: Hotkey) -> Option<String> {
        let window = self.window;
        let error =
            settings::replace_hotkey(&mut NativeRegistrar(window), &mut self.registered, initial)
                .err()
                .map(|error| format!("{}：{error}。请在设置中更换快捷键。", initial.label()));
        if let Some(error) = &error {
            diag::write(format!("startup hotkey: {error}"));
        }
        error
    }

    fn messages(&mut self, pending: Receiver<BrokerRequest>) -> Result<()> {
        let window = self.window;
        let mut probe = crate::hotkey_check::native(window);
        let mut msg = MSG::default();
        loop {
            match unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 {
                -1 => return Err(windows::core::Error::from_win32()).context("监听消息读取失败"),
                0 => return Ok(()),
                _ => self.dispatch(&msg, &pending, &mut probe),
            }
        }
    }

    fn dispatch(
        &mut self,
        msg: &MSG,
        pending: &Receiver<BrokerRequest>,
        probe: &mut crate::hotkey_check::Probe<crate::hotkey_check::NativeApi>,
    ) {
        match msg.message {
            SETTINGS_MESSAGE => {
                while let Ok(request) = pending.try_recv() {
                    broker_messages::request(
                        request,
                        &mut NativeRegistrar(self.window),
                        &mut self.registered,
                        &mut |key| probe.check(key),
                    );
                }
            }
            hook::RECORD_MESSAGE => broker_messages::recording(msg),
            hook::ACTION_MESSAGE => broker_messages::action(msg),
            hook::HEALTH_MESSAGE => broker_messages::health(msg),
            _ => unsafe {
                // TranslateMessage 的 false 表示没有字符消息，并非失败。
                let _ = TranslateMessage(msg);
                DispatchMessageW(msg);
            },
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        BROKER.store(0, Ordering::Release);
        LISTENER.store(0, Ordering::Release);
        let window = self.window;
        if !window.is_invalid() {
            unsafe {
                if self.listening {
                    record(
                        "broker listener removal failed",
                        RemoveClipboardFormatListener(window),
                    );
                }
                if self.registered.is_some() {
                    record(
                        "broker hotkey removal failed",
                        UnregisterHotKey(Some(window), 1),
                    );
                }
                if self.timer {
                    record(
                        "broker timer removal failed",
                        KillTimer(Some(window), TIMER_ID),
                    );
                }
                record("broker window destruction failed", DestroyWindow(window));
            }
        }
        record("broker class removal failed", unsafe {
            UnregisterClassW(WINDOW_CLASS, Some(self.instance))
        });
    }
}
