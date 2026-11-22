use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

use super::views::{session_activity, session_quota};
use super::{Runtime, response};
use crate::client::Framed;
use crate::delivery::{AnyChannel, Channel};
use crate::emulator::Size;
use crate::messaging;
use crate::session::{AttachmentSlot, Session};
use crate::wire::{ClientFrame, Response, ServerFrame, StatusInfo, TeamMember};

struct AttachmentGuard {
    session: Arc<Session>,
    generation: u64,
    closed: watch::Sender<bool>,
    presence: Arc<Mutex<HashMap<String, messaging::Presence>>>,
    viewer: bool,
    counted: bool,
}

impl Drop for AttachmentGuard {
    fn drop(&mut self) {
        {
            let mut state = self.session.lock();
            if state
                .attachment
                .as_ref()
                .is_some_and(|slot| slot.generation == self.generation)
            {
                state.attachment = None;
            }
            if state.pending_attachment == Some(self.generation) {
                state.pending_attachment = None;
            }
        }
        if self.counted
            && let Some(presence) = self
                .presence
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_mut(&self.session.id().0)
        {
            presence.attach_ended(Instant::now(), self.viewer);
        }
        let _ = self.closed.send(true);
    }
}

async fn attachment_status(
    session: &Arc<Session>,
    runtime: &Runtime,
) -> anyhow::Result<StatusInfo> {
    let counts = runtime.store.open_counts(&runtime.boot).await?;
    let team_name = session.team().map(|team| team.name.clone());
    let team_sessions = team_name.as_ref().map_or_else(Vec::new, |team_name| {
        runtime
            .sessions
            .snapshot()
            .iter()
            .filter(|candidate| candidate.team().is_some_and(|team| team.name == *team_name))
            .cloned()
            .collect::<Vec<_>>()
    });
    let mut members = team_sessions
        .into_iter()
        .map(|member| {
            let activity = session_activity(&member);
            let held = AnyChannel::for_session(member.clone())
                .hold_reason()
                .or(member.needs_input())
                .is_some();
            TeamMember {
                id: member.id().0.clone(),
                name: member
                    .name()
                    .map_or_else(|| member.id().0.clone(), str::to_owned),
                activity,
                exited: member.exit_code().is_some(),
                held,
                starting: member.starting(),
            }
        })
        .collect::<Vec<_>>();
    members.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    let hold = AnyChannel::for_session(session.clone())
        .hold_reason()
        .or(session.needs_input())
        .map(str::to_owned);
    let activity = session_activity(session);
    let starting = session.starting();
    Ok(StatusInfo {
        address: messaging::address(session.name(), &session.id().0, &runtime.host_name),
        pending: counts.get(&session.id().0).copied().unwrap_or(0),
        hold,
        harness: Some(session.harness()),
        activity,
        confined: session.confined(),
        confine_auto: session.confine_auto(),
        starting,
        quota: session_quota(session, runtime.omp_usage.current().as_ref()),
        team: session.team().cloned(),
        role: session.role().map(str::to_owned),
        members,
        remote: runtime.remote_status(session),
    })
}

#[allow(clippy::too_many_arguments)] // Locked attach flow keeps status, viewer, and runtime handles explicit.
pub(super) async fn attach(
    connection: &mut Framed,
    session: Arc<Session>,
    force: bool,
    size: Size,
    top_rows: u16,
    viewer: bool,
    status: bool,
    runtime: Arc<Runtime>,
) -> anyhow::Result<bool> {
    if top_rows > 8 {
        response(
            connection,
            Response::Error {
                message: "top rows exceed 8".into(),
            },
        )
        .await?;
        return Ok(false);
    }
    let (takeover, mut taken_over) = watch::channel(false);
    let (closed, completion) = watch::channel(false);
    let reservation = {
        let mut state = session.lock();
        if (state.attachment.is_some() || state.pending_attachment.is_some()) && !force {
            None
        } else {
            let previous = state
                .attachment
                .as_ref()
                .map(|slot| (slot.takeover.clone(), slot.closed.clone()));
            state.generation += 1;
            let generation = state.generation;
            state.pending_attachment = Some(generation);
            Some((generation, previous))
        }
    };
    let Some((generation, previous)) = reservation else {
        response(
            connection,
            Response::Error {
                message: format!(
                    "session {} is attached elsewhere (use --force to take over)",
                    session.id().0
                ),
            },
        )
        .await?;
        return Ok(false);
    };
    let mut guard = AttachmentGuard {
        session: session.clone(),
        generation,
        closed,
        presence: runtime.presence.clone(),
        viewer,
        counted: false,
    };
    if let Some((previous, mut previous_closed)) = previous {
        let _ = previous.send(true);
        while !*previous_closed.borrow() {
            if previous_closed.changed().await.is_err() {
                break;
            }
        }
    }
    let snapshot = {
        let mut state = session.lock();
        if state.pending_attachment != Some(generation) {
            Err(anyhow::anyhow!("attachment was taken over"))
        } else {
            state.emulator.set_row_origin(top_rows);
            let resize = session.resize_locked(&mut state, size);
            resize.map(|()| {
                let bytes = state.emulator.render_full();
                state.dirty = false;
                state.attachment = Some(AttachmentSlot {
                    generation,
                    takeover,
                    closed: completion,
                    viewer,
                });
                state.pending_attachment = None;
                (bytes, state.exit_code)
            })
        }
    };
    let (bytes, exited) = match snapshot {
        Ok(snapshot) => snapshot,
        Err(error) => {
            response(
                connection,
                Response::Error {
                    message: error.to_string(),
                },
            )
            .await?;
            return Ok(false);
        }
    };
    if exited.is_none() {
        runtime
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(session.id().0.clone())
            .or_insert_with(|| messaging::Presence::new(runtime.limits.away_secs))
            .attach_started(Instant::now(), viewer);
        guard.counted = true;
    }
    let attached = if top_rows == 0 {
        Response::Attached
    } else {
        Response::AttachedTop { top_rows }
    };
    response(connection, attached).await?;
    send_data(connection, &bytes).await?;
    if let Some(code) = exited {
        connection.send(&ServerFrame::Exit(code).encode()).await?;
        connection.stream.shutdown().await?;
        return Ok(true);
    }
    let mut last = None;
    if status {
        let info = attachment_status(&session, &runtime).await?;
        connection
            .send(&ServerFrame::Status(info.clone()).encode())
            .await?;
        last = Some(info);
    }
    let period = Duration::from_secs(1);
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        if *taken_over.borrow() {
            connection
                .send(&ServerFrame::Detached("taken over by another attachment".into()).encode())
                .await?;
            connection.stream.shutdown().await?;
            return Ok(true);
        }
        // Delivery shares this notifier and can consume its stored permit while
        // a snapshot is being sent. Register before checking pending render state.
        let output = session.notify().notified();
        tokio::pin!(output);
        output.as_mut().enable();
        let output_pending = {
            let state = session.lock();
            (state.dirty && !state.emulator.is_scrolled())
                || (state.exit_code.is_some() && !state.dirty)
        };
        tokio::select! {
            _ = taken_over.changed() => {},
            // shortcut: one-second poll per attachment; push from the store's write path if latency
            // or load matters.
            _ = ticker.tick() => {
                if status {
                    let info = attachment_status(&session, &runtime).await?;
                    if last.as_ref() != Some(&info) {
                        connection.send(&ServerFrame::Status(info.clone()).encode()).await?;
                        last = Some(info);
                    }
                }
            },
            _ = async { if !output_pending { output.await; } } => {
                let (bytes, exited) = {
                    let mut state = session.lock();
                    let bytes = state.emulator.render_update();
                    if !state.emulator.is_scrolled() { state.dirty = false; }
                    let exited = state.exit_code.filter(|_| !state.dirty);
                    (bytes, exited)
                };
                send_data(connection, &bytes).await?;
                if let Some(code) = exited {
                    connection.send(&ServerFrame::Exit(code).encode()).await?;
                    connection.stream.shutdown().await?;
                    return Ok(true);
                }
            },
            payload = connection.recv() => {
                let Some(payload) = payload? else { return Ok(true); };
                match ClientFrame::decode(&payload)? {
                    ClientFrame::Detach => return Ok(true),
                    ClientFrame::Input(bytes) if !guard.viewer => {
                        session.note_human_input(&bytes);
                        if !session.enqueue(bytes).await? {
                            session.notify().notify_one();
                        }
                    },
                    ClientFrame::Input(_) => {},
                    ClientFrame::Release if !guard.viewer => session.release(),
                    ClientFrame::Release => {},
                    ClientFrame::Mode { viewer } => {
                        if guard.counted && guard.viewer != viewer {
                            if let Some(presence) = runtime
                                .presence
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .get_mut(&session.id().0)
                            {
                                presence.mode_changed(Instant::now(), viewer);
                            }
                            guard.viewer = viewer;
                        }
                        let mut state = session.lock();
                        if let Some(slot) = state
                            .attachment
                            .as_mut()
                            .filter(|slot| slot.generation == guard.generation)
                        {
                            slot.viewer = viewer;
                        }
                    },
                    frame => {
                        let (bytes, exited) = {
                            let mut state = session.lock();
                            match frame {
                                ClientFrame::Resize { cols, rows }
                                    if state.exit_code.is_none() =>
                                    session.resize_locked(&mut state, Size { cols, rows })?,
                                ClientFrame::Resize { .. } => {},
                                ClientFrame::Scroll(scroll) => state.emulator.scroll(scroll),
                                ClientFrame::Redraw => {},
                                _ => unreachable!(),
                            }
                            state.dirty = false;
                            (state.emulator.render_full(), state.exit_code)
                        };
                        send_data(connection, &bytes).await?;
                        if let Some(code) = exited {
                            connection.send(&ServerFrame::Exit(code).encode()).await?;
                            connection.stream.shutdown().await?;
                            return Ok(true);
                        }
                    },
                }
            },
        }
    }
}

async fn send_data(connection: &mut Framed, bytes: &[u8]) -> anyhow::Result<()> {
    for chunk in bytes.chunks(64 * 1024) {
        connection.send_tagged(0x01, chunk).await?;
    }
    Ok(())
}
