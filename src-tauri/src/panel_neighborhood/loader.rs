use super::{Key, Request, Response};
use anyhow::{anyhow, bail, Result};
use std::sync::{Arc, Mutex};

type Reader = dyn Fn(Request) -> Response + Send + Sync;
type Notify = dyn Fn() + Send + Sync;

#[derive(Default)]
struct Queue {
    pending: Option<Request>,
    wanted: Option<Key>,
    response: Option<Response>,
    running: bool,
    closed: bool,
    notified: bool,
}

impl Queue {
    fn clear(&mut self) {
        self.pending = None;
        self.wanted = None;
        self.response = None;
    }
}

struct Shared {
    queue: Mutex<Queue>,
    reader: Box<Reader>,
    notify: Box<Notify>,
}

/// 单实例最多一个在途读取、一个待处理请求和一个未消费响应。
pub struct Loader(Arc<Shared>);

impl Loader {
    pub fn new(
        reader: impl Fn(Request) -> Response + Send + Sync + 'static,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            reader: Box::new(reader),
            notify: Box::new(notify),
        }))
    }

    pub fn submit(&self, request: Request) -> Result<()> {
        let mut queue = self.0.queue.lock().map_err(|_| anyhow!("邻域队列不可用"))?;
        if queue.closed {
            bail!("邻域加载器已关闭");
        }
        queue.wanted = Some(request.key());
        queue.pending = Some(request);
        queue.response = None;
        if queue.running {
            return Ok(());
        }
        queue.running = true;
        drop(queue);
        let shared = self.0.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) = shared.run() {
                crate::diag::write(format!("panel neighborhood loader failed: {error}"));
            }
        });
        Ok(())
    }

    pub fn take_response(&self) -> Result<Option<Response>> {
        let mut queue = self.0.queue.lock().map_err(|_| anyhow!("邻域队列不可用"))?;
        queue.notified = false;
        Ok(queue.response.take())
    }

    /// 不打断已经执行的 SQL；清除待处理请求并拒绝在途结果。
    pub fn clear(&self) -> Result<()> {
        let mut queue = self.0.queue.lock().map_err(|_| anyhow!("邻域队列不可用"))?;
        queue.clear();
        Ok(())
    }

    pub fn close(&self) -> Result<()> {
        let mut queue = self.0.queue.lock().map_err(|_| anyhow!("邻域队列不可用"))?;
        queue.closed = true;
        queue.clear();
        Ok(())
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            crate::diag::write(format!("panel neighborhood loader close failed: {error}"));
        }
    }
}

impl Shared {
    fn next_request(&self) -> Result<Option<Request>> {
        let mut queue = self.queue.lock().map_err(|_| anyhow!("邻域队列不可用"))?;
        let next = queue.pending.take();
        if next.is_none() {
            queue.running = false;
        }
        Ok(next)
    }

    fn publish(&self, response: Response) -> Result<bool> {
        let mut queue = self.queue.lock().map_err(|_| anyhow!("邻域队列不可用"))?;
        if queue.closed || queue.wanted != Some(response.key()) {
            return Ok(false);
        }
        queue.response = Some(response);
        let notify = !queue.notified;
        queue.notified = true;
        Ok(notify)
    }

    fn run(&self) -> Result<()> {
        while let Some(request) = self.next_request()? {
            let response = (self.reader)(request);
            if self.publish(response)? {
                (self.notify)();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
