use super::Role;
use crate::wire::Request;

/// Which token roles may send a request.
enum Audience {
    /// Only the gateway token.
    Gateway,
    /// The admin token or a session token.
    AdminOrSession,
    /// Only the admin token.
    AdminOnly,
}

/// The one table: every request variant is named once, with no wildcard, so a new variant fails to compile until it is classified.
fn audience(request: &Request) -> Audience {
    match request {
        Request::GatewayAgents
        | Request::HumanInbound { .. }
        | Request::HumanOutboundNext
        | Request::HumanOutboundDone { .. }
        | Request::RedeemLeaseCode { .. } => Audience::Gateway,
        Request::SendMessage { .. }
        | Request::Reset { .. }
        | Request::ListAgents
        | Request::MessageStatus { .. }
        | Request::ReportPrompt { .. }
        | Request::ReportQuota { .. }
        | Request::BridgeAttach => Audience::AdminOrSession,
        Request::Hello { .. }
        | Request::Confine { .. }
        | Request::NewSession { .. }
        | Request::List
        | Request::Version
        | Request::Attach { .. }
        | Request::Kill { .. }
        | Request::Screen { .. }
        | Request::Explain { .. }
        | Request::Shutdown { .. }
        | Request::IssueLeaseCode { .. }
        | Request::ListMessages { .. }
        | Request::CancelMessage { .. } => Audience::AdminOnly,
    }
}

/// The verdict `serve` applies before dispatch.
pub(super) fn request_permitted(role: &Role, request: &Request) -> bool {
    matches!(
        (audience(request), role),
        (Audience::Gateway, Role::Gateway)
            | (Audience::AdminOrSession, Role::Admin | Role::Session(_))
            | (Audience::AdminOnly, Role::Admin)
    )
}

/// The sender id of a session token, or the refusal message for any other role.
pub(super) fn session_id<'a>(role: &'a Role, tool: &str) -> Result<&'a str, String> {
    match role {
        Role::Session(id) => Ok(id.as_str()),
        Role::Admin | Role::Gateway => Err(format!("{tool} needs a session token")),
    }
}
