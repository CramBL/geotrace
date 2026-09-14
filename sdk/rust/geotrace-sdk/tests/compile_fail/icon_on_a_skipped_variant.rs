use geotrace_sdk::EventKind;

#[derive(Debug, EventKind)]
enum PowerEvent {
    #[event_kind(skip, icon = Check)]
    Boot,
}

fn main() {
    let _event = PowerEvent::Boot;
}
