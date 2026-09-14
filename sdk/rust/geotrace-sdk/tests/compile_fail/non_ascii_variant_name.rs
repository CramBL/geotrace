use geotrace_sdk::EventKind;

#[derive(Debug, EventKind)]
enum MeasurementEvent {
    Größe,
}

fn main() {
    let _event = MeasurementEvent::Größe;
}
