use geotrace_sdk::EventKind;

#[derive(Debug, EventKind)]
#[event_kind(note = display, note = none)]
enum PowerEvent {
    Boot,
}

fn main() {
    let _event = PowerEvent::Boot;
}
