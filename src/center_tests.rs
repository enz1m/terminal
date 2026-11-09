use super::*;
use enzui_agent::center::{AgentCaller, Invocation, InvocationEvent, Uuid};

fn invocation(function_id: &str, arguments: Value) -> Invocation {
    Invocation {
        invocation_id: Uuid::new_v4().expect("entropy"),
        function_id: function_id.into(),
        arguments,
        caller: AgentCaller {
            credential_id: "agent.test".into(),
            display_name: "Test".into(),
            session_id: Uuid::new_v4().expect("entropy"),
        },
        deadline_unix_ms: 0,
        idempotency_key: None,
    }
}

fn app_with_center() -> App {
    let mut app = App::session_fixture();
    app.center = Some(CenterHost::offline(registry()));
    app
}

fn call(app: &mut App, invocation: Invocation) -> Vec<InvocationEvent> {
    let _ = app.update(Message::Center(ConnectorEvent::Invoke(Box::new(
        invocation,
    ))));
    app.center.as_mut().expect("center host").take_sent()
}

fn result(events: &[InvocationEvent]) -> &Value {
    match events {
        [
            InvocationEvent::Accepted { .. },
            InvocationEvent::Completed { result, .. },
        ] => result,
        other => panic!("expected accepted then completed, got {other:?}"),
    }
}

fn failure(events: &[InvocationEvent]) -> &str {
    events
        .iter()
        .find_map(|event| match event {
            InvocationEvent::Failed { code, .. } => Some(code.as_str()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected a failure, got {events:?}"))
}
