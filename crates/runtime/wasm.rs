use std::future::Future;
use std::time::Duration;

use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable, Either};
use futures::{pin_mut, FutureExt};

pub(crate) struct TaskHandle {
    abort: AbortHandle,
    completion: oneshot::Receiver<()>,
}

pub(crate) fn spawn(future: impl Future<Output = ()> + 'static) -> TaskHandle {
    let (abort, registration) = AbortHandle::new_pair();
    let (done_tx, done_rx) = oneshot::channel();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = Abortable::new(future, registration).await;
        let _ = done_tx.send(());
    });
    TaskHandle {
        abort,
        completion: done_rx,
    }
}

pub(crate) async fn timeout<F: Future>(duration: Duration, future: F) -> Result<F::Output, ()> {
    let future = future.fuse();
    let timer = futures_timer::Delay::new(duration).fuse();
    pin_mut!(future, timer);
    match futures::future::select(future, timer).await {
        Either::Left((value, _)) => Ok(value),
        Either::Right((_, _)) => Err(()),
    }
}

pub(crate) async fn sleep(duration: Duration) {
    futures_timer::Delay::new(duration).await;
}

impl TaskHandle {
    pub(crate) async fn shutdown(self, grace: Duration) {
        let TaskHandle { abort, completion } = self;
        let completion = completion.fuse();
        let timer = futures_timer::Delay::new(grace).fuse();
        pin_mut!(completion, timer);
        if matches!(
            futures::future::select(completion, timer).await,
            Either::Right(_)
        ) {
            abort.abort();
        }
    }
}
