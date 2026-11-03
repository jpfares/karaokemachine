//! Background work for an immediate-mode UI.
//!
//! Every long operation (network fetches, child processes, offline analysis)
//! runs on its own thread and is polled once per frame. Two shapes exist:
//!
//! - [`Job<T>`]: one answer, `Result<T, String>`, delivered once.
//! - [`StreamJob<M>`]: a sequence of messages (progress, then a final one).
//!
//! Both hold a clone of the `egui::Context` so the worker can request a
//! repaint whenever it has something to say, and once more when it exits.
//! Without that, a result that arrives while the window is idle sits unseen
//! until the next input event.
//!
//! A worker that dies without answering (a panic, or a dropped sender) is
//! reported as an error rather than leaving the UI waiting forever.

use eframe::egui;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

/// Handed to a worker thread. Each `send` wakes the UI, and so does dropping
/// the sender, which happens on every exit path including a panic.
pub struct JobSender<M> {
    tx: Sender<M>,
    ctx: egui::Context,
}

impl<M> JobSender<M> {
    /// Send one message. A closed receiver (the UI dropped the job) is not an
    /// error for the worker; it simply has nobody left to tell.
    pub fn send(&self, message: M) {
        let _ = self.tx.send(message);
        self.ctx.request_repaint();
    }
}

impl<M> Drop for JobSender<M> {
    fn drop(&mut self) {
        self.ctx.request_repaint();
    }
}

fn spawn<M: Send + 'static>(
    ctx: &egui::Context,
    work: impl FnOnce(JobSender<M>) + Send + 'static,
) -> Receiver<M> {
    let (tx, rx) = mpsc::channel();
    let sender = JobSender { tx, ctx: ctx.clone() };
    thread::spawn(move || work(sender));
    rx
}

/// A background task that produces one `Result<T, String>`.
pub struct Job<T> {
    name: &'static str,
    rx: Receiver<Result<T, String>>,
}

impl<T: Send + 'static> Job<T> {
    /// Run `work` on a new thread. `name` is used in the error shown if the
    /// worker dies without answering, so it should read as a subject:
    /// "The map browser", "The metadata helper".
    pub fn run(
        name: &'static str,
        ctx: &egui::Context,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Self {
        let rx = spawn(ctx, move |sender| sender.send(work()));
        Self { name, rx }
    }

    /// Non-blocking. `None` while the worker is still running; otherwise the
    /// worker's answer, or an error if it went away without one. Once this
    /// returns `Some`, the caller should drop the job.
    pub fn poll(&mut self) -> Option<Result<T, String>> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(format!("{} stopped unexpectedly.", self.name)))
            }
        }
    }
}

/// Everything a [`StreamJob`] had queued since the last drain.
pub struct Drained<M> {
    pub messages: Vec<M>,
    /// The worker has exited (its sender is gone). No more messages will come.
    pub ended: bool,
}

/// A background task that reports progress as a stream of messages.
pub struct StreamJob<M> {
    name: &'static str,
    rx: Receiver<M>,
}

impl<M: Send + 'static> StreamJob<M> {
    /// Run `work` on a new thread with a sender for its messages.
    pub fn run(
        name: &'static str,
        ctx: &egui::Context,
        work: impl FnOnce(JobSender<M>) + Send + 'static,
    ) -> Self {
        Self { name, rx: spawn(ctx, work) }
    }

    /// Non-blocking. Returns every queued message and whether the worker has
    /// exited. Process the messages first, then check `ended`: a worker that
    /// sends its final message and exits shows up as both in one drain. When
    /// `ended` is true the caller should drop the job; if it has not seen a
    /// final message, [`Self::death_notice`] describes why.
    pub fn drain(&mut self) -> Drained<M> {
        let mut messages = Vec::new();
        let ended = loop {
            match self.rx.try_recv() {
                Ok(message) => messages.push(message),
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };
        Drained { messages, ended }
    }

    /// Error text for a worker that exited without sending a final message.
    pub fn death_notice(&self) -> String {
        format!("{} stopped unexpectedly.", self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait<T>(mut poll: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(value) = poll() {
                return value;
            }
            assert!(Instant::now() < deadline, "job did not finish in time");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn job_delivers_its_answer() {
        let ctx = egui::Context::default();
        let mut job = Job::run("The test job", &ctx, || Ok(3));
        assert_eq!(wait(|| job.poll()), Ok(3));
    }

    #[test]
    fn job_reports_a_worker_that_died() {
        let ctx = egui::Context::default();
        let mut job: Job<u8> = Job::run("The test job", &ctx, || panic!("boom"));
        assert_eq!(
            wait(|| job.poll()),
            Err("The test job stopped unexpectedly.".to_string())
        );
    }

    #[test]
    fn stream_job_drains_messages_then_ends() {
        let ctx = egui::Context::default();
        let mut job = StreamJob::run("The test stream", &ctx, |sender| {
            sender.send(1);
            sender.send(2);
        });
        let mut seen = Vec::new();
        // `wait` returns only once a drain reports `ended`, or panics.
        wait(|| {
            let drained = job.drain();
            seen.extend(drained.messages);
            drained.ended.then_some(())
        });
        assert_eq!(seen, vec![1, 2]);
        assert_eq!(job.death_notice(), "The test stream stopped unexpectedly.");
    }
}
