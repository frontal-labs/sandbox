//! CPU accounting for a command. See ADR 0033.

use std::time::Duration;

/// The CPU time of the calling thread, from the thread CPU clock.
pub fn thread_cpu() -> Duration {
    let time = rustix::time::clock_gettime(rustix::time::ClockId::ThreadCPUTime);
    Duration::new(
        u64::try_from(time.tv_sec).unwrap_or(0),
        u32::try_from(time.tv_nsec).unwrap_or(0),
    )
}

/// Runs a future and totals the CPU time spent in its `poll` calls. A command is polled
/// on whichever thread the runtime picks, so the total is taken per poll, not from one
/// thread's clock at the start and the end. Time spent waiting, such as on the
/// scheduler, is not CPU time and is not counted.
pub struct CpuMeter<F> {
    inner: std::pin::Pin<Box<F>>,
    cpu: Duration,
}

impl<F: std::future::Future> CpuMeter<F> {
    pub fn new(inner: F) -> Self {
        CpuMeter {
            inner: Box::pin(inner),
            cpu: Duration::ZERO,
        }
    }
}

impl<F: std::future::Future> std::future::Future for CpuMeter<F> {
    type Output = (F::Output, Duration);

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        let before = thread_cpu();
        let polled = this.inner.as_mut().poll(cx);
        this.cpu += thread_cpu().saturating_sub(before);
        match polled {
            std::task::Poll::Ready(output) => std::task::Poll::Ready((output, this.cpu)),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}
