use std::future::Future;
use std::time::Duration;

pub(crate) struct TaskHandle {
    inner: tokio::task::JoinHandle<()>,
}

pub(crate) fn spawn(future: impl Future<Output = ()> + Send + 'static) -> TaskHandle {
    TaskHandle {
        inner: tokio::spawn(future),
    }
}

pub(crate) async fn timeout<F: Future>(duration: Duration, future: F) -> Result<F::Output, ()> {
    tokio::time::timeout(duration, future).await.map_err(|_| ())
}

impl TaskHandle {
    pub(crate) async fn shutdown(mut self, grace: Duration) {
        tokio::select! {
            result = &mut self.inner => {
                let _ = result;
            }
            _ = tokio::time::sleep(grace) => {
                self.inner.abort();
                let _ = self.inner.await;
            }
        }
    }
}
