use chrono::{DateTime, Utc};
use gt_logfile::RecordingAssociationIndex;
use gt_types::AssociationWindow;

fn associate(index: RecordingAssociationIndex<'_>, reset_time: DateTime<Utc>) {
    index.position_at(reset_time, AssociationWindow::ZERO);
}

fn main() {}
