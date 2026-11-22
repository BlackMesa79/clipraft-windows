use crate::{diag, platform, settings::Theme};
use anyhow::{anyhow, Result};
use std::{cell::RefCell, mem::size_of};
use windows::{
    core::Interface,
    System::{DispatcherQueue, DispatcherQueueController},
    Win32::{
        Foundation::RECT,
        System::WinRT::{
            Composition::ICompositorDesktopInterop, CreateDispatcherQueueController,
            DispatcherQueueOptions, RoInitialize, RoUninitialize, DQTAT_COM_STA,
            DQTYPE_THREAD_CURRENT, RO_INIT_SINGLETHREADED,
        },
        UI::WindowsAndMessaging::GetClientRect,
    },
    UI::{
        Color,
        Composition::{Compositor, ContainerVisual, Desktop::DesktopWindowTarget, SpriteVisual},
    },
};

thread_local! {
    static CONTEXT: RefCell<Option<ThreadContext>> = const { RefCell::new(None) };
    static SURFACE: RefCell<Option<Surface>> = const { RefCell::new(None) };
}
const DEFAULT_BACKDROP_STRENGTH: u32 = 22;
const MAX_BACKDROP_STRENGTH: u32 = 100;
const MAX_BACKDROP_TRANSPARENCY: u32 = 36;

struct Runtime;

impl Runtime {
    fn new() -> Result<Self> {
        unsafe { RoInitialize(RO_INIT_SINGLETHREADED)? };
        Ok(Self)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

struct ThreadContext {
    _queue: Option<DispatcherQueueController>,
    _runtime: Runtime,
}

impl ThreadContext {
    fn new() -> Result<Self> {
        let runtime = Runtime::new()?;
        let queue = match DispatcherQueue::GetForCurrentThread() {
            Ok(_) => None,
            Err(_) => Some(unsafe {
                CreateDispatcherQueueController(DispatcherQueueOptions {
                    dwSize: size_of::<DispatcherQueueOptions>() as u32,
                    threadType: DQTYPE_THREAD_CURRENT,
                    apartmentType: DQTAT_COM_STA,
                })?
            }),
        };
        Ok(Self {
            _queue: queue,
            _runtime: runtime,
        })
    }
}

struct OwnedCompositor(Compositor);

impl Drop for OwnedCompositor {
    fn drop(&mut self) {
        if let Err(error) = self.0.Close() {
            diag::write(format!("main backdrop compositor close failed: {error}"));
        }
    }
}

struct Surface {
    raw: isize,
    target: DesktopWindowTarget,
    root: ContainerVisual,
    blur: SpriteVisual,
    tint: SpriteVisual,
    compositor: OwnedCompositor,
}

impl Surface {
    fn new(raw: isize, theme: Theme, strength: u32) -> Result<Self> {
        let compositor = OwnedCompositor(Compositor::new()?);
        let interop: ICompositorDesktopInterop = compositor.0.cast()?;
        let target = unsafe { interop.CreateDesktopWindowTarget(platform::hwnd(raw), false)? };
        let root = compositor.0.CreateContainerVisual()?;
        let blur = compositor.0.CreateSpriteVisual()?;
        blur.SetBrush(&compositor.0.CreateHostBackdropBrush()?)?;
        let tint = compositor.0.CreateSpriteVisual()?;
        tint.SetBrush(
            &compositor
                .0
                .CreateColorBrushWithColor(tint_color(theme, strength))?,
        )?;
        root.Children()?.InsertAtTop(&blur)?;
        root.Children()?.InsertAtTop(&tint)?;
        target.SetRoot(&root)?;
        let surface = Self {
            raw,
            target,
            root,
            blur,
            tint,
            compositor,
        };
        surface.resize()?;
        Ok(surface)
    }

    fn set_appearance(&self, theme: Theme, strength: u32) -> Result<()> {
        self.tint.SetBrush(
            &self
                .compositor
                .0
                .CreateColorBrushWithColor(tint_color(theme, strength))?,
        )?;
        Ok(())
    }

    fn resize(&self) -> Result<()> {
        let mut rect = RECT::default();
        unsafe { GetClientRect(platform::hwnd(self.raw), &mut rect)? };
        let mut size = self.root.Size()?;
        size.X = (rect.right - rect.left) as f32;
        size.Y = (rect.bottom - rect.top) as f32;
        self.root.SetSize(size)?;
        self.blur.SetSize(size)?;
        self.tint.SetSize(size)?;
        Ok(())
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        if let Err(error) = self
            .target
            .SetRoot(None::<&windows::UI::Composition::Visual>)
        {
            diag::write(format!("main backdrop detach failed: {error}"));
        }
        if let Err(error) = self.target.Close() {
            diag::write(format!("main backdrop target close failed: {error}"));
        }
    }
}

fn tint_color(theme: Theme, strength: u32) -> Color {
    let base_alpha = match theme {
        Theme::Light => 34,
        Theme::Dark => 54,
    };
    let remaining_tint =
        MAX_BACKDROP_STRENGTH * MAX_BACKDROP_STRENGTH - strength * MAX_BACKDROP_TRANSPARENCY;
    let reference_tint =
        MAX_BACKDROP_STRENGTH * (MAX_BACKDROP_STRENGTH - DEFAULT_BACKDROP_STRENGTH);
    let alpha = base_alpha * remaining_tint / reference_tint;
    match theme {
        Theme::Light => Color {
            A: alpha as u8,
            R: 248,
            G: 251,
            B: 249,
        },
        Theme::Dark => Color {
            A: alpha as u8,
            R: 21,
            G: 34,
            B: 31,
        },
    }
}

pub(super) fn create(raw: isize, theme: Theme, strength: u32) -> Result<()> {
    CONTEXT.with(|context| -> Result<()> {
        let mut context = context.borrow_mut();
        if context.is_none() {
            *context = Some(ThreadContext::new()?);
        }
        Ok(())
    })?;
    let surface = Surface::new(raw, theme, strength)?;
    SURFACE.with(|active| *active.borrow_mut() = Some(surface));
    Ok(())
}

pub(super) fn active(raw: isize) -> bool {
    SURFACE.with(|active| {
        active
            .borrow()
            .as_ref()
            .is_some_and(|surface| surface.raw == raw)
    })
}

pub(super) fn set_appearance(theme: Theme, strength: u32) -> Result<()> {
    SURFACE.with(|active| {
        let active = active.borrow();
        active
            .as_ref()
            .ok_or_else(|| anyhow!("主窗口 HostBackdrop 不可用"))?
            .set_appearance(theme, strength)
    })
}

pub(super) fn clear(raw: isize) {
    SURFACE.with(|active| {
        let matches = active
            .borrow()
            .as_ref()
            .is_some_and(|surface| surface.raw == raw);
        if matches {
            active.borrow_mut().take();
        }
    });
}

pub(super) fn resize(raw: isize) -> Result<()> {
    SURFACE.with(|active| {
        let active = active.borrow();
        let Some(surface) = active.as_ref().filter(|surface| surface.raw == raw) else {
            return Ok(());
        };
        surface.resize()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        core::w,
        Win32::{
            Foundation::{HWND, POINT},
            Graphics::{
                Dwm::{
                    DwmGetWindowAttribute, DwmSetWindowAttribute, DWMSBT_NONE, DWMWA_BORDER_COLOR,
                    DWMWA_COLOR_DEFAULT, DWMWA_COLOR_NONE, DWMWA_SYSTEMBACKDROP_TYPE,
                    DWMWA_USE_HOSTBACKDROPBRUSH, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
                },
                Gdi::ClientToScreen,
            },
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetWindowRect, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
                WS_POPUP, WS_THICKFRAME,
            },
        },
    };

    struct HiddenWindow(windows::Win32::Foundation::HWND);

    impl Drop for HiddenWindow {
        fn drop(&mut self) {
            clear(self.0 .0 as isize);
            unsafe { DestroyWindow(self.0).expect("hidden probe window should be destroyed") };
        }
    }

    #[test]
    fn maximum_backdrop_strength_matches_previous_sixty_percent_tint() {
        assert_eq!(tint_color(Theme::Light, MAX_BACKDROP_STRENGTH).A, 27);
        assert_eq!(tint_color(Theme::Dark, MAX_BACKDROP_STRENGTH).A, 44);
        assert_eq!(tint_color(Theme::Light, 50).A, 35);
        assert_eq!(tint_color(Theme::Dark, 50).A, 56);
    }

    fn client_insets(window: HWND) -> [i32; 4] {
        let mut origin = POINT::default();
        let mut outer = RECT::default();
        let mut client = RECT::default();
        unsafe {
            assert!(ClientToScreen(window, &mut origin).as_bool());
            GetWindowRect(window, &mut outer).expect("hidden window bounds");
            GetClientRect(window, &mut client).expect("hidden client bounds");
        }
        [
            origin.x - outer.left,
            origin.y - outer.top,
            outer.right - origin.x - client.right,
            outer.bottom - origin.y - client.bottom,
        ]
    }

    fn corner_preference(window: HWND) -> i32 {
        let mut preference = 0_i32;
        unsafe {
            DwmGetWindowAttribute(
                window,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&mut preference as *mut i32).cast(),
                size_of::<i32>() as u32,
            )
            .expect("读取主窗 DWM 圆角设置");
        }
        preference
    }

    #[test]
    #[ignore = "需要 Windows 11 和 WebView2，仅验证隐藏主窗的原生状态与尺寸"]
    fn hidden_main_frame_survives_wallpaper_and_skin_switches() {
        let app = tauri::Builder::default()
            .any_thread()
            .build(tauri::generate_context!())
            .unwrap();
        let profile = std::env::temp_dir().join(format!(
            "clipraft-wallpaper-frame-{}-{}",
            std::process::id(),
            crate::storage::unix_millis()
        ));
        let window = tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default())
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .visible(false)
            .focused(false)
            .data_directory(profile)
            .build()
            .unwrap();
        let handle = crate::platform::hwnd(window.hwnd().unwrap().0 as isize);
        let raw = handle.0 as isize;
        super::super::prepare_frame(raw);
        let frame_insets = [0; 4];
        assert_eq!(client_insets(handle), frame_insets);
        assert_eq!(corner_preference(handle), DWMWCP_ROUND.0);

        let mut settings = crate::settings::Settings::default();
        let mut width = 640;
        for skin in [
            crate::settings::Skin::Clipraft,
            crate::settings::Skin::Fluent,
        ] {
            settings.skin = skin;
            for has_wallpaper in [false, true, false] {
                super::super::sync_main(&window, &settings, has_wallpaper);
                window
                    .set_size(tauri::PhysicalSize::new(width, 460))
                    .unwrap();
                assert_eq!(client_insets(handle), frame_insets);
                assert_eq!(corner_preference(handle), DWMWCP_ROUND.0);
                width += 24;
            }
        }
        clear(raw);
        assert!(!window.is_visible().unwrap());
        window.destroy().unwrap();
    }

    #[test]
    #[ignore = "Windows 原生背景画笔隐藏窗口探测，不验证真实视觉效果"]
    fn hidden_host_backdrop_probe() {
        let window = HiddenWindow(unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("ClipRaft host backdrop probe"),
                WS_POPUP | WS_THICKFRAME,
                0,
                0,
                420,
                360,
                None,
                None,
                None,
                None,
            )
            .expect("hidden probe window should be created")
        });
        unsafe {
            DwmSetWindowAttribute(
                window.0,
                DWMWA_SYSTEMBACKDROP_TYPE,
                (&DWMSBT_NONE.0 as *const i32).cast(),
                size_of::<i32>() as u32,
            )
            .expect("system backdrop should be disabled for the host brush");
            DwmSetWindowAttribute(
                window.0,
                DWMWA_USE_HOSTBACKDROPBRUSH,
                (&1_i32 as *const i32).cast(),
                size_of::<i32>() as u32,
            )
            .expect("host backdrop brush should be enabled");
        }
        let raw = window.0 .0 as isize;
        create(raw, Theme::Light, DEFAULT_BACKDROP_STRENGTH)
            .expect("host backdrop surface should be created");
        super::super::set(window.0, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE as i32)
            .expect("native window border should be disabled");
        assert!(active(raw));
        resize(raw).expect("hidden surface should resize");
        set_appearance(Theme::Dark, 40).expect("hidden surface tint should update");
        clear(raw);
        assert!(!active(raw));
        let mut backdrop = -1_i32;
        unsafe {
            DwmGetWindowAttribute(
                window.0,
                DWMWA_SYSTEMBACKDROP_TYPE,
                (&mut backdrop as *mut i32).cast(),
                size_of::<i32>() as u32,
            )
            .expect("system backdrop should remain disabled");
        }
        assert_eq!(backdrop, DWMSBT_NONE.0);
        super::super::set(window.0, DWMWA_BORDER_COLOR, DWMWA_COLOR_DEFAULT as i32)
            .expect("native window border should be restored");
        println!("host_backdrop_graph=created theme=updated resize=passed cleanup=passed visual_verified=false");
    }
}
