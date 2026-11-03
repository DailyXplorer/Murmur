use super::MeetingMarkdownKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Finished {
    pub id: i64,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeetingName {
    pub title: Option<String>,
    pub started_at_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum OpenTarget {
    Meeting(i64),
    Meetings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    One {
        finished: Finished,
        name: MeetingName,
    },
    Batch {
        ready: usize,
        failed: usize,
    },
}

/// A nonempty batch. A retry replaces the outcome for the same meeting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    first: Finished,
    rest: Vec<Finished>,
}

impl Batch {
    pub fn new(finished: Finished) -> Self {
        Self {
            first: finished,
            rest: Vec::new(),
        }
    }

    pub fn merge(&mut self, finished: Finished) -> bool {
        let previous = std::iter::once(&mut self.first)
            .chain(self.rest.iter_mut())
            .find(|previous| previous.id == finished.id);
        if let Some(previous) = previous {
            let changed = *previous != finished;
            *previous = finished;
            changed
        } else {
            self.rest.push(finished);
            true
        }
    }

    pub fn single(&self) -> Option<Finished> {
        self.rest.is_empty().then_some(self.first)
    }

    pub fn without(&self, id: i64) -> Option<Self> {
        let mut kept = std::iter::once(self.first)
            .chain(self.rest.iter().copied())
            .filter(|finished| finished.id != id);
        Some(Self {
            first: kept.next()?,
            rest: kept.collect(),
        })
    }

    pub fn content(&self, name: impl FnOnce(i64) -> Option<MeetingName>) -> Option<Content> {
        if let Some(finished) = self.single() {
            return Some(Content::One {
                finished,
                name: name(finished.id)?,
            });
        }
        let ready = std::iter::once(&self.first)
            .chain(self.rest.iter())
            .filter(|finished| finished.outcome == Outcome::Ready)
            .count();
        Some(Content::Batch {
            ready,
            failed: 1 + self.rest.len() - ready,
        })
    }

    pub fn open_target(&self) -> OpenTarget {
        self.single().map_or(OpenTarget::Meetings, |finished| {
            OpenTarget::Meeting(finished.id)
        })
    }

    pub fn copy_target(&self) -> Option<(i64, MeetingMarkdownKind)> {
        self.single()
            .filter(|finished| finished.outcome == Outcome::Ready)
            .map(|finished| (finished.id, MeetingMarkdownKind::Transcript))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(id: i64, outcome: Outcome) -> Finished {
        Finished { id, outcome }
    }

    #[test]
    fn each_meeting_has_one_outcome_and_a_retry_replaces_it() {
        let failed = finished(1, Outcome::Failed);
        let ready = finished(1, Outcome::Ready);
        let mut batch = Batch::new(failed);
        assert!(!batch.merge(failed));
        assert!(batch.merge(ready));
        assert!(!batch.merge(ready));
        assert_eq!(batch.single(), Some(ready));
        assert_eq!(batch.open_target(), OpenTarget::Meeting(1));
    }

    #[test]
    fn batches_count_ready_mixed_and_failed_outcomes() {
        for (outcomes, expected) in [
            (
                [Outcome::Ready, Outcome::Ready],
                Content::Batch {
                    ready: 2,
                    failed: 0,
                },
            ),
            (
                [Outcome::Ready, Outcome::Failed],
                Content::Batch {
                    ready: 1,
                    failed: 1,
                },
            ),
            (
                [Outcome::Failed, Outcome::Failed],
                Content::Batch {
                    ready: 0,
                    failed: 2,
                },
            ),
        ] {
            let mut batch = Batch::new(finished(1, outcomes[0]));
            batch.merge(finished(2, outcomes[1]));
            assert_eq!(batch.content(|_| None), Some(expected));
            assert_eq!(batch.open_target(), OpenTarget::Meetings);
            assert_eq!(batch.copy_target(), None);
        }
    }

    #[test]
    fn a_single_meeting_uses_the_latest_title_without_changing_its_outcome() {
        let finished = finished(7, Outcome::Ready);
        let batch = Batch::new(finished);
        for title in [None, Some("Planning".to_string())] {
            let name = MeetingName {
                title,
                started_at_ms: 123,
            };
            assert_eq!(
                batch.content(|id| {
                    assert_eq!(id, 7);
                    Some(name.clone())
                }),
                Some(Content::One { finished, name })
            );
        }
    }

    #[test]
    fn only_one_ready_meeting_copies_and_it_always_copies_the_transcript() {
        let mut batch = Batch::new(finished(7, Outcome::Ready));
        assert_eq!(
            batch.copy_target(),
            Some((7, MeetingMarkdownKind::Transcript))
        );
        batch.merge(finished(8, Outcome::Ready));
        assert_eq!(batch.copy_target(), None);
        assert_eq!(Batch::new(finished(7, Outcome::Failed)).copy_target(), None);
    }

    #[test]
    fn removing_the_viewed_meeting_can_shrink_or_end_a_batch() {
        let ready = finished(1, Outcome::Ready);
        let mut batch = Batch::new(ready);
        assert_eq!(batch.without(1), None);
        assert_eq!(batch.without(2), Some(batch.clone()));
        batch.merge(finished(2, Outcome::Failed));
        assert_eq!(batch.without(2), Some(Batch::new(ready)));
        assert_eq!(
            batch.without(1).unwrap().open_target(),
            OpenTarget::Meeting(2)
        );
    }
}
