use geotrace_sdk::EventKind;

#[derive(Debug, EventKind)]
enum StorageEvent {
    IOError,
    IoError,
}

fn main() {
    let _events = [StorageEvent::IOError, StorageEvent::IoError];
}
