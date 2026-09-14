use geotrace_sdk::EventKind;

#[derive(Debug, EventKind)]
enum PowerEvent {
    #[event_kind(rename = "power/boot")]
    Boot,
}

fn main() {
    let _event = PowerEvent::Boot;
}
