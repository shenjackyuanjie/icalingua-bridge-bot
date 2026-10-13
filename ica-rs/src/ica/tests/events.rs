use rust_socketio::{Event, Payload};
use serde_json::json;

use super::{first_arg, forwarded_event_name, payload_args};

#[test]
fn generic_business_hook_keeps_original_argument_positions() {
    let payload = Payload::Text(vec![json!([{"roomId":-1},{"roomId":-2}]), json!("second")]);
    let args = payload_args(&payload);
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].as_array().unwrap().len(), 2);
    assert_eq!(args[1], "second");
}

#[test]
fn dedicated_and_gui_events_are_not_delivered_twice_by_on_any() {
    for event in [
        "addMessage",
        "deleteMessage",
        "renewMessage",
        "renewMessageURL",
        "setMessages",
        "setAllRooms",
        "updateRoom",
        "setAllChatGroups",
        "syncRead",
        "login-verify",
        "login-qrcodeLogin",
        "login-smsCodeVerify",
        "login-slider",
        "messageError",
        "requestSetup",
        "requireAuth",
        "authSucceed",
        "notify",
        "notifyMessage",
        "addMessageText",
        "closeLoading",
    ] {
        assert!(forwarded_event_name(&Event::Custom(event.into())).is_none(), "{event}");
    }
    assert!(forwarded_event_name(&Event::Message).is_none());
}

#[test]
fn unknown_non_gui_business_event_is_available_to_plugins() {
    let event = Event::Custom("businessResult".into());
    assert_eq!(forwarded_event_name(&event), Some("businessResult"));
    assert!(forwarded_event_name(&Event::Connect).is_none());
    assert!(forwarded_event_name(&Event::Close).is_none());
}

#[test]
fn empty_payload_cannot_enter_a_state_reducer() {
    let payload = Payload::Text(Vec::new());
    assert!(first_arg(&payload).is_none());
    assert!(payload_args(&payload).is_empty());
}
