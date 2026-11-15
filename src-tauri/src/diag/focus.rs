//! 只读窗口身份；不读取窗口文本、输入法组合文本或剪贴板。
use super::*;
fn identity(raw: isize) -> String {
    unsafe {
        let window = platform::hwnd(raw);
        let mut class = [0_u16; 256];
        let count = GetClassNameW(window, &mut class).max(0) as usize;
        let mut pid = 0;
        let tid = GetWindowThreadProcessId(window, Some(&mut pid));
        let hkl = GetKeyboardLayout(tid);
        format!(
            "hwnd=0x{raw:X},class={:?},pid={pid},tid={tid},hkl={:X}",
            String::from_utf16_lossy(&class[..count]),
            hkl.0 as usize
        )
    }
}
pub fn snapshot(stage: &str, target: isize, generation: u64) {
    if !enabled() {
        return;
    }
    unsafe {
        let mut gui = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let tid = GetWindowThreadProcessId(platform::hwnd(target), None);
        let ok = GetGUIThreadInfo(tid, &mut gui).is_ok();
        write(format!("focus_probe stage={stage} generation={generation} target=[{}] focus=[{}] foreground=0x{:X} gui_ok={ok}",identity(target),identity(gui.hwndFocus.0 as isize),platform::foreground()));
    }
}
pub fn quick_window(stage: &str, raw: isize, generation: u64) {
    if !enabled() {
        return;
    }
    unsafe {
        let mut bounds = windows::Win32::Foundation::RECT::default();
        let ok = GetWindowRect(platform::hwnd(raw), &mut bounds).is_ok();
        write(format!("quick_window stage={stage} generation={generation} hwnd=0x{raw:X} visible={} rect_ok={ok} width={} height={} foreground=0x{:X}",IsWindowVisible(platform::hwnd(raw)).as_bool(),bounds.right-bounds.left,bounds.bottom-bounds.top,platform::foreground()));
    }
}
