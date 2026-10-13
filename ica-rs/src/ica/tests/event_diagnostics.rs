use super::{EventDiagnostics, REPORT_INTERVAL};
use std::time::{Duration, Instant};

#[test]
fn repeated_invalid_events_are_aggregated_at_the_window_boundary() {
    let now = Instant::now();
    let mut diagnostics = EventDiagnostics::default();
    assert_eq!(diagnostics.record_invalid("addMessage", now), Some(1));
    for second in 1..30 {
        assert_eq!(
            diagnostics.record_invalid("addMessage", now + Duration::from_secs(second)),
            None
        );
    }
    assert_eq!(diagnostics.record_invalid("addMessage", now + REPORT_INTERVAL), Some(30));
    assert_eq!(diagnostics.record_invalid("addMessage", now + REPORT_INTERVAL), None);
    assert_eq!(diagnostics.record_invalid("addMessage", now + REPORT_INTERVAL * 2), Some(2));
}

#[test]
fn one_noisy_event_does_not_hide_another_events_first_warning() {
    let now = Instant::now();
    let mut diagnostics = EventDiagnostics::default();
    assert_eq!(diagnostics.record_invalid("addMessage", now), Some(1));
    assert_eq!(diagnostics.record_invalid("addMessage", now), None);
    assert_eq!(diagnostics.record_invalid("renewMessage", now), Some(1));
    assert_eq!(diagnostics.record_invalid("renewMessage", now), None);
    assert_eq!(diagnostics.record_invalid("addMessage", now + REPORT_INTERVAL), Some(2));
    assert_eq!(diagnostics.record_invalid("renewMessage", now + REPORT_INTERVAL), Some(2));
}

#[test]
fn diagnostic_windows_are_isolated_between_bot_instances() {
    let now = Instant::now();
    let mut first = EventDiagnostics::default();
    let mut second = EventDiagnostics::default();
    assert_eq!(first.record_invalid("addMessage", now), Some(1));
    assert_eq!(first.record_invalid("addMessage", now), None);
    assert_eq!(second.record_invalid("addMessage", now), Some(1));
    assert_eq!(first.record_invalid("addMessage", now + REPORT_INTERVAL), Some(2));
    assert_eq!(second.record_invalid("addMessage", now + REPORT_INTERVAL), Some(1));
}
