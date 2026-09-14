use geotrace_sdk::EventKind;

#[derive(Debug, EventKind)]
enum PowerEvent {
    #[event_kind(icon = Check)]
    Boot,
}

#[derive(Debug, EventKind)]
enum DeviceEvent {
    #[event_kind(icon = Error)]
    Power(PowerEvent),
}

fn main() {
    let DeviceEvent::Power(_power_event) = DeviceEvent::Power(PowerEvent::Boot);
}
