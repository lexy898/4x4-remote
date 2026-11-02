//! The texts of the Files tab that carry values: the names screen readers hear for the parts of a
//! row, the priority a row shows, its progress in percent and the "Playable from start" line.
//!
//! The switch, the name of the list and the line of a magnet link without metadata are plain
//! messages, which the tab fetches itself.

use gpui_kit::SharedString;
use x4_core::{Playable, PriorityView};
use x4_rpc::FilePriority;

use super::{size_string, t};

/// The name of the checkbox of the file or folder `name`, such as "Download Season 1".
pub(crate) fn files_wanted_label(name: &str) -> SharedString {
    t!("files-wanted", name = name).into()
}

/// The priority a row shows: "Low", "Normal" or "High", or "Mixed" for a folder whose files
/// differ.
pub(crate) fn priority_text(priority: PriorityView) -> SharedString {
    priority_string(priority).into()
}

fn priority_string(priority: PriorityView) -> String {
    match priority {
        PriorityView::One(FilePriority::Low) => t!("files-priority-low"),
        PriorityView::One(FilePriority::Normal) => t!("files-priority-normal"),
        PriorityView::One(FilePriority::High) => t!("files-priority-high"),
        PriorityView::Mixed => t!("files-priority-mixed"),
    }
}

/// The name of the priority button of the file or folder `name`, with the priority it shows,
/// such as "Priority of Season 1: Mixed".
pub(crate) fn priority_label(name: &str, priority: PriorityView) -> SharedString {
    t!(
        "files-priority-label",
        name = name,
        priority = priority_string(priority)
    )
    .into()
}

/// The name of the progress bar of the file or folder `name`, such as "Progress of E01.mkv".
pub(crate) fn file_progress_label(name: &str) -> SharedString {
    t!("files-progress-label", name = name).into()
}

/// Progress in whole percent, such as "65%".
pub(crate) fn percent_text(percent: u8) -> SharedString {
    t!("files-percent", percent = percent).into()
}

/// The line under a file that downloads, such as "Playable from start: 12.4 GiB (34%)".
pub(crate) fn playable_text(playable: Playable) -> SharedString {
    t!(
        "files-playable",
        size = size_string(playable.size),
        percent = playable.percent
    )
    .into()
}

#[cfg(test)]
mod tests {
    use x4_core::human_size;

    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn a_row_names_its_parts_after_the_file() {
        assert_eq!(files_wanted_label("Season 1"), "Download Season 1");
        assert_eq!(file_progress_label("E01.mkv"), "Progress of E01.mkv");
        assert_eq!(
            priority_label("Season 1", PriorityView::Mixed),
            "Priority of Season 1: Mixed"
        );
        assert_eq!(
            priority_label("E01.mkv", PriorityView::One(FilePriority::High)),
            "Priority of E01.mkv: High"
        );
    }

    #[test]
    fn each_priority_has_its_word_and_a_folder_of_several_is_mixed() {
        assert_eq!(priority_text(PriorityView::One(FilePriority::Low)), "Low");
        assert_eq!(
            priority_text(PriorityView::One(FilePriority::Normal)),
            "Normal"
        );
        assert_eq!(priority_text(PriorityView::One(FilePriority::High)), "High");
        assert_eq!(priority_text(PriorityView::Mixed), "Mixed");
    }

    #[test]
    fn progress_and_the_playable_part_are_in_whole_percent() {
        assert_eq!(percent_text(0), "0%");
        assert_eq!(percent_text(65), "65%");
        assert_eq!(
            playable_text(Playable {
                bytes: 20 * MIB,
                size: human_size(20 * MIB),
                percent: 20,
            }),
            "Playable from start: 20.0 MiB (20%)"
        );
    }
}
