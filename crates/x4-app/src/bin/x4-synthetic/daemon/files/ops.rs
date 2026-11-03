//! The file lists of a `torrent_set` and of a `torrent_add`, as Transmission 4.1 reads them.

use serde_json::Value;

use super::super::refusal::{FILE_INDEX_OUT_OF_RANGE, Refusal};
use super::FileState;

/// What a list of file indices in a `torrent_set` selects.
enum Which {
    /// An empty list: every file of the torrent, as the daemon reads it.
    All,
    /// The integers of the list; entries of any other type are skipped, as the daemon skips them.
    Listed(Vec<i64>),
}

/// What a list of a `torrent_set` does to the files it selects.
#[derive(Debug, Clone, Copy)]
enum Step {
    Unwanted,
    Wanted,
    High,
    Low,
    Normal,
}

/// The file lists of a `torrent_set`, in the order the daemon applies them: unwanted, wanted,
/// then the priorities high, low and normal, so that a later list wins for a file in both. A
/// `torrent_add` applies the priorities in the order low, normal and high.
pub(in super::super) struct FileOps {
    steps: Vec<(Step, Which)>,
}

impl FileOps {
    const KEYS: [(&'static str, Step); 5] = [
        ("files_unwanted", Step::Unwanted),
        ("files_wanted", Step::Wanted),
        ("priority_high", Step::High),
        ("priority_low", Step::Low),
        ("priority_normal", Step::Normal),
    ];

    /// The lists of a `torrent_add`, in the order its priorities are applied.
    const ADD_KEYS: [(&'static str, Step); 5] = [
        ("files_unwanted", Step::Unwanted),
        ("files_wanted", Step::Wanted),
        ("priority_low", Step::Low),
        ("priority_normal", Step::Normal),
        ("priority_high", Step::High),
    ];

    /// The file lists in the `params` of a `torrent_set`.
    ///
    /// # Errors
    ///
    /// A refusal for a list that is not a list.
    pub(in super::super) fn of(params: Option<&Value>) -> Result<Self, Refusal> {
        let mut steps = Vec::new();
        for (key, step) in Self::KEYS {
            let Some(value) = params.and_then(|params| params.get(key)) else {
                continue;
            };
            let list = value
                .as_array()
                .ok_or_else(|| Refusal::invalid(format!("{key} must be a list of file indices")))?;
            let which = if list.is_empty() {
                Which::All
            } else {
                Which::Listed(list.iter().filter_map(Value::as_i64).collect())
            };
            steps.push((step, which));
        }
        Ok(Self { steps })
    }

    /// The file lists in the `params` of a `torrent_add` of a torrent with `file_count` files.
    ///
    /// The daemon is more lenient here than in a `torrent_set`. A list that is not a list is not
    /// there, an empty one selects nothing instead of every file, and an index outside the files
    /// is ignored: the rest of an unwanted or wanted list holds, and a list of priorities with
    /// such an index is ignored as a whole. None of it is an error.
    pub(in super::super) fn for_add(params: Option<&Value>, file_count: usize) -> Self {
        let mut steps = Vec::new();
        for (key, step) in Self::ADD_KEYS {
            let Some(list) = params
                .and_then(|params| params.get(key))
                .and_then(Value::as_array)
            else {
                continue;
            };
            let listed: Vec<i64> = list.iter().filter_map(Value::as_i64).collect();
            let inside =
                |index: &i64| usize::try_from(*index).is_ok_and(|index| index < file_count);
            let kept: Vec<i64> = if matches!(step, Step::Unwanted | Step::Wanted) {
                listed.iter().copied().filter(inside).collect()
            } else if listed.iter().all(inside) {
                listed.clone()
            } else {
                Vec::new()
            };
            if kept.len() != listed.len() {
                tracing::debug!(
                    list = key,
                    ignored = listed.len() - kept.len(),
                    "the synthetic daemon ignored file indices outside the torrent"
                );
            }
            if !kept.is_empty() {
                steps.push((step, Which::Listed(kept)));
            }
        }
        Self { steps }
    }

    /// Whether there is no list to apply.
    pub(in super::super) fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Checks the indices against a torrent of `file_count` files.
    ///
    /// The daemon fails the call at the first list with an index out of range, after it has
    /// applied the lists before it. This one is stricter: the caller checks every torrent before
    /// it changes any, so that a call that fails changes nothing.
    ///
    /// # Errors
    ///
    /// Error 6, "file index out of range", for an index below 0 or not below `file_count`.
    pub(in super::super) fn check(&self, file_count: usize) -> Result<(), Refusal> {
        let outside = |index: &i64| {
            usize::try_from(*index)
                .ok()
                .is_none_or(|index| index >= file_count)
        };
        let any_outside = self.steps.iter().any(|(_, which)| match which {
            Which::All => false,
            Which::Listed(listed) => listed.iter().any(outside),
        });
        if any_outside {
            return Err(Refusal::daemon(FILE_INDEX_OUT_OF_RANGE, None, None));
        }
        Ok(())
    }

    /// Applies the lists to `state`, whose torrent has passed [`FileOps::check`].
    pub(super) fn apply(&self, state: &mut FileState) {
        for (step, which) in &self.steps {
            let indices: Vec<usize> = match which {
                Which::All => (0..state.file_count()).collect(),
                Which::Listed(listed) => listed
                    .iter()
                    .filter_map(|&index| usize::try_from(index).ok())
                    .collect(),
            };
            for index in indices {
                match step {
                    Step::Unwanted | Step::Wanted => {
                        if let Some(wanted) = state.wanted.get_mut(index) {
                            *wanted = matches!(step, Step::Wanted);
                        }
                    }
                    Step::High | Step::Low | Step::Normal => {
                        if let Some(priority) = state.priority.get_mut(index) {
                            *priority = match step {
                                Step::High => 1,
                                Step::Low => -1,
                                _ => 0,
                            };
                        }
                    }
                }
            }
        }
        state.refresh_needed();
    }
}
