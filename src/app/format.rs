//! Value formatting shared across the app's windows.

/// A byte count for display, in binary units.
///
/// Zero renders as an em dash, the table convention for "nothing here".
pub(crate) fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return gt_ui_theme::EM_DASH.to_owned();
    }
    if bytes < 1_024 {
        return format!("{bytes} B");
    }
    if bytes < 1_024 * 1_024 {
        let kb = bytes as f64 / 1_024.0;
        return format!("{kb:.1} KB");
    }
    let mb = bytes as f64 / (1_024.0 * 1_024.0);
    format!("{mb:.1} MB")
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::nothing(0)]
    #[case::bytes(512)]
    #[case::exactly_one_kib(1024)]
    #[case::kilobytes(1536)]
    #[case::exactly_one_mib(1024 * 1024)]
    #[case::a_day_of_interference(81 * 1024)]
    #[case::the_whole_interference_archive(1600 * 81 * 1024)]
    fn sizes_read_the_same_everywhere(#[case] bytes: u64) {
        insta::assert_snapshot!(format!("format_size_{bytes}"), format_size(bytes));
    }
}
