use super::*;
use crate::{
    domain::{ErrorCode, ProxySnapshot, RunMode, RuntimeCommandOutput},
    ipc::frame,
    ui::{CoreStatus, UiResponse, UiState},
};
use serde_json::json;

/// Wire fixtures are intentionally independent of the Rust serializer under test.
#[test]
fn future_error_code_keeps_the_response_and_does_not_mark_core_offline() {
    let mut bytes = Vec::new();
    frame::write_message(
        &mut bytes,
        &json!({
            "Response": {
                "request_id": 17, "operation_id": 9,
                "response": {"Runtime": {
                    "request": {"type": "get_mode"},
                    "result": {"Err": {
                        "code": "future_controller_busy", "message": "Retry shortly",
                        "retry_after_ms": 100
                    }}
                }},
                "future_metadata": {"source": "daemon"}
            }
        }),
    )
    .unwrap();
    let ClientMessage::Response(envelope) = frame::read_message(&mut bytes.as_slice()).unwrap()
    else {
        panic!("expected response");
    };
    assert_eq!(envelope.request_id, Some(17));
    assert_eq!(envelope.operation_id, Some(9));
    let mut state = UiState::default();
    state.core_status = CoreStatus::Running;
    state.apply_response_envelope(envelope);
    let error = state.last_error.unwrap();
    assert_eq!(error.code, ErrorCode::Unknown);
    assert_eq!(error.message, "Retry shortly");
    assert_eq!(state.core_status, CoreStatus::Running);
}

#[test]
fn optional_node_metadata_can_be_absent_or_extended() {
    let minimal = json!({
        "groups": [{"name": "Route", "kind": "Selector", "members": ["Node"]}],
        "proxies": {"Node": {"kind": "VLESS"}}
    });
    let snapshot: ProxySnapshot = serde_json::from_value(minimal.clone()).unwrap();
    let node = &snapshot.proxies["Node"];
    assert_eq!(node.kind, "VLESS");
    assert_eq!(node.udp, None);
    assert!(!node.xudp && !node.tfo && !node.mptcp && !node.smux && !node.hidden);
    assert_eq!(node.delay, None);
    assert_eq!(node.selected, None);
    assert_eq!(snapshot.groups[0].selected, None);

    let mut future = minimal;
    future["proxies"]["Node"]["future_capability"] = json!({"enabled": true});
    future["future_snapshot_metadata"] = json!(42);
    assert_eq!(
        serde_json::from_value::<ProxySnapshot>(future).unwrap(),
        snapshot
    );
}

#[test]
fn older_mode_response_and_future_fields_decode_identically() {
    let old = json!({"Response": {
        "request_id": 1, "operation_id": 1,
        "response": {"Runtime": {
            "request": {"type": "get_mode"}, "result": {"Ok": {"output": {"type": "mode", "value": "rule"}, "summary": "loaded"}}
        }}
    }});
    let mut future = old.clone();
    future["Response"]["response"]["Runtime"]["result"]["Ok"]["future_metadata"] = json!(true);
    for fixture in [old, future] {
        let ClientMessage::Response(envelope) = serde_json::from_value(fixture).unwrap() else {
            panic!("expected response")
        };
        let UiResponse::Runtime {
            result: Ok(result), ..
        } = envelope.response
        else {
            panic!("expected runtime result")
        };
        assert_eq!(result.output, RuntimeCommandOutput::Mode(RunMode::Rule));
    }
}

#[test]
fn missing_required_fields_and_unknown_commands_are_still_rejected() {
    assert!(serde_json::from_value::<crate::domain::ProxyDetails>(json!({"udp": true})).is_err());
    assert!(
        serde_json::from_value::<crate::domain::AppError>(json!({"code": "future_error"})).is_err()
    );
    assert!(
        serde_json::from_value::<crate::domain::AppError>(json!({"message": "missing code"}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<crate::domain::RuntimeCommand>(json!({"type": "future_command"}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<crate::domain::RuntimeCommand>(
            json!({"type": "select_proxy", "group": "Route"})
        )
        .is_err()
    );
}

#[test]
fn known_error_codes_keep_their_wire_names() {
    for (name, code) in [
        ("invalid_input", ErrorCode::InvalidInput),
        ("not_found", ErrorCode::NotFound),
        ("conflict", ErrorCode::Conflict),
        ("permission_denied", ErrorCode::PermissionDenied),
        ("validation_failed", ErrorCode::ValidationFailed),
        ("storage_failed", ErrorCode::StorageFailed),
        ("platform_failed", ErrorCode::PlatformFailed),
        ("core_unavailable", ErrorCode::CoreUnavailable),
        ("core_rejected_config", ErrorCode::CoreRejectedConfig),
        ("request_timeout", ErrorCode::RequestTimeout),
        ("proxy_delay_failed", ErrorCode::ProxyDelayFailed),
    ] {
        assert_eq!(
            serde_json::from_value::<ErrorCode>(json!(name)).unwrap(),
            code
        );
        assert_eq!(serde_json::to_value(code).unwrap(), json!(name));
    }
}

#[test]
fn legacy_welcome_has_no_unified_proxy_capability_and_settings_get_defaults() {
    let old = json!({"Welcome": {
        "protocol_version": 6,
        "initial": {
            "profiles": [], "selected_profile": null, "runtime_settings": null,
            "application_settings": {"settings": {"theme": "dark", "language": "zh-CN", "log_limit": 2000}, "data_directory": "/tmp/test"}
        }
    }});
    let ClientMessage::Welcome { initial, .. } = serde_json::from_value(old.clone()).unwrap()
    else {
        panic!("expected welcome")
    };
    assert!(initial.capabilities.is_empty());
    assert_eq!(
        *initial.application_settings.settings.system_proxy,
        crate::domain::SystemProxySettings::default()
    );
    let mut future = old;
    future["Welcome"]["initial"]["capabilities"] =
        json!(["unified_system_proxy", "future_unknown_capability"]);
    let ClientMessage::Welcome { initial, .. } = serde_json::from_value(future).unwrap() else {
        panic!("expected welcome")
    };
    assert!(
        initial
            .capabilities
            .iter()
            .any(|c| c == UNIFIED_SYSTEM_PROXY)
    );
    assert_eq!(PROTOCOL_VERSION, 6);
}

#[test]
fn unified_proxy_command_does_not_change_legacy_enable_shape() {
    use crate::domain::SystemProxyCommand;
    let legacy: SystemProxyCommand = serde_json::from_value(json!({"type": "enable", "services": ["Wi-Fi"], "endpoint": {"host": "127.0.0.1", "port": 7890}})).unwrap();
    assert!(matches!(legacy, SystemProxyCommand::Enable { .. }));
    let unified: SystemProxyCommand =
        serde_json::from_value(json!({"type": "set_enabled", "enabled": true})).unwrap();
    assert!(matches!(
        unified,
        SystemProxyCommand::SetEnabled { enabled: true }
    ));
    assert!(serde_json::from_value::<SystemProxyCommand>(json!({"type": "set_enabled"})).is_err());
}
