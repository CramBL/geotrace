use geotrace_sdk::EventKind;

#[derive(Debug, EventKind)]
enum PowerEvent {
    Boot,
}

#[derive(Debug, EventKind)]
enum DeviceEvent {
    #[event_kind(delegate)]
    Power(PowerEvent, u8),
}

fn main() {
    let DeviceEvent::Power(_power_event, _count) = DeviceEvent::Power(PowerEvent::Boot, 1);
}
