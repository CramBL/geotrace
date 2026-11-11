use chrono::{DateTime, Duration, Utc};
use gt_logfile::RecordingAssociationIndex;

fn associate(index: RecordingAssociationIndex<'_>, reset_time: DateTime<Utc>) {
    index.position_at(reset_time, Duration::seconds(60));
}

fn main() {}
