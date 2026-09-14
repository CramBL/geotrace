use geotrace_sdk::EventKind;

#[derive(Debug, EventKind)]
enum PowerEvent {
    Boot,
}

#[derive(Debug, EventKind)]
enum DeviceEvent {
    #[event_kind(leaf, delegate)]
    Power(PowerEvent),
}

fn main() {
    let DeviceEvent::Power(_power_event) = DeviceEvent::Power(PowerEvent::Boot);
}
