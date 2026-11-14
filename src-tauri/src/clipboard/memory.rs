//! HGLOBAL 所有权与解锁；Win32 的零返回值不能套用普通 BOOL 成功规则。
use anyhow::{bail, Result};
use windows::Win32::{
    Foundation::{GetLastError, SetLastError, ERROR_SUCCESS, HGLOBAL},
    System::Memory::{GlobalAlloc, GlobalLock, GMEM_MOVEABLE},
};

// windows 0.61 的包装把 GlobalUnlock 的正常最终解锁判为 Err，
// GlobalFree 的空返回才表示成功。直接保留原生返回值再按各自契约判断。
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GlobalUnlock"]
    fn native_unlock(memory: HGLOBAL) -> i32;
    #[link_name = "GlobalFree"]
    fn native_free(memory: HGLOBAL) -> HGLOBAL;
}

pub(super) struct Allocation(pub(super) HGLOBAL);
impl Drop for Allocation {
    fn drop(&mut self) {
        if self.0 .0.is_null() {
            return;
        }
        if let Err(error) = unsafe { free_memory(self.0) } {
            crate::diag::write(format!("clipboard memory release failed: {error}"));
        }
    }
}

pub(super) fn allocate(bytes: &[u8]) -> Result<Allocation> {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))?;
        let owned = Allocation(memory);
        let pointer = GlobalLock(memory);
        if pointer.is_null() {
            bail!("剪贴板内存分配失败");
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
        unlock(memory);
        Ok(owned)
    }
}

pub(super) unsafe fn unlock(memory: HGLOBAL) {
    if let Err(error) = unlock_memory(memory) {
        crate::diag::write(format!("clipboard memory unlock failed: {error}"));
    }
}

unsafe fn unlock_memory(memory: HGLOBAL) -> Result<()> {
    SetLastError(ERROR_SUCCESS);
    if native_unlock(memory) != 0 {
        return Ok(());
    }
    let error = GetLastError();
    if error == ERROR_SUCCESS {
        return Ok(());
    }
    Err(windows::core::Error::from_hresult(error.to_hresult()).into())
}

unsafe fn free_memory(memory: HGLOBAL) -> Result<()> {
    if native_free(memory).0.is_null() {
        return Ok(());
    }
    Err(windows::core::Error::from_win32().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_unlock_and_free_succeed_without_false_errors() {
        unsafe {
            let memory = GlobalAlloc(GMEM_MOVEABLE, 8).unwrap();
            assert!(!GlobalLock(memory).is_null());
            assert!(!GlobalLock(memory).is_null());
            unlock_memory(memory).unwrap();
            unlock_memory(memory).unwrap();
            free_memory(memory).unwrap();
        }
    }
    #[test]
    fn allocated_bytes_remain_readable_until_ownership_is_released() {
        let bytes = [0, 1, 127, 255];
        let allocation = allocate(&bytes).unwrap();
        unsafe {
            let pointer = GlobalLock(allocation.0);
            assert!(!pointer.is_null());
            assert_eq!(
                std::slice::from_raw_parts(pointer.cast::<u8>(), bytes.len()),
                bytes
            );
            unlock_memory(allocation.0).unwrap();
        }
        drop(allocation);
    }
}
