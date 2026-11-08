use super::*;

impl QueueState {
    fn held(&self) -> bool {
        self.held_until.is_some() || self.offline.holds()
    }

    fn goes_through(&self, queued: &Queued, local_ready: bool) -> bool {
        !self.held()
            || queued.asked
            || (self.held_until.is_none() && local_ready && !queued.codex_only)
    }

    /// The meeting to process now or, when none may go, what to wait for.
    pub(super) fn take_next(&mut self, now: SystemTime, local_ready: bool) -> Next {
        self.held_until = self.held_until.filter(|until| now < *until);
        let index = self
            .queued
            .iter()
            .position(|queued| self.goes_through(queued, local_ready));
        if let Some(queued) = index.and_then(|index| self.queued.remove(index)) {
            self.local_pass = self.held() && !queued.asked;
            return Next::Process(queued.id);
        }
        if self.offline.due(now) {
            return Next::CheckConnection;
        }
        let ends = [self.held_until, self.offline.retry_at()];
        Next::Wait(ends.into_iter().flatten().min())
    }
}

impl ProcessingQueue {
    pub(super) fn new(paused: bool) -> Self {
        let state = QueueState {
            queued: VecDeque::new(),
            current: None,
            paused,
            held_until: None,
            rate_limit: RateLimitHold::default(),
            offline: OfflineHold::default(),
            local_pass: false,
        };
        Self {
            state: Mutex::new(state),
            changed: tokio::sync::Notify::new(),
            reachability: Box::new(CodexHost::new()),
            local_ready: Box::new(|| false),
        }
    }

    pub(super) fn state(&self) -> std::sync::MutexGuard<'_, QueueState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn enqueue(&self, id: i64) {
        self.push(id, false, false);
    }

    /// Queues a meeting the user just asked for, which a rate-limit hold
    /// does not keep waiting. Already queued, it keeps its place in line
    /// and goes through the hold.
    pub(super) fn enqueue_asked(&self, id: i64) {
        self.push(id, true, false);
    }

    /// Queues a meeting that an offline hold may keep waiting for the
    /// connection. `codex_only`: this Mac's model cannot do what is left.
    pub(super) fn requeue(&self, id: i64, codex_only: bool) {
        self.push(id, false, codex_only);
    }

    /// Meetings waiting for the connection may go on with this Mac's model.
    pub(super) fn local_model_ready(&self) {
        self.changed.notify_one();
    }

    fn push(&self, id: i64, asked: bool, codex_only: bool) {
        {
            let mut state = self.state();
            match state.queued.iter_mut().find(|queued| queued.id == id) {
                Some(queued) => {
                    queued.asked |= asked;
                    queued.codex_only |= codex_only;
                }
                None => state.queued.push_back(Queued {
                    id,
                    asked,
                    codex_only,
                }),
            }
        }
        // `notify_one` keeps a permit, so a worker between its check and its
        // wait still wakes; a stale permit only costs it one more check.
        self.changed.notify_one();
    }

    /// How often a held queue looks at the clock again: timers run on uptime,
    /// so after a sleep only a fresh look sees that the hold has ended.
    const RECHECK: Duration = Duration::from_secs(60);

    /// Waits for the next meeting to process, past any rate-limit hold and
    /// until the connection is back after Codex could not be reached.
    pub(super) async fn next(&self) -> i64 {
        loop {
            let changed = self.changed.notified();
            let now = SystemTime::now();
            let local_ready = (self.local_ready)();
            let next = self.state().take_next(now, local_ready);
            match next {
                Next::Process(id) => return id,
                Next::CheckConnection => {
                    let online = self.reachability.online().await;
                    if online {
                        info!("The connection is back; meetings waiting for it go on");
                    }
                    self.state().offline.checked(online, SystemTime::now());
                }
                Next::Wait(Some(until)) => {
                    let left = until.duration_since(now).unwrap_or_default();
                    tokio::select! {
                        () = tokio::time::sleep(left.min(Self::RECHECK)) => {}
                        () = changed => {}
                    }
                }
                Next::Wait(None) => changed.await,
            }
        }
    }

    /// Holds the meetings not asked for during `wait`.
    #[cfg(test)]
    pub(super) fn hold(&self, wait: Duration) {
        self.state().held_until = Some(SystemTime::now() + wait);
    }

    /// Holds or frees the queue from what Codex said while a meeting was
    /// processed, and returns the wait it starts. Only an answer lifts a hold: a
    /// pass that asked Codex nothing, such as re-analysing, says nothing
    /// about its cap.
    pub(super) fn codex_said(&self, verdict: CodexVerdict) -> Option<Duration> {
        let mut state = self.state();
        let local_pass = std::mem::take(&mut state.local_pass);
        match verdict {
            CodexVerdict::RateLimited => {
                // A refusal is an answer: the connection works.
                state.offline.reset();
                let wait = state.rate_limit.rate_limited();
                state.held_until = Some(SystemTime::now() + wait);
                Some(wait)
            }
            CodexVerdict::Unreachable => {
                let now = SystemTime::now();
                match state.offline.retry_at() {
                    // The hold already waits for the connection: a meeting
                    // this Mac's model took past it leaves the next check
                    // where it was.
                    Some(at) if local_pass => Some(at.duration_since(now).unwrap_or_default()),
                    _ => Some(state.offline.unreachable(now)),
                }
            }
            CodexVerdict::Answered => {
                state.offline.reset();
                state.rate_limit.reset();
                state.held_until = None;
                None
            }
            CodexVerdict::Unknown => None,
        }
    }

    /// Makes `id` the meeting in progress, unless the queue is paused: the
    /// meeting then waits, and `resume_processing` queues it again. Deciding
    /// under the lock `pause` takes means a meeting is either refused here
    /// or cancelled there, never sent after a pause.
    pub(super) fn begin(&self, id: i64, operation: &ProcessingOperation) -> bool {
        let mut state = self.state();
        if state.paused {
            return false;
        }
        state.current = Some((id, operation.clone()));
        true
    }

    pub(super) fn end(&self) {
        self.state().current = None;
    }

    /// How much longer a rate-limit hold keeps queued meeting `id` waiting.
    /// `None` when it is not waiting on one.
    pub(super) fn held_for(&self, id: i64) -> Option<Duration> {
        let state = self.state();
        let left = state.held_until?.duration_since(SystemTime::now()).ok()?;
        state
            .queued
            .iter()
            .any(|queued| queued.id == id && !queued.asked)
            .then_some(left)
    }

    /// Whether queued meeting `id` waits for the connection to come back.
    pub(super) fn offline_for(&self, id: i64) -> bool {
        let local_ready = (self.local_ready)();
        let state = self.state();
        state.offline.holds()
            && state
                .queued
                .iter()
                .any(|queued| queued.id == id && !state.goes_through(queued, local_ready))
    }

    pub(super) fn queued(&self) -> Vec<i64> {
        self.state().queued.iter().map(|queued| queued.id).collect()
    }

    /// The meetings queued or in progress, which may still read their audio.
    pub(super) fn busy(&self) -> HashSet<i64> {
        let state = self.state();
        state
            .queued
            .iter()
            .map(|queued| queued.id)
            .chain(state.current.as_ref().map(|(id, _)| *id))
            .collect()
    }

    /// Stops processing meeting `id`. What was saved stays, and processing
    /// resumes from there next time.
    pub(super) fn cancel(&self, id: i64) {
        if let Some((current, operation)) = &self.state().current {
            if *current == id {
                operation.cancel();
            }
        }
    }

    /// Stops the meeting in progress and holds every other one until
    /// `resume`.
    pub(super) fn pause(&self) {
        let mut state = self.state();
        state.paused = true;
        if let Some((_, operation)) = &state.current {
            operation.cancel();
        }
    }

    pub(super) fn resume(&self) {
        self.state().paused = false;
    }
}

/// Queues the meetings a quit (or turning meetings off) left mid-way.
pub(super) fn queue_waiting(queue: &ProcessingQueue, store: &MeetingsStore) {
    match store.waiting_for_processing() {
        Ok(ids) => ids
            .into_iter()
            .for_each(|id| queue.requeue(id, codex_only(store, id))),
        Err(error) => error!("Failed to list meetings to process: {error}"),
    }
}

/// Meeting `id` waits for its summary only, which this Mac's model cannot
/// write.
pub(super) fn codex_only(store: &MeetingsStore, id: i64) -> bool {
    store.get(id).ok().flatten().is_some_and(|meeting| {
        meeting.status == store::MeetingStatus::Ready
            && meeting.summary_status == store::SummaryStatus::Pending
    })
}
