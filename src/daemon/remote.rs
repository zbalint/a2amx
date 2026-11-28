use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::messaging::{self, code};
use crate::session::{Session, SessionId};
use crate::wire::{GatewayAgent, RemoteLease, RemoteStatus, Response};

use super::{Runtime, constant_time_eq, random_hex};

/// The lease and code state of every team. The lock is a leaf: held only inside these methods.
pub(super) struct Leases {
    state: Mutex<LeaseState>,
}

impl Leases {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(LeaseState::default()),
        }
    }

    /// Store a fresh pending code for `team`, replacing any earlier one, and reset the wrong-attempt count.
    pub(super) fn issue(&self, team: String, code: String, expires: Instant) {
        let mut leases = self.lock();
        leases.teams.entry(team).or_default().pending = Some(PendingLeaseCode { code, expires });
        leases.wrong_codes = 0;
    }

    /// Consume the pending code that matches; None for a wrong, expired or already-used code or after too many wrong attempts.
    pub(super) fn redeem(&self, code: &str, now: Instant) -> Option<String> {
        let mut leases = self.lock();
        if leases.wrong_codes >= messaging::MAX_WRONG_CODES {
            return None;
        }
        if let Some(team) = leases.take_matching(code, now) {
            return Some(team);
        }
        leases.record_wrong_code(now);
        None
    }

    /// Begin the lease of `team` for `members` (replacing an earlier one).
    pub(super) fn start(&self, team: &str, members: Vec<SessionId>, expires: Instant) {
        let mut leases = self.lock();
        leases.teams.entry(team.to_owned()).or_default().lease =
            Some(ActiveLease { expires, members });
    }

    /// None when `id` is not a member of `team`'s lease, Active before expiry, Ended after.
    pub(super) fn state(&self, team: &str, id: &str, now: Instant) -> RemoteLease {
        let leases = self.lock();
        let Some(lease) = leases
            .teams
            .get(team)
            .and_then(|state| state.lease.as_ref())
        else {
            return RemoteLease::None;
        };
        if !lease.members.iter().any(|member| member.0.as_str() == id) {
            return RemoteLease::None;
        }
        if now < lease.expires {
            RemoteLease::Active
        } else {
            RemoteLease::Ended
        }
    }

    fn lock(&self) -> MutexGuard<'_, LeaseState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Default)]
struct LeaseState {
    teams: HashMap<String, TeamLease>,
    wrong_codes: u32,
}

impl LeaseState {
    /// Scan every team without early exit so comparison timing does not reveal match position.
    fn take_matching(&mut self, code: &str, now: Instant) -> Option<String> {
        let mut matched_team = None;
        for (team, state) in &self.teams {
            if state
                .pending
                .as_ref()
                .is_some_and(|pending| pending.accepts(code, now))
            {
                matched_team.get_or_insert_with(|| team.clone());
            }
        }
        let team = matched_team?;
        self.teams
            .get_mut(&team)
            .and_then(|state| state.pending.take());
        Some(team)
    }

    fn record_wrong_code(&mut self, now: Instant) {
        self.wrong_codes = self.wrong_codes.saturating_add(1);
        let drop_all = self.wrong_codes >= messaging::MAX_WRONG_CODES;
        for state in self.teams.values_mut() {
            if drop_all
                || state
                    .pending
                    .as_ref()
                    .is_some_and(|pending| now >= pending.expires)
            {
                state.pending = None;
            }
        }
    }
}

#[derive(Default)]
struct TeamLease {
    lease: Option<ActiveLease>,
    pending: Option<PendingLeaseCode>,
}

struct ActiveLease {
    expires: Instant,
    members: Vec<SessionId>,
}

struct PendingLeaseCode {
    code: String,
    expires: Instant,
}

impl PendingLeaseCode {
    fn accepts(&self, code: &str, now: Instant) -> bool {
        now < self.expires && constant_time_eq(code, &self.code)
    }
}

impl Runtime {
    fn remote_members(&self, team: &str) -> Vec<(SessionId, String, messaging::RemoteMode)> {
        self.sessions
            .snapshot()
            .iter()
            .filter_map(|session| {
                (session.exit_code().is_none()
                    && session.team().is_some_and(|scope| scope.name == team))
                .then(|| {
                    session.remote().map(|mode| {
                        let name = session
                            .name()
                            .map(str::to_owned)
                            .unwrap_or_else(|| session.id().0.clone());
                        (session.id().clone(), name, mode)
                    })
                })
                .flatten()
            })
            .collect()
    }

    pub(super) async fn issue_lease_code(&self, team: String) -> anyhow::Result<Response> {
        let members = self.remote_members(&team);
        if members.is_empty() {
            return Ok(Response::Error {
                message: "no agent in this team has remote messaging enabled".into(),
            });
        }
        let code = tokio::task::spawn_blocking(random_hex::<6>).await??;
        let agents = members
            .iter()
            .map(|(_, name, mode)| {
                let mode = match mode {
                    messaging::RemoteMode::Send => "send",
                    messaging::RemoteMode::Both => "both",
                };
                format!("{name} ({mode})")
            })
            .collect();
        self.leases.issue(
            team,
            code.clone(),
            Instant::now() + Duration::from_secs(self.limits.code_secs),
        );
        Ok(Response::LeaseCode {
            code,
            agents,
            valid_secs: self.limits.code_secs,
        })
    }

    pub(super) fn redeem_lease_code(&self, code: String) -> Response {
        let now = Instant::now();
        let Some(team) = self.leases.redeem(&code, now) else {
            return Response::Error {
                message: "invalid or expired code".into(),
            };
        };
        let members = self
            .remote_members(&team)
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        self.leases.start(
            &team,
            members,
            now + Duration::from_secs(self.limits.lease_secs),
        );
        Response::Ok
    }

    pub(super) fn remote_status(&self, session: &Session) -> Option<RemoteStatus> {
        session.remote()?;
        let now = Instant::now();
        let lease = session.team().map_or(RemoteLease::None, |team| {
            self.leases.state(&team.name, &session.id().0, now)
        });
        Some(RemoteStatus {
            lease,
            gateway: self.gateway_connections.load(Ordering::Relaxed) > 0,
        })
    }

    pub(super) fn gateway_agents(&self) -> Response {
        let now = Instant::now();
        let agents = self
            .sessions
            .snapshot()
            .iter()
            .filter_map(|session| {
                if session.exit_code().is_some() || !session.confined() {
                    return None;
                }
                let mode = session.remote()?;
                let team = session.team()?;
                if self.leases.state(&team.name, &session.id().0, now) != RemoteLease::Active {
                    return None;
                }
                let mode = match mode {
                    messaging::RemoteMode::Send => "send",
                    messaging::RemoteMode::Both => "both",
                };
                Some(GatewayAgent {
                    address: messaging::address(session.name(), &session.id().0, &self.host_name),
                    name: session
                        .name()
                        .map(str::to_owned)
                        .unwrap_or_else(|| session.id().0.clone()),
                    team: team.name.clone(),
                    mode: mode.into(),
                })
            })
            .collect();
        Response::GatewayAgents { agents }
    }

    pub(super) fn remote_ineligible(
        &self,
        session: &Session,
        now: Instant,
        direction: messaging::Direction,
    ) -> Option<(&'static str, &'static str)> {
        if session.exit_code().is_some() {
            return Some((code::RECIPIENT_EXITED, "recipient has exited"));
        }
        if !session.confined() {
            return Some((
                code::NOT_PERMITTED,
                "remote messaging needs a confined session",
            ));
        }
        let enabled = match direction {
            messaging::Direction::ToAgent => match session.remote() {
                None | Some(messaging::RemoteMode::Send) => false,
                Some(messaging::RemoteMode::Both) => true,
            },
            messaging::Direction::ToHuman => match session.remote() {
                None => false,
                Some(messaging::RemoteMode::Send | messaging::RemoteMode::Both) => true,
            },
        };
        if !enabled {
            return Some((
                code::NOT_PERMITTED,
                "remote messages to this session are not enabled",
            ));
        }
        let Some(team) = session.team() else {
            return Some((
                code::NOT_PERMITTED,
                "remote messaging is not active for this team; start it at the computer",
            ));
        };
        (self.leases.state(&team.name, &session.id().0, now) != RemoteLease::Active).then_some((
            code::NOT_PERMITTED,
            "remote messaging is not active for this team; start it at the computer",
        ))
    }
}
