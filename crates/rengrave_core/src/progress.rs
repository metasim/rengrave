// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Progress reporting and cooperative cancellation for long calculations.

use crate::error::{Error, Result};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A handle passed to long-running operations.
///
/// Clones share one cancellation flag, so another thread can stop a running calculation.
#[derive(Clone, Default)]
pub struct Progress {
    canceled: Arc<AtomicBool>,
    callback: Option<Arc<dyn Fn(f64) + Send + Sync>>,
}

impl Progress {
    /// A handle with no observer. It can still be canceled.
    pub fn none() -> Self {
        Self::default()
    }

    /// A handle that calls `callback` with the completed fraction, in `0.0..=1.0`.
    pub fn with_callback(callback: impl Fn(f64) + Send + Sync + 'static) -> Self {
        Self {
            canceled: Arc::default(),
            callback: Some(Arc::new(callback)),
        }
    }

    /// A handle sharing this one's cancellation flag that reports to `callback`.
    pub fn observed(&self, callback: impl Fn(f64) + Send + Sync + 'static) -> Self {
        Self {
            canceled: self.canceled.clone(),
            callback: Some(Arc::new(callback)),
        }
    }

    /// Requests that the operation stop as soon as possible.
    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::Relaxed);
    }

    /// Returns `true` once [`Progress::cancel`] has been called on any clone.
    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Relaxed)
    }

    pub(crate) fn check_canceled(&self) -> Result<()> {
        if self.is_canceled() {
            Err(Error::Canceled)
        } else {
            Ok(())
        }
    }

    pub(crate) fn report(&self, fraction: f64) {
        if let Some(callback) = &self.callback {
            callback(fraction);
        }
    }
}

impl std::fmt::Debug for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Progress")
            .field("canceled", &self.is_canceled())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_and_observers_share_cancellation() {
        let original = Progress::none();
        let observed = original.observed(|_| {});
        let clone = observed.clone();
        assert!(!original.is_canceled());
        clone.cancel();
        assert!(original.is_canceled() && observed.is_canceled());
        assert!(observed.check_canceled().is_err());
    }

    #[test]
    fn reports_reach_the_callback() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let progress = Progress::with_callback(move |f| sink.lock().unwrap().push(f));
        progress.report(0.5);
        Progress::none().report(0.25);
        assert_eq!(*seen.lock().unwrap(), [0.5]);
    }
}
