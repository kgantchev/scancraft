//! The job runner (§6.4): one scheduler thread with a priority queue, a rayon pool for the pixel
//! work, a cancellation token on every job, and a channel back to the engine.
//!
//! The scheduler keeps at most as many jobs running as the pool has threads, so a high-priority job
//! that arrives while the pool is busy is next in line, not queued behind lower ones inside rayon.
//! A job that panics comes back as an error for its page (§6.8: the last-resort guard).

use crate::error::EngineError;
use scancraft_core::recipe::{PageId, PhotoId};
use std::cmp::Ordering as CmpOrdering;
use std::collections::BinaryHeap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

/// The four priorities of §6.4, highest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobPriority {
    /// The preview of the page on screen.
    Preview,
    /// Detection for upcoming photos.
    Detection,
    /// Full-resolution renders.
    Render,
    /// Text recognition.
    Recognition,
}

impl JobPriority {
    fn rank(self) -> u8 {
        match self {
            JobPriority::Preview => 3,
            JobPriority::Detection => 2,
            JobPriority::Render => 1,
            JobPriority::Recognition => 0,
        }
    }
}

impl PartialOrd for JobPriority {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for JobPriority {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.rank().cmp(&other.rank())
    }
}

/// A job's cancellation flag. Changing a recipe cancels the jobs that became stale.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> CancellationToken {
        CancellationToken::default()
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// `Err(Cancelled)` once cancelled, for a job to check between its stages.
    pub fn check(&self) -> Result<(), EngineError> {
        if self.is_cancelled() { Err(EngineError::Cancelled) } else { Ok(()) }
    }
}

/// What a job is for, so its result can be matched to the recipe it was made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JobMeta {
    pub page: Option<PageId>,
    pub photo: Option<PhotoId>,
    /// The recipe's generation when the job was made; a result from an older one is dropped.
    pub generation: u64,
    /// Identifies the request, so the engine can tell when it is no longer in flight.
    pub tag: u64,
}

pub type JobWork<T> = Box<dyn FnOnce(&CancellationToken) -> Result<T, EngineError> + Send + 'static>;

pub struct Job<T> {
    pub priority: JobPriority,
    pub token: CancellationToken,
    pub meta: JobMeta,
    pub work: JobWork<T>,
}

/// A finished job.
pub struct JobDone<T> {
    pub meta: JobMeta,
    pub outcome: Result<T, EngineError>,
}

struct Queued<T> {
    priority: JobPriority,
    seq: u64,
    job: Job<T>,
}

impl<T> PartialEq for Queued<T> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.seq == other.seq
    }
}

impl<T> Eq for Queued<T> {}

impl<T> PartialOrd for Queued<T> {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Queued<T> {
    /// The greatest pops first: the highest priority, and among equals the earliest submitted.
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.priority.cmp(&other.priority).then_with(|| other.seq.cmp(&self.seq))
    }
}

struct State<T> {
    heap: BinaryHeap<Queued<T>>,
    running: usize,
    seq: u64,
    shutdown: bool,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    wake: Condvar,
}

impl<T> Shared<T> {
    fn lock(&self) -> std::sync::MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The scheduler thread and the rayon pool. When they can't be started the runner works inline: a
/// job runs on the thread that submits it, so the app still works, only less smoothly.
pub struct JobRunner<T: Send + 'static> {
    shared: Arc<Shared<T>>,
    results: Receiver<JobDone<T>>,
    inline_tx: Sender<JobDone<T>>,
    scheduler: Option<JoinHandle<()>>,
}

impl<T: Send + 'static> JobRunner<T> {
    /// Starts the scheduler and a pool of `threads` workers (at least one).
    pub fn new(threads: usize) -> JobRunner<T> {
        let threads = threads.max(1);
        let shared =
            Arc::new(Shared { state: Mutex::new(State { heap: BinaryHeap::new(), running: 0, seq: 0, shutdown: false }), wake: Condvar::new() });
        let (tx, results) = channel();
        let inline_tx = tx.clone();
        let started =
            rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("scancraft-pool-{i}")).build().ok().and_then(|pool| {
                let pool = Arc::new(pool);
                let shared = shared.clone();
                std::thread::Builder::new().name("scancraft-scheduler".to_string()).spawn(move || schedule(&shared, &pool, threads, &tx)).ok()
            });
        JobRunner { shared, results, inline_tx, scheduler: started }
    }

    /// Queues a job.
    pub fn submit(&self, job: Job<T>) {
        if self.scheduler.is_none() {
            // No scheduler: run it here, unless it is already cancelled.
            let Job { token, meta, work, .. } = job;
            let outcome = if token.is_cancelled() {
                Err(EngineError::Cancelled)
            } else {
                match catch_unwind(AssertUnwindSafe(|| work(&token))) {
                    Ok(outcome) => outcome,
                    Err(_) => Err(EngineError::msg("the page could not be prepared because of an internal error")),
                }
            };
            let _ = self.inline_tx.send(JobDone { meta, outcome });
            return;
        }
        let mut state = self.shared.lock();
        state.seq += 1;
        let seq = state.seq;
        state.heap.push(Queued { priority: job.priority, seq, job });
        self.shared.wake.notify_all();
    }

    /// Takes every finished result, without blocking.
    pub fn drain(&self) -> Vec<JobDone<T>> {
        self.results.try_iter().collect()
    }

    /// Queued and running jobs.
    pub fn pending(&self) -> usize {
        let state = self.shared.lock();
        state.heap.len() + state.running
    }

    /// Waits up to `timeout` for the queue to empty and returns whether it did (for tests and shutdown).
    pub fn wait_idle(&self, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while self.pending() > 0 {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        true
    }
}

impl<T: Send + 'static> Drop for JobRunner<T> {
    fn drop(&mut self) {
        {
            let mut state = self.shared.lock();
            state.shutdown = true;
            // Jobs still queued will never run: cancel them so anything holding their tokens knows.
            for q in state.heap.drain() {
                q.job.token.cancel();
            }
            self.shared.wake.notify_all();
        }
        if let Some(handle) = self.scheduler.take() {
            let _ = handle.join();
        }
    }
}

fn schedule<T: Send + 'static>(shared: &Arc<Shared<T>>, pool: &Arc<rayon::ThreadPool>, limit: usize, tx: &Sender<JobDone<T>>) {
    loop {
        let job = {
            let mut state = shared.lock();
            loop {
                if state.shutdown {
                    return;
                }
                if state.running < limit
                    && let Some(next) = state.heap.pop()
                {
                    if next.job.token.is_cancelled() {
                        // Stale before it started: it never runs; the result only says so, so whoever
                        // is waiting for it can stop.
                        let _ = tx.send(JobDone { meta: next.job.meta, outcome: Err(EngineError::Cancelled) });
                        continue;
                    }
                    state.running += 1;
                    break next.job;
                }
                state = shared.wake.wait(state).unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        let (shared, tx) = (shared.clone(), tx.clone());
        pool.spawn(move || {
            let Job { token, meta, work, .. } = job;
            let outcome = match catch_unwind(AssertUnwindSafe(|| work(&token))) {
                Ok(outcome) => outcome,
                // The payload can carry a file name, so it is not kept or printed.
                Err(_) => Err(EngineError::msg("the page could not be prepared because of an internal error")),
            };
            let _ = tx.send(JobDone { meta, outcome });
            let mut state = shared.lock();
            state.running = state.running.saturating_sub(1);
            shared.wake.notify_all();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;

    fn job(priority: JobPriority, n: u32) -> Job<u32> {
        Job {
            priority,
            token: CancellationToken::new(),
            meta: JobMeta { generation: u64::from(n), ..JobMeta::default() },
            work: Box::new(move |_| Ok(n)),
        }
    }

    /// A job that holds the single worker until `release` is sent.
    fn blocker(release: std::sync::mpsc::Receiver<()>) -> Job<u32> {
        Job {
            priority: JobPriority::Preview,
            token: CancellationToken::new(),
            meta: JobMeta::default(),
            work: Box::new(move |_| {
                let _ = release.recv_timeout(Duration::from_secs(10));
                Ok(0)
            }),
        }
    }

    fn order(runner: &JobRunner<u32>) -> Vec<u32> {
        assert!(runner.wait_idle(Duration::from_secs(10)));
        runner.drain().into_iter().filter_map(|d| d.outcome.ok()).filter(|n| *n != 0).collect()
    }

    #[test]
    fn previews_run_before_detection_renders_and_recognition() {
        let runner = JobRunner::<u32>::new(1);
        let (release, held) = sync_channel(1);
        runner.submit(blocker(held));
        std::thread::sleep(Duration::from_millis(50));
        // Submitted lowest first; they must run highest first, equal priorities in arrival order.
        runner.submit(job(JobPriority::Recognition, 1));
        runner.submit(job(JobPriority::Render, 2));
        runner.submit(job(JobPriority::Detection, 3));
        runner.submit(job(JobPriority::Render, 4));
        runner.submit(job(JobPriority::Preview, 5));
        release.send(()).unwrap();
        assert_eq!(order(&runner), vec![5, 3, 2, 4, 1]);
    }

    #[test]
    fn a_cancelled_job_does_not_run() {
        let runner = JobRunner::<u32>::new(1);
        let (release, held) = sync_channel(1);
        runner.submit(blocker(held));
        std::thread::sleep(Duration::from_millis(50));
        let stale = job(JobPriority::Render, 7);
        let token = stale.token.clone();
        runner.submit(stale);
        runner.submit(job(JobPriority::Render, 8));
        token.cancel();
        release.send(()).unwrap();
        assert_eq!(order(&runner), vec![8]);
        assert!(token.is_cancelled() && token.check() == Err(EngineError::Cancelled));
    }

    #[test]
    fn a_job_cancelled_before_it_ran_still_reports_that_it_did_not() {
        let runner = JobRunner::<u32>::new(1);
        let (release, held) = sync_channel(1);
        runner.submit(blocker(held));
        std::thread::sleep(Duration::from_millis(50));
        let stale = job(JobPriority::Render, 7);
        stale.token.cancel();
        runner.submit(stale);
        release.send(()).unwrap();
        assert!(runner.wait_idle(Duration::from_secs(10)));
        let done = runner.drain();
        let cancelled: Vec<_> = done.iter().filter(|d| d.outcome == Err(EngineError::Cancelled)).collect();
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0].meta.generation, 7, "the report carries the job's meta, so its request can be cleared");
    }

    #[test]
    fn a_running_job_sees_its_cancellation() {
        let runner = JobRunner::<u32>::new(2);
        let (started_tx, started) = sync_channel(1);
        let token = CancellationToken::new();
        let seen = token.clone();
        runner.submit(Job {
            priority: JobPriority::Render,
            token,
            meta: JobMeta::default(),
            work: Box::new(move |t| {
                started_tx.send(()).ok();
                for _ in 0..2000 {
                    t.check()?;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(1)
            }),
        });
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        seen.cancel();
        assert!(runner.wait_idle(Duration::from_secs(10)));
        let done = runner.drain();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].outcome, Err(EngineError::Cancelled));
    }

    #[test]
    fn an_escaped_panic_becomes_an_error_for_that_page() {
        let runner = JobRunner::<u32>::new(2);
        runner.submit(Job {
            priority: JobPriority::Render,
            token: CancellationToken::new(),
            meta: JobMeta { page: Some(PageId(4)), ..JobMeta::default() },
            work: Box::new(|_| {
                let v: Vec<u32> = Vec::new();
                #[allow(clippy::indexing_slicing)]
                Ok(v[3])
            }),
        });
        runner.submit(job(JobPriority::Render, 9));
        assert!(runner.wait_idle(Duration::from_secs(10)));
        let done = runner.drain();
        assert_eq!(done.len(), 2);
        let failed = done.iter().find(|d| d.meta.page == Some(PageId(4))).unwrap();
        let Err(EngineError::Message(m)) = &failed.outcome else { panic!("{:?}", failed.outcome) };
        assert!(m.contains("internal error"));
        // The runner keeps working afterwards.
        runner.submit(job(JobPriority::Render, 10));
        assert_eq!(order(&runner), vec![10]);
    }

    #[test]
    fn the_caller_is_never_blocked_by_a_job() {
        let runner = JobRunner::<u32>::new(1);
        let (release, held) = sync_channel(1);
        let started = std::time::Instant::now();
        runner.submit(blocker(held));
        runner.submit(job(JobPriority::Render, 1));
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(runner.pending(), 2);
        assert!(runner.drain().is_empty());
        release.send(()).unwrap();
        assert_eq!(order(&runner), vec![1]);
        assert_eq!(runner.pending(), 0);
    }

    #[test]
    fn dropping_the_runner_with_queued_work_does_not_hang() {
        let runner = JobRunner::<u32>::new(1);
        let (_release, held) = sync_channel(1);
        runner.submit(blocker(held));
        let queued = job(JobPriority::Render, 1);
        let token = queued.token.clone();
        runner.submit(queued);
        std::thread::sleep(Duration::from_millis(50));
        let t = std::time::Instant::now();
        drop(runner);
        assert!(t.elapsed() < Duration::from_secs(5));
        assert!(token.is_cancelled());
    }
}
