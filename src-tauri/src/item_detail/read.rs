//! 图片资源读取在存储锁之外，返回前复查记录仍有效且隐私状态未改变。
use super::ItemDetail;
use crate::{model::ItemView, storage::Storage};
use anyhow::{anyhow, bail, Result};
use std::sync::Mutex;

pub(crate) fn display(storage: &Mutex<Storage>, id: &str) -> Result<(ItemView, ItemDetail)> {
    read(storage, id, Storage::display_detail)
}
pub(crate) fn reveal(storage: &Mutex<Storage>, id: &str) -> Result<(ItemView, ItemDetail)> {
    read(storage, id, Storage::reveal_detail)
}
fn read(
    storage: &Mutex<Storage>,
    id: &str,
    detail: fn(&Storage, &str) -> Result<ItemDetail>,
) -> Result<(ItemView, ItemDetail)> {
    let (item, mut content) = {
        let store = storage.lock().map_err(|_| anyhow!("存储不可用"))?;
        let item = store.preview_item(id)?;
        valid(&item)?;
        (item, detail(&store, id)?)
    };
    if let Some(rich) = &mut content.rich {
        rich.resolve_images();
    }
    let store = storage.lock().map_err(|_| anyhow!("存储不可用"))?;
    let latest = store.preview_item(id)?;
    verify_state(&item, &latest)?;
    Ok((latest, content))
}
fn valid(item: &ItemView) -> Result<()> {
    if item
        .expires_at
        .is_some_and(|expiry| expiry <= crate::storage::unix_millis())
    {
        bail!("记录已到期，请重新选择");
    }
    Ok(())
}
fn verify_state(original: &ItemView, latest: &ItemView) -> Result<()> {
    valid(latest)?;
    if original.sensitive != latest.sensitive || original.expires_at != latest.expires_at {
        bail!("记录隐私状态已变化，请重新选择");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_privacy_or_expiry_rejects_old_preview() {
        let original = ItemView::default();
        assert!(verify_state(
            &original,
            &ItemView {
                sensitive: true,
                ..Default::default()
            }
        )
        .is_err());
        assert!(verify_state(
            &original,
            &ItemView {
                expires_at: Some(crate::storage::unix_millis() + 60000),
                ..Default::default()
            }
        )
        .is_err());
        assert!(verify_state(
            &original,
            &ItemView {
                expires_at: Some(1),
                ..Default::default()
            }
        )
        .is_err());
        assert!(verify_state(&original, &original).is_ok());
    }
}
