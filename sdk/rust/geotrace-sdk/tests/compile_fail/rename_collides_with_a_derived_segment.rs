use geotrace_sdk::EventKind;

#[derive(Debug, EventKind)]
enum PowerEvent {
    Boot,
    #[event_kind(rename = "boot")]
    ColdStart,
}

fn main() {
    let _events = [PowerEvent::Boot, PowerEvent::ColdStart];
}
