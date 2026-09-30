//! A connection-owning thread keeps file syscalls and durability waits off Tokio.
//! Acceptance and message transitions return only after their transactions commit.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use tokio::sync::oneshot;

use crate::messaging::{Limits, MessageState};

#[derive(Debug)]
struct QueueFullMarker;

impl std::fmt::Display for QueueFullMarker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("message queue is full")
    }
}

impl std::error::Error for QueueFullMarker {}

#[derive(Debug, Clone)]
pub(crate) struct NewMessage {
    pub(crate) boot: String,
    pub(crate) sender_session: String,
    pub(crate) sender_address: String,
    pub(crate) recipient_session: String,
    pub(crate) recipient_address: String,
    pub(crate) subject: String,
    pub(crate) body: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Message {
    pub(crate) seq: i64,
    pub(crate) boot: String,
    pub(crate) sender_session: String,
    pub(crate) sender_address: String,
    pub(crate) recipient_session: String,
    pub(crate) recipient_address: String,
    pub(crate) subject: String,
    pub(crate) body: String,
    pub(crate) state: MessageState,
    pub(crate) detail: Option<String>,
    pub(crate) observed: bool,
}

#[derive(Debug)]
pub(crate) enum InsertError {
    QueueFull,
    Internal(anyhow::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CancelResult {
    Cancelled,
    NotCancellable(MessageState),
    Unknown,
}

#[derive(Clone)]
pub(crate) struct Store {
    inner: Arc<Inner>,
}

struct Inner {
    sender: Mutex<Option<std::sync::mpsc::Sender<Job>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

type JobFn = Box<dyn FnOnce(&mut Connection, &Limits, &Path) + Send + 'static>;

enum Job {
    Run(JobFn),
    Shutdown(oneshot::Sender<Result<()>>),
}

impl Store {
    pub(crate) async fn open(state_dir: PathBuf, limits: Limits) -> Result<Self> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let (ready_sender, ready_receiver) = oneshot::channel();
        let database = state_dir.join("messages.db");
        let thread_database = database.clone();
        let thread = thread::Builder::new()
            .name("a2amx-store".to_owned())
            .spawn(move || {
                let setup = open_database(&thread_database, limits);
                let (mut connection, database) = match setup {
                    Ok(value) => {
                        let _ = ready_sender.send(Ok(()));
                        value
                    }
                    Err(error) => {
                        let _ = ready_sender.send(Err(error));
                        return;
                    }
                };

                while let Ok(job) = receiver.recv() {
                    match job {
                        Job::Run(job) => job(&mut connection, &limits, &database),
                        Job::Shutdown(reply) => {
                            let result = ensure_secure_files(&database);
                            let _ = reply.send(result);
                            break;
                        }
                    }
                }
            })
            .context("spawn store thread")?;
        ready_receiver
            .await
            .map_err(|_| anyhow!("store thread exited while opening"))??;
        Ok(Self {
            inner: Arc::new(Inner {
                sender: Mutex::new(Some(sender)),
                thread: Mutex::new(Some(thread)),
            }),
        })
    }

    pub(crate) async fn close(&self) -> Result<()> {
        let sender = { lock(&self.inner.sender).take() };
        let shutdown_result = match sender {
            Some(sender) => {
                let (reply_sender, reply_receiver) = oneshot::channel();
                match sender.send(Job::Shutdown(reply_sender)) {
                    Ok(()) => reply_receiver
                        .await
                        .map_err(|_| anyhow!("store thread exited while closing"))
                        .and_then(|result| result),
                    Err(_) => Err(anyhow!("store thread has exited")),
                }
            }
            None => Ok(()),
        };

        let thread = { lock(&self.inner.thread).take() };
        let join_result = if let Some(thread) = thread {
            tokio::task::spawn_blocking(move || {
                thread.join().map_err(|_| anyhow!("store thread panicked"))
            })
            .await
            .map_err(|error| anyhow!("joining store thread: {error}"))?
        } else {
            Ok(())
        };
        if let Err(error) = shutdown_result {
            return match join_result {
                Ok(()) => Err(error),
                Err(join_error) => {
                    Err(error.context(format!("joining store thread: {join_error}")))
                }
            };
        }
        join_result
    }

    async fn run<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection, &Limits) -> Result<T> + Send + 'static,
    {
        let sender = lock(&self.inner.sender)
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("store is closed"))?;
        let (reply_sender, reply_receiver) = oneshot::channel();
        let job = Box::new(
            move |connection: &mut Connection, limits: &Limits, database: &Path| {
                // Check before mutating: a failure after commit would report an
                // internal error for a message that is already stored.
                if let Err(error) = ensure_secure_files(database) {
                    let _ = reply_sender.send(Err(error));
                    return;
                }
                let operation_result = operation(connection, limits);
                let permission_result = ensure_secure_files(database);
                let result = match operation_result {
                    Ok(value) => permission_result.map(|_| value),
                    Err(error) => {
                        let _ = permission_result;
                        Err(error)
                    }
                };
                let _ = reply_sender.send(result);
            },
        );
        sender
            .send(Job::Run(job))
            .map_err(|_| anyhow!("store thread has exited"))?;
        reply_receiver
            .await
            .map_err(|_| anyhow!("store thread exited"))?
    }

    pub(crate) async fn insert_message(
        &self,
        new: NewMessage,
    ) -> std::result::Result<i64, InsertError> {
        match self
            .run(move |connection, limits| insert_message(connection, limits, new))
            .await
        {
            Ok(sequence) => Ok(sequence),
            Err(error) if error.downcast_ref::<QueueFullMarker>().is_some() => {
                Err(InsertError::QueueFull)
            }
            Err(error) => Err(InsertError::Internal(error)),
        }
    }

    pub(crate) async fn next_pending(&self, boot: &str, session: &str) -> Result<Option<Message>> {
        let boot = boot.to_owned();
        let session = session.to_owned();
        self.run(move |connection, _| next_pending(connection, &boot, &session))
            .await
    }

    pub(crate) async fn begin_attempt(&self, seq: i64) -> Result<Option<i64>> {
        self.run(move |connection, _| begin_attempt(connection, seq))
            .await
    }

    pub(crate) async fn finish_attempt(
        &self,
        seq: i64,
        attempt: i64,
        outcome: &str,
        detail: Option<&str>,
    ) -> Result<()> {
        let outcome = outcome.to_owned();
        let detail = detail.map(str::to_owned);
        self.run(move |connection, _| {
            finish_attempt(connection, seq, attempt, &outcome, detail.as_deref())
        })
        .await
    }

    pub(crate) async fn record_receipt(&self, seq: i64) -> Result<bool> {
        self.run(move |connection, _| record_receipt(connection, seq))
            .await
    }

    pub(crate) async fn reject_attempt(&self, seq: i64) -> Result<bool> {
        self.run(move |connection, _| reject_attempt(connection, seq))
            .await
    }

    pub(crate) async fn fail_open(
        &self,
        boot: &str,
        session: Option<&str>,
        detail: &str,
    ) -> Result<()> {
        let boot = boot.to_owned();
        let session = session.map(str::to_owned);
        let detail = detail.to_owned();
        self.run(move |connection, _| fail_open(connection, &boot, session.as_deref(), &detail))
            .await
    }

    pub(crate) async fn cancel(&self, seq: i64) -> Result<CancelResult> {
        self.run(move |connection, _| cancel(connection, seq)).await
    }

    pub(crate) async fn get(&self, seq: i64) -> Result<Option<Message>> {
        self.run(move |connection, _| get(connection, seq)).await
    }

    pub(crate) async fn list(
        &self,
        boot: &str,
        session: Option<&str>,
        state: Option<MessageState>,
    ) -> Result<Vec<Message>> {
        let boot = boot.to_owned();
        let session = session.map(str::to_owned);
        self.run(move |connection, _| list(connection, &boot, session.as_deref(), state))
            .await
    }

    pub(crate) async fn open_counts(&self, boot: &str) -> Result<HashMap<String, u32>> {
        let boot = boot.to_owned();
        self.run(move |connection, _| open_counts(connection, &boot))
            .await
    }

    pub(crate) async fn has_earlier_open(
        &self,
        boot: &str,
        session: &str,
        seq: i64,
    ) -> Result<bool> {
        let boot = boot.to_owned();
        let session = session.to_owned();
        self.run(move |connection, _| has_earlier_open(connection, &boot, &session, seq))
            .await
    }

    pub(crate) async fn purge(&self, timestamp: u64) -> Result<()> {
        self.run(move |connection, limits| purge(connection, limits, timestamp))
            .await
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Dropping the final sender wakes the store thread; deliberately do not join here,
        // because a final Store drop may happen on a tokio worker.
        let _ = lock(&self.sender).take();
        let _ = lock(&self.thread).take();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn open_database(path: &Path, limits: Limits) -> Result<(Connection, PathBuf)> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    drop(file);

    let mut connection =
        Connection::open(path).with_context(|| format!("open {}", path.display()))?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=FULL;
         PRAGMA foreign_keys=ON;",
    )?;
    ensure_secure_files(path)?;
    migrate(&mut connection)?;
    recover(&mut connection, &limits, now())?;
    ensure_secure_files(path)?;
    Ok((connection, path.to_owned()))
}

fn ensure_secure_files(database: &Path) -> Result<()> {
    for path in [
        database.to_owned(),
        PathBuf::from(format!("{}-wal", database.display())),
        PathBuf::from(format!("{}-shm", database.display())),
    ] {
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() {
            bail!("database sibling is not a regular file: {}", path.display());
        }
        let mut mode = metadata.permissions().mode() & 0o777;
        if mode != 0o600 {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            mode = fs::metadata(&path)?.permissions().mode() & 0o777;
        }
        if mode != 0o600 {
            bail!("database file is not owner-only: {}", path.display());
        }
    }
    Ok(())
}

fn migrate(connection: &mut Connection) -> Result<()> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    match version {
        0 => {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "CREATE TABLE messages (
                   seq INTEGER PRIMARY KEY AUTOINCREMENT,
                   boot TEXT NOT NULL,
                   sender_session TEXT NOT NULL,
                   sender_address TEXT NOT NULL,
                   recipient_session TEXT NOT NULL,
                   recipient_address TEXT NOT NULL,
                   subject TEXT NOT NULL,
                   body TEXT NOT NULL,
                   state TEXT NOT NULL,
                   detail TEXT,
                   accepted_at INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL
                 );
                 CREATE INDEX messages_recipient
                   ON messages (boot, recipient_session, state, seq);
                 CREATE TABLE attempts (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   message_seq INTEGER NOT NULL REFERENCES messages(seq),
                   started_at INTEGER NOT NULL,
                   outcome TEXT NOT NULL,
                   detail TEXT,
                   finished_at INTEGER,
                   receipt_at INTEGER
                 );
                 PRAGMA user_version = 2;",
            )?;
            transaction.commit()?;
            Ok(())
        }
        1 => {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "ALTER TABLE attempts ADD COLUMN receipt_at INTEGER;
                 PRAGMA user_version = 2;",
            )?;
            transaction.commit()?;
            Ok(())
        }
        2 => Ok(()),
        other => bail!("unsupported messages.db schema version {other}"),
    }
}

fn recover(connection: &mut Connection, limits: &Limits, timestamp: u64) -> Result<()> {
    let transaction = connection.transaction()?;
    let timestamp = timestamp as i64;
    transaction.execute(
        "UPDATE messages
            SET state = 'undeliverable', detail = 'daemon_restarted', updated_at = ?1
          WHERE state IN ('pending', 'delivering')",
        params![timestamp],
    )?;
    transaction.execute(
        "UPDATE attempts
            SET outcome = 'unknown', detail = 'daemon_restarted', finished_at = ?1
          WHERE outcome = 'started'",
        params![timestamp],
    )?;
    purge_transaction(&transaction, limits, timestamp as u64)?;
    transaction.commit()?;
    Ok(())
}

fn insert_message(connection: &mut Connection, limits: &Limits, new: NewMessage) -> Result<i64> {
    let transaction = connection.transaction()?;
    let open_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM messages
          WHERE boot = ?1 AND recipient_session = ?2
            AND state IN ('pending', 'delivering')",
        params![new.boot, new.recipient_session],
        |row| row.get(0),
    )?;
    let stored_count: i64 =
        transaction.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))?;
    if open_count >= i64::from(limits.max_pending_per_recipient)
        || stored_count >= i64::from(limits.max_stored_messages)
    {
        return Err(anyhow::Error::new(QueueFullMarker));
    }
    // shortcut: message bodies are stored as plaintext in messages.db; encryption is out of scope.
    let timestamp = now() as i64;
    transaction.execute(
        "INSERT INTO messages (
            boot, sender_session, sender_address, recipient_session,
            recipient_address, subject, body, state, detail, accepted_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', NULL, ?8, ?8)",
        params![
            new.boot,
            new.sender_session,
            new.sender_address,
            new.recipient_session,
            new.recipient_address,
            new.subject,
            new.body,
            timestamp,
        ],
    )?;
    let sequence = transaction.last_insert_rowid();
    transaction.commit()?;
    Ok(sequence)
}

fn next_pending(connection: &mut Connection, boot: &str, session: &str) -> Result<Option<Message>> {
    let mut statement = connection.prepare(
        "SELECT seq, boot, sender_session, sender_address, recipient_session,
                recipient_address, subject, body, state, detail,
                EXISTS (SELECT 1 FROM attempts WHERE attempts.message_seq = messages.seq
                                         AND attempts.receipt_at IS NOT NULL)
           FROM messages
          WHERE boot = ?1 AND recipient_session = ?2 AND state = 'pending'
          ORDER BY seq ASC LIMIT 1",
    )?;
    let mut rows = statement.query(params![boot, session])?;
    rows.next()?.map(message_from_row).transpose()
}

fn begin_attempt(connection: &mut Connection, seq: i64) -> Result<Option<i64>> {
    let transaction = connection.transaction()?;
    let timestamp = now() as i64;
    let changed = transaction.execute(
        "UPDATE messages
            SET state = 'delivering', updated_at = ?1
          WHERE seq = ?2 AND state = 'pending'",
        params![timestamp, seq],
    )?;
    if changed == 0 {
        transaction.commit()?;
        return Ok(None);
    }
    transaction.execute(
        "INSERT INTO attempts (message_seq, started_at, outcome, detail, finished_at)
         VALUES (?1, ?2, 'started', NULL, NULL)",
        params![seq, timestamp],
    )?;
    let attempt = transaction.last_insert_rowid();
    transaction.commit()?;
    Ok(Some(attempt))
}

fn finish_attempt(
    connection: &mut Connection,
    seq: i64,
    attempt: i64,
    outcome: &str,
    detail: Option<&str>,
) -> Result<()> {
    let (state, message_detail) = match outcome {
        "submitted" => (MessageState::Submitted, None),
        "unsubmitted" => (MessageState::Unsubmitted, detail),
        "failed" => (MessageState::Undeliverable, detail),
        other => bail!("invalid attempt outcome {other}"),
    };
    let transaction = connection.transaction()?;
    let timestamp = now() as i64;
    let changed = transaction.execute(
        "UPDATE attempts
            SET outcome = ?1, detail = ?2, finished_at = ?3
          WHERE id = ?4 AND message_seq = ?5 AND outcome = 'started'",
        params![outcome, detail, timestamp, attempt, seq],
    )?;
    if changed == 0 {
        let exists: Option<i64> = transaction
            .query_row(
                "SELECT id FROM attempts WHERE id = ?1 AND message_seq = ?2",
                params![attempt, seq],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            bail!("unknown attempt {attempt}");
        }
        transaction.commit()?;
        return Ok(());
    }
    let changed = transaction.execute(
        "UPDATE messages SET state = ?1, detail = ?2, updated_at = ?3 WHERE seq = ?4",
        params![state.as_str(), message_detail, timestamp, seq],
    )?;
    if changed == 0 {
        bail!("unknown message sequence {seq}");
    }
    transaction.commit()?;
    Ok(())
}

fn record_receipt(connection: &mut Connection, seq: i64) -> Result<bool> {
    let transaction = connection.transaction()?;
    let attempt: Option<i64> = transaction
        .query_row(
            "SELECT id FROM attempts
              WHERE message_seq = ?1 AND outcome IN ('started','submitted','unsubmitted')
                AND receipt_at IS NULL
              ORDER BY id DESC LIMIT 1",
            params![seq],
            |row| row.get(0),
        )
        .optional()?;
    let Some(attempt) = attempt else {
        transaction.commit()?;
        return Ok(false);
    };
    let timestamp = now() as i64;
    transaction.execute(
        "UPDATE attempts
            SET outcome = 'submitted', detail = NULL, receipt_at = ?1,
                finished_at = COALESCE(finished_at, ?1)
          WHERE id = ?2 AND message_seq = ?3",
        params![timestamp, attempt, seq],
    )?;
    transaction.execute(
        "UPDATE messages
            SET state = 'submitted', detail = NULL, updated_at = ?1
          WHERE seq = ?2
            AND state IN ('delivering', 'submitted', 'unsubmitted')",
        params![timestamp, seq],
    )?;
    transaction.commit()?;
    Ok(true)
}

fn reject_attempt(connection: &mut Connection, seq: i64) -> Result<bool> {
    let transaction = connection.transaction()?;
    let attempt: Option<i64> = transaction
        .query_row(
            "SELECT id FROM attempts
              WHERE message_seq = ?1 AND outcome IN ('started','submitted','unsubmitted')
                AND receipt_at IS NULL
              ORDER BY id DESC LIMIT 1",
            params![seq],
            |row| row.get(0),
        )
        .optional()?;
    let Some(attempt) = attempt else {
        transaction.commit()?;
        return Ok(false);
    };
    let timestamp = now() as i64;
    transaction.execute(
        "UPDATE attempts
            SET outcome = 'rejected', detail = 'corrupted_submission',
                finished_at = COALESCE(finished_at, ?1)
          WHERE id = ?2 AND message_seq = ?3",
        params![timestamp, attempt, seq],
    )?;
    transaction.execute(
        "UPDATE messages
            SET state = 'pending', detail = NULL, updated_at = ?1
          WHERE seq = ?2
            AND state IN ('delivering', 'submitted', 'unsubmitted')",
        params![timestamp, seq],
    )?;
    transaction.commit()?;
    Ok(true)
}

fn fail_open(
    connection: &mut Connection,
    boot: &str,
    session: Option<&str>,
    detail: &str,
) -> Result<()> {
    let transaction = connection.transaction()?;
    let timestamp = now() as i64;
    let changed = match session {
        Some(session) => transaction.execute(
            "UPDATE messages
                SET state = 'undeliverable', detail = ?1, updated_at = ?2
              WHERE boot = ?3 AND recipient_session = ?4
                AND state IN ('pending', 'delivering')",
            params![detail, timestamp, boot, session],
        )?,
        None => transaction.execute(
            "UPDATE messages
                SET state = 'undeliverable', detail = ?1, updated_at = ?2
              WHERE boot = ?3 AND state IN ('pending', 'delivering')",
            params![detail, timestamp, boot],
        )?,
    };
    let _ = changed;
    transaction.commit()?;
    Ok(())
}

fn cancel(connection: &mut Connection, seq: i64) -> Result<CancelResult> {
    let transaction = connection.transaction()?;
    let timestamp = now() as i64;
    let changed = transaction.execute(
        "UPDATE messages
            SET state = 'cancelled', updated_at = ?1
          WHERE seq = ?2 AND state = 'pending'",
        params![timestamp, seq],
    )?;
    if changed != 0 {
        transaction.commit()?;
        return Ok(CancelResult::Cancelled);
    }
    let state = transaction
        .query_row(
            "SELECT state FROM messages WHERE seq = ?1",
            params![seq],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    transaction.commit()?;
    match state {
        None => Ok(CancelResult::Unknown),
        Some(state) => MessageState::parse(&state)
            .map(CancelResult::NotCancellable)
            .ok_or_else(|| anyhow!("invalid message state {state}")),
    }
}

fn get(connection: &mut Connection, seq: i64) -> Result<Option<Message>> {
    let mut statement = connection.prepare(
        "SELECT seq, boot, sender_session, sender_address, recipient_session,
                recipient_address, subject, body, state, detail,
                EXISTS (SELECT 1 FROM attempts WHERE attempts.message_seq = messages.seq
                                         AND attempts.receipt_at IS NOT NULL)
           FROM messages WHERE seq = ?1",
    )?;
    let mut rows = statement.query(params![seq])?;
    rows.next()?.map(message_from_row).transpose()
}

fn list(
    connection: &mut Connection,
    boot: &str,
    session: Option<&str>,
    state: Option<MessageState>,
) -> Result<Vec<Message>> {
    let state = state.map(MessageState::as_str);
    let mut messages = Vec::new();
    if let Some(session) = session {
        let mut statement = connection.prepare(
            "SELECT seq, boot, sender_session, sender_address, recipient_session,
                    recipient_address, subject, body, state, detail,
                    EXISTS (SELECT 1 FROM attempts WHERE attempts.message_seq = messages.seq
                                             AND attempts.receipt_at IS NOT NULL)
               FROM messages
              WHERE boot = ?1
                AND recipient_session = ?2
                AND (?3 IS NULL OR state = ?3)
              ORDER BY seq ASC",
        )?;
        let mut rows = statement.query(params![boot, session, state])?;
        while let Some(row) = rows.next()? {
            messages.push(message_from_row(row)?);
        }
    } else {
        let mut statement = connection.prepare(
            "SELECT seq, boot, sender_session, sender_address, recipient_session,
                    recipient_address, subject, body, state, detail,
                    EXISTS (SELECT 1 FROM attempts WHERE attempts.message_seq = messages.seq
                                             AND attempts.receipt_at IS NOT NULL)
               FROM messages
              WHERE (?1 IS NULL OR state = ?1)
              ORDER BY seq ASC",
        )?;
        let mut rows = statement.query(params![state])?;
        while let Some(row) = rows.next()? {
            messages.push(message_from_row(row)?);
        }
    }
    Ok(messages)
}

fn open_counts(connection: &mut Connection, boot: &str) -> Result<HashMap<String, u32>> {
    let mut statement = connection.prepare(
        "SELECT recipient_session, COUNT(*)
           FROM messages
          WHERE boot = ?1 AND state IN ('pending', 'delivering')
          GROUP BY recipient_session",
    )?;
    let mut rows = statement.query(params![boot])?;
    let mut counts = HashMap::new();
    while let Some(row) = rows.next()? {
        let session: String = row.get(0)?;
        let count: i64 = row.get(1)?;
        let count = u32::try_from(count).context("open message count exceeds u32")?;
        counts.insert(session, count);
    }
    Ok(counts)
}

fn has_earlier_open(
    connection: &mut Connection,
    boot: &str,
    session: &str,
    seq: i64,
) -> Result<bool> {
    let exists = connection.query_row(
        "SELECT EXISTS (
            SELECT 1
              FROM messages
             WHERE boot = ?1
               AND recipient_session = ?2
               AND state IN ('pending', 'delivering')
               AND seq < ?3
        )",
        params![boot, session, seq],
        |row| row.get(0),
    )?;
    Ok(exists)
}

fn purge(connection: &mut Connection, limits: &Limits, timestamp: u64) -> Result<()> {
    let transaction = connection.transaction()?;
    purge_transaction(&transaction, limits, timestamp)?;
    transaction.commit()?;
    Ok(())
}

fn purge_transaction(transaction: &Transaction<'_>, limits: &Limits, timestamp: u64) -> Result<()> {
    let Some(cutoff) = timestamp.checked_sub(limits.retention_secs) else {
        return Ok(());
    };
    let cutoff = i64::try_from(cutoff).context("retention cutoff exceeds i64")?;
    transaction.execute(
        "DELETE FROM attempts
          WHERE message_seq IN (
            SELECT seq FROM messages
             WHERE state IN ('submitted', 'unsubmitted', 'cancelled', 'undeliverable')
               AND updated_at <= ?1
          )",
        params![cutoff],
    )?;
    transaction.execute(
        "DELETE FROM messages
          WHERE state IN ('submitted', 'unsubmitted', 'cancelled', 'undeliverable')
            AND updated_at <= ?1",
        params![cutoff],
    )?;
    Ok(())
}

fn message_from_row(row: &Row<'_>) -> Result<Message> {
    let state_text: String = row.get(8)?;
    let state = MessageState::parse(&state_text)
        .ok_or_else(|| anyhow!("invalid message state {state_text}"))?;
    Ok(Message {
        seq: row.get(0)?,
        boot: row.get(1)?,
        sender_session: row.get(2)?,
        sender_address: row.get(3)?,
        recipient_session: row.get(4)?,
        recipient_address: row.get(5)?,
        subject: row.get(6)?,
        body: row.get(7)?,
        state,
        detail: row.get(9)?,
        observed: row.get(10)?,
    })
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
