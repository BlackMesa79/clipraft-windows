//! 只观察剪贴板元数据；枚举与来源查询绝不请求剪贴板数据。
#[cfg(test)]
use crate::clipboard_owner::exe_filename;
use crate::{clipboard_owner::clipboard_owner, diag};
use windows::Win32::System::DataExchange::{
    EnumClipboardFormats, GetClipboardFormatNameW, GetClipboardSequenceNumber,
};

fn standard_format_name(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "CF_TEXT",
        2 => "CF_BITMAP",
        3 => "CF_METAFILEPICT",
        4 => "CF_SYLK",
        5 => "CF_DIF",
        6 => "CF_TIFF",
        7 => "CF_OEMTEXT",
        8 => "CF_DIB",
        9 => "CF_PALETTE",
        10 => "CF_PENDATA",
        11 => "CF_RIFF",
        12 => "CF_WAVE",
        13 => "CF_UNICODETEXT",
        14 => "CF_ENHMETAFILE",
        15 => "CF_HDROP",
        16 => "CF_LOCALE",
        17 => "CF_DIBV5",
        0x80 => "CF_OWNERDISPLAY",
        0x81 => "CF_DSPTEXT",
        0x82 => "CF_DSPBITMAP",
        0x83 => "CF_DSPMETAFILEPICT",
        0x8e => "CF_DSPENHMETAFILE",
        0x200 => "CF_PRIVATEFIRST",
        0x2ff => "CF_PRIVATELAST",
        0x300 => "CF_GDIOBJFIRST",
        0x3ff => "CF_GDIOBJLAST",
        _ => return None,
    })
}

// 元数据中的换行、分隔符也需转义，避免伪造额外日志行。
fn log_name(name: &str) -> String {
    let mut escaped = String::new();
    for c in name.chars() {
        if matches!(c, '[' | ']' | ',') {
            escaped.extend(c.escape_unicode());
        } else if c.is_control() || c == '\\' {
            escaped.extend(c.escape_default());
        } else {
            escaped.push(c);
        }
    }
    escaped
}

fn format_name(id: u32) -> String {
    if let Some(name) = standard_format_name(id) {
        return name.into();
    }
    if (0xc000..=0xffff).contains(&id) {
        let mut buffer = [0u16; 256];
        let len = unsafe { GetClipboardFormatNameW(id, &mut buffer) }.max(0) as usize;
        if len > 0 {
            return log_name(&String::from_utf16_lossy(&buffer[..len]));
        }
    }
    "-".into()
}

pub fn clipboard_update() {
    if !diag::enabled() {
        return;
    }
    let sequence = unsafe { GetClipboardSequenceNumber() };
    let owner = clipboard_owner();
    diag::write(format!(
        "clipboard_update sequence={sequence} owner_exe={}",
        log_name(&owner.exe)
    ));
}

/// 默认不采样。采集结束（包括提前返回）时再输出实际读取的元数据。
#[derive(Default)]
pub struct CaptureLog {
    reads: Option<Vec<String>>,
}
impl CaptureLog {
    /// 调用方已成功打开剪贴板，且尚未读取任何数据。
    pub fn begin() -> Self {
        if !diag::enabled() {
            return Self::default();
        }
        let sequence = unsafe { GetClipboardSequenceNumber() };
        let owner = clipboard_owner();
        let mut formats = Vec::new();
        let mut id = 0;
        loop {
            id = unsafe { EnumClipboardFormats(id) };
            if id == 0 {
                break;
            }
            formats.push(format!("{id}:{}", format_name(id)));
        }
        diag::write(format!(
            "capture formats sequence={sequence} owner_pid={} owner_exe={} list=[{}]",
            owner.pid.map_or_else(|| "-".into(), |pid| pid.to_string()),
            log_name(&owner.exe),
            formats.join(", ")
        ));
        Self {
            reads: Some(Vec::new()),
        }
    }

    pub fn read(&mut self, id: u32, bytes: usize) {
        if let Some(reads) = &mut self.reads {
            reads.push(read_line(&format_name(id), bytes));
        }
    }

    pub fn empty(&mut self, id: u32) {
        self.read(id, 0);
    }
}
impl Drop for CaptureLog {
    fn drop(&mut self) {
        if let Some(reads) = self.reads.take() {
            for line in reads {
                diag::write(line);
            }
        }
    }
}

fn read_line(name: &str, bytes: usize) -> String {
    if bytes == 0 {
        format!("capture empty format={name}")
    } else {
        format!("capture read format={name} bytes={bytes}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_formats_have_constant_names() {
        let names = [
            "CF_TEXT",
            "CF_BITMAP",
            "CF_METAFILEPICT",
            "CF_SYLK",
            "CF_DIF",
            "CF_TIFF",
            "CF_OEMTEXT",
            "CF_DIB",
            "CF_PALETTE",
            "CF_PENDATA",
            "CF_RIFF",
            "CF_WAVE",
            "CF_UNICODETEXT",
            "CF_ENHMETAFILE",
            "CF_HDROP",
            "CF_LOCALE",
            "CF_DIBV5",
        ];
        for (i, name) in names.iter().enumerate() {
            assert_eq!(standard_format_name(i as u32 + 1), Some(*name));
        }
    }

    #[test]
    fn display_and_reserved_formats_have_constant_names() {
        for (id, name) in [
            (0x80, "CF_OWNERDISPLAY"),
            (0x81, "CF_DSPTEXT"),
            (0x82, "CF_DSPBITMAP"),
            (0x83, "CF_DSPMETAFILEPICT"),
            (0x8e, "CF_DSPENHMETAFILE"),
            (0x200, "CF_PRIVATEFIRST"),
            (0x2ff, "CF_PRIVATELAST"),
            (0x300, "CF_GDIOBJFIRST"),
            (0x3ff, "CF_GDIOBJLAST"),
        ] {
            assert_eq!(standard_format_name(id), Some(name));
        }
    }

    #[test]
    fn unknown_and_registered_formats_are_not_standard() {
        for id in [0, 18, 0x201, 0x301, 0xc000, 0xffff, u32::MAX] {
            assert_eq!(standard_format_name(id), None);
        }
    }

    #[test]
    fn exe_filename_strips_directories() {
        for path in [
            r"C:\Program Files\XnView\xnview.exe",
            r"\\server\share\xnview.exe",
            r"\\?\C:\apps\xnview.exe",
            "C:/apps/xnview.exe",
            "xnview.exe",
        ] {
            assert_eq!(exe_filename(Some(path)), "xnview.exe");
        }
        assert_eq!(exe_filename(Some(r"C:\程序\看图.exe")), "看图.exe");
    }

    #[test]
    fn exe_filename_unavailable_falls_back() {
        for path in [
            None,
            Some(""),
            Some("C:\\apps\\"),
            Some("/"),
            Some("."),
            Some(".."),
        ] {
            assert_eq!(exe_filename(path), "-");
        }
    }

    #[test]
    fn read_lines_distinguish_empty_and_actual_bytes() {
        assert_eq!(
            read_line("CF_DIBV5", 124),
            "capture read format=CF_DIBV5 bytes=124"
        );
        assert_eq!(read_line("CF_DIBV5", 0), "capture empty format=CF_DIBV5");
    }

    #[test]
    fn default_capture_log_collects_nothing() {
        let mut log = CaptureLog::default();
        log.read(13, 24);
        log.empty(17);
        assert!(log.reads.is_none());
    }

    #[test]
    fn enabled_capture_log_collects_only_format_and_size() {
        let mut log = CaptureLog {
            reads: Some(Vec::new()),
        };
        log.read(15, 96);
        log.empty(17);
        assert_eq!(
            log.reads.take().unwrap(),
            [
                "capture read format=CF_HDROP bytes=96",
                "capture empty format=CF_DIBV5",
            ]
        );
    }

    #[test]
    fn names_cannot_inject_log_lines() {
        assert_eq!(log_name("HTML Format"), "HTML Format");
        assert_eq!(log_name("[,]"), r"\u{5b}\u{2c}\u{5d}");
        assert_eq!(log_name("name\r\n\t"), r"name\r\n\t");
    }
}
