//! The terminal's enzui-center functions, behind the `center` feature.
//!
//! The surface is session management only: list, open and select sessions. Driving a live PTY (`send_input`,
//! `get_output`) is deliberately absent — a far larger surface with no agent
//! use case that justified it. Every mutation is an ordinary [`Message`]
//! through [`App::update`]; the host answers each call exactly once.

use enzui_agent::center::{
    AppFunction, AppFunctionRegistry, AppMetadata, CenterHost, CenterProvision, CenterTurn,
    ConnectorEvent, ConsentScope, FunctionRisk, FunctionSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;

/// What an admitted call asks the terminal to do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CenterCall {
    ListSessions,
    NewLocalSession,
    SelectSession { session_id: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionArgs {
    session_id: String,
}

fn no_arguments() -> Value {
    json!({"type": "object", "additionalProperties": false})
}

fn session_id_argument() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_id"],
        "properties": {"session_id": {"type": "string", "maxLength": 128}}
    })
}

fn session_id_result() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_id"],
        "properties": {"session_id": {"type": "string"}}
    })
}

/// The terminal's registered functions.
pub(crate) fn registry() -> AppFunctionRegistry<CenterCall> {
    let list = FunctionSpec::new(
        "list_sessions",
        "List sessions",
        "Lists the open terminal sessions: id, title and whether it is the active one.",
        no_arguments(),
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["sessions"],
            "properties": {"sessions": {"type": "array", "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["session_id", "title", "active"],
                "properties": {
                    "session_id": {"type": "string"},
                    "title": {"type": "string"},
                    "active": {"type": "boolean"}
                }
            }}}
        }),
        // Titles follow the shell and can name commands and directories.
        FunctionRisk::SensitiveRead,
        vec![ConsentScope::Once, ConsentScope::Session],
    );
    let new_local = FunctionSpec::new(
        "new_local_session",
        "Open a local shell",
        "Opens a new local shell session and makes it the active one.",
        no_arguments(),
        session_id_result(),
        FunctionRisk::Mutating,
        vec![ConsentScope::Once, ConsentScope::Session],
    );
    let select = FunctionSpec::new(
        "select_session",
        "Select a session",
        "Makes the session with this id the active one.",
        session_id_argument(),
        session_id_result(),
        FunctionRisk::Mutating,
        vec![ConsentScope::Once, ConsentScope::Session],
    );
    AppFunctionRegistry::from_functions(vec![
        AppFunction::new(list, |_| CenterCall::ListSessions).expect("list_sessions spec"),
        AppFunction::new(new_local, |_| CenterCall::NewLocalSession)
            .expect("new_local_session spec"),
        AppFunction::typed(select, |args: SessionArgs| CenterCall::SelectSession {
            session_id: args.session_id,
        })
        .expect("select_session spec"),
    ])
    .expect("the terminal registry is valid")
}

fn metadata() -> AppMetadata {
    AppMetadata {
        id: APP_ID.into(),
        name: "Terminal".into(),
        description: "Local shell and SSH sessions".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    }
}

type Outcome = Result<Value, (&'static str, String)>;

impl App {
    /// Ask for the center connection: now if the app is ready, else when
    /// startup finishes ([`Self::center_after_startup`]).
    pub(crate) fn request_center(&mut self) {
        self.center_requested = true;
        if self.startup.is_none() {
            self.start_center();
        }
    }

    /// Called once startup has installed the loaded state.
    pub(crate) fn center_after_startup(&mut self) {
        if self.center_requested && self.center.is_none() {
            self.start_center();
        }
    }

    /// Connect to enzui-center when this build opted in and a credential is
    /// enrolled for [`APP_ID`]; otherwise stay unexposed.
    pub(crate) fn start_center(&mut self) {
        let Some(provision) = CenterProvision::discover(APP_ID) else {
            return;
        };
        match CenterHost::spawn(provision, metadata(), registry()) {
            Ok(host) => self.center = Some(host),
            Err(error) => eprintln!("terminal-emulator: center connector disabled: {error}"),
        }
    }

    pub(crate) fn center_subscription(&self) -> Subscription<Message> {
        self.center
            .as_ref()
            .map_or_else(Subscription::none, |host| {
                host.subscription(Message::Center)
            })
    }

    /// Final shutdown: refuse new calls, cancel live ones, join the connector.
    pub(crate) fn shutdown_center(&mut self) {
        if let Some(host) = self.center.take() {
            host.shutdown();
        }
    }

    pub(crate) fn center_event(&mut self, event: ConnectorEvent) -> Task<Message> {
        let Some(host) = self.center.as_mut() else {
            return Task::none();
        };
        // These handlers answer in this turn; subsequent UI work belongs to the app.
        let CenterTurn::Call(call, request) = host.receive_with_ends(event).turn else {
            return Task::none();
        };
        if self.startup.is_some() {
            host.fail(call, "unavailable", "the terminal is still starting");
            return Task::none();
        }
        let (task, outcome) = self.apply_center_call(request);
        if let Some(host) = self.center.as_mut() {
            match outcome {
                Ok(result) => host.complete(call, result),
                Err((code, message)) => host.fail(call, code, message),
            }
        }
        task
    }

    fn apply_center_call(&mut self, request: CenterCall) -> (Task<Message>, Outcome) {
        match request {
            CenterCall::ListSessions => {
                let sessions: Vec<Value> = self
                    .sessions
                    .iter()
                    .enumerate()
                    .map(|(index, session)| {
                        json!({
                            "session_id": session.id,
                            "title": session.title,
                            "active": index == self.active,
                        })
                    })
                    .collect();
                (Task::none(), Ok(json!({ "sessions": sessions })))
            }
            CenterCall::NewLocalSession => {
                let before: Vec<String> = self.sessions.iter().map(|s| s.id.clone()).collect();
                let task = self.update(Message::NewLocal);
                let opened = self
                    .sessions
                    .iter()
                    .find(|session| !before.contains(&session.id))
                    .map(|session| session.id.clone());
                let outcome = match opened {
                    Some(session_id) => Ok(json!({ "session_id": session_id })),
                    None => Err((
                        "unavailable",
                        self.pane_feedback
                            .clone()
                            .unwrap_or_else(|| "no new session could be opened".into()),
                    )),
                };
                (task, outcome)
            }
            CenterCall::SelectSession { session_id } => {
                let Some(index) = self.sessions.iter().position(|s| s.id == session_id) else {
                    return (
                        Task::none(),
                        Err(("not_found", "no session has that id".into())),
                    );
                };
                let task = self.update(Message::SelectSession(index));
                let outcome = if self
                    .sessions
                    .get(self.active)
                    .is_some_and(|s| s.id == session_id)
                {
                    Ok(json!({ "session_id": session_id }))
                } else {
                    Err(("unavailable", "the session could not be selected".into()))
                };
                (task, outcome)
            }
        }
    }
}

#[cfg(test)]
#[path = "center_tests.rs"]
mod tests;
