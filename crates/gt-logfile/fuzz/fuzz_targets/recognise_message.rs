#![no_main]

use gt_logfile::{
    recognise_message, HostnameColumn, RecognisedLevel, RecognisedMessage, RecognisedService,
};
use libfuzzer_sys::fuzz_target;

// Feed one arbitrary message to the recogniser, read as both layouts a log can
// have. Every span it reads must slice that message and stand where the layout
// puts it, whatever the message holds.
fuzz_target!(|message: &str| {
    for hostname_column in [HostnameColumn::Present, HostnameColumn::Absent] {
        check_the_spans_slice_the_message(message, recognise_message(message, hostname_column));
    }
});

fn check_the_spans_slice_the_message(message: &str, read: RecognisedMessage) {
    let hostname = read.hostname();
    let service = read.service().map(RecognisedService::span);
    let level = read.level().map(RecognisedLevel::span);

    let spans = [hostname.clone(), service.clone(), level.clone()];
    for span in spans.into_iter().flatten() {
        assert!(
            message.get(span.clone()).is_some(),
            "{span:?} lies outside {message:?}"
        );
    }
    if let (Some(hostname), Some(service)) = (&hostname, &service) {
        assert!(
            hostname.end < service.start,
            "the host {hostname:?} runs into the service {service:?}"
        );
    }
    if let (Some(service), Some(level)) = (&service, &level) {
        assert!(
            level.start >= service.end,
            "the level {level:?} runs into the service {service:?}"
        );
    }
    if let Some(name) = service.and_then(|span| message.get(span)) {
        assert!(!name.contains(' '), "the service {name:?} holds a space");
    }
}
