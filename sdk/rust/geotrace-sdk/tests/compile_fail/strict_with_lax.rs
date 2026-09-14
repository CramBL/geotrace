use geotrace_sdk::EventKind;

#[derive(Debug, EventKind)]
#[event_kind(strict, lax)]
enum PowerEvent {
    Boot,
}

fn main() {
    let _event = PowerEvent::Boot;
}
