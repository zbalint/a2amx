use std::sync::atomic::Ordering;
use std::time::Duration;

use rustix::process::{Signal, kill_process_group};

use crate::harness::Harness;

use super::Session;

const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(10);
const EXIT_KEY_GAP: Duration = Duration::from_millis(300);

impl Session {
    pub(crate) async fn kill(&self, graceful: bool) -> anyhow::Result<()> {
        self.daemon_ended.store(true, Ordering::Release);
        if self.exit_code().is_some() {
            return Ok(());
        }
        if graceful && self.harness != Harness::Generic && self.graceful_exit().await {
            return Ok(());
        }
        if let Err(error) = kill_process_group(self.pid, Signal::HUP)
            && error != rustix::io::Errno::SRCH
        {
            return Err(error.into());
        }
        if tokio::time::timeout(Duration::from_secs(3), self.wait_exit())
            .await
            .is_err()
        {
            if !self.child_done.load(Ordering::Acquire)
                && let Err(error) = kill_process_group(self.pid, Signal::KILL)
                && error != rustix::io::Errno::SRCH
            {
                return Err(error.into());
            }
            self.wait_exit().await;
        }
        Ok(())
    }

    fn log_graceful(&self, outcome: &str, code: Option<&str>, error: Option<&anyhow::Error>) {
        match (code, error) {
            (Some(code), Some(error)) => tracing::info!(
                session = %self.id.0,
                outcome = outcome,
                code = code,
                %error,
                "graceful exit"
            ),
            (Some(code), None) => tracing::info!(
                session = %self.id.0,
                outcome = outcome,
                code = code,
                "graceful exit"
            ),
            (None, Some(error)) => tracing::info!(
                session = %self.id.0,
                outcome = outcome,
                %error,
                "graceful exit"
            ),
            (None, None) => tracing::info!(
                session = %self.id.0,
                outcome = outcome,
                "graceful exit"
            ),
        }
    }

    async fn graceful_exit(&self) -> bool {
        let _guard = match self.begin_reset() {
            Ok(guard) => guard,
            Err(error) => {
                self.log_graceful("skipped", Some(error.code), None);
                return false;
            }
        };
        // shortcut: a human can start typing between this gate check and the key;
        // re-check inside the input lock if a composer corruption is observed.
        if let Some((code, _)) = self.reset_gate_reason() {
            self.log_graceful("skipped", Some(code), None);
            return false;
        }
        let key = vec![0x04];
        match self.enqueue(key.clone()).await {
            Ok(true) => {}
            Ok(false) => {
                self.log_graceful("exited", None, None);
                return true;
            }
            Err(error) => {
                self.log_graceful("skipped", Some("input_closed"), Some(&error));
                return false;
            }
        }
        if matches!(self.harness, Harness::Claude | Harness::Agy) {
            // shortcut: fixed 300 ms against Claude and Agy's pending-exit window; use a
            // screen-based wait if either harness release shortens that window.
            tokio::time::sleep(EXIT_KEY_GAP).await;
            match self.enqueue(key).await {
                Ok(true) => {}
                Ok(false) => {
                    self.log_graceful("exited", None, None);
                    return true;
                }
                Err(error) => {
                    self.log_graceful("skipped", Some("input_closed"), Some(&error));
                    return false;
                }
            }
        }
        let exited = tokio::time::timeout(GRACEFUL_TIMEOUT, self.wait_exit())
            .await
            .is_ok();
        self.log_graceful(if exited { "exited" } else { "timeout" }, None, None);
        exited
    }

    pub(crate) async fn wait_exit(&self) {
        loop {
            let notified = self.notify().notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.exit_code().is_some() {
                return;
            }
            notified.await;
        }
    }
}
