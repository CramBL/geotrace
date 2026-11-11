use chrono::{DateTime, Utc};
use gt_logfile::EligibleLogTime;

fn construct(reset_time: DateTime<Utc>) {
    let _ = EligibleLogTime(reset_time);
}

fn main() {}
