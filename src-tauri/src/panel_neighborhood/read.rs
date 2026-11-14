use super::{Request, Response};
use crate::storage::Storage;

/// 只能在后台持有 storage 锁时调用，返回值仍须经 Coordinator 的版本校验。
pub fn read(storage: &Storage, request: Request) -> Response {
    match request {
        Request::Views(request) => {
            let ids: Vec<_> = request.ids.iter().map(String::as_str).collect();
            let result = storage.panel_items(&ids).map_err(|error| error.to_string());
            Response::Views { request, result }
        }
        Request::Validate { version, ids } => {
            let ids: Vec<_> = ids.iter().map(String::as_str).collect();
            let result = storage
                .existing_panel_ids(&ids)
                .map_err(|error| error.to_string());
            Response::Validated { version, result }
        }
    }
}
