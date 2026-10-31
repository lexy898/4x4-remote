//! The General tab: label and value rows for x4-core's [`GeneralModel`], in two columns. The
//! left one is about the data (where it is, its size and hash, what moved), the right one about
//! the torrent (when it came and was made, by what, and how its trackers are doing). A problem
//! the daemon reports for the torrent comes first, across both.
//!
//! What only the details know (the totals, the metainfo, the trackers) says "—" until they have
//! been read. The path and the hash each have a copy button. Every row carries its label and its
//! value for screen readers.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, Theme, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div, px,
};
use x4_core::{ErrorView, GeneralModel, TrackerProblemKind};

use super::{CopyRequest, DetailsPanel, DetailsPanelEvent};
use crate::i18n::{
    created_text, date_time_text, error_texts, pieces_text, ratio_text, size_text,
    size_with_wanted_text, t, tracker_problem_text, trackers_text,
};

/// The width of the labels, room for "Downloaded" and "Completed" in the panel's text size.
const LABEL_WIDTH: f32 = 92.;

/// One row: its element id, its label and its value as drawn and as read out, and what else it
/// holds.
struct Row {
    id: &'static str,
    label: SharedString,
    value: SharedString,
    /// The copy button after the value, with its element id, its name and what it copies.
    copy: Option<(&'static str, SharedString, &'static str)>,
    /// A line under the value, in its own color, with its element id.
    note: Option<(&'static str, SharedString, Hsla)>,
    /// The color of the value; the theme's text color when `None`.
    color: Option<Hsla>,
}

impl Row {
    fn new(id: &'static str, label: SharedString, value: SharedString) -> Self {
        Self {
            id,
            label,
            value,
            copy: None,
            note: None,
            color: None,
        }
    }
}

/// The General tab for `model`. Its copy buttons ask the panel of `cx` to copy.
pub(super) fn general_tab(model: &GeneralModel, cx: &Context<'_, DetailsPanel>) -> AnyElement {
    let theme = cx.theme();
    let (left, right) = rows(model, theme);
    let column = |rows: Vec<Row>| {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .children(rows.into_iter().map(|row| draw(row, theme, cx)))
    };
    v_flex()
        .gap_2()
        .px_3()
        .py_2()
        .children(
            model
                .error
                .as_ref()
                .map(|error| draw(error_row(error, theme), theme, cx)),
        )
        .child(
            h_flex()
                .items_start()
                .gap_6()
                .child(column(left))
                .child(column(right)),
        )
        .into_any_element()
}

/// The rows of the two columns for `model`.
fn rows(model: &GeneralModel, theme: &Theme) -> (Vec<Row>, Vec<Row>) {
    (data_rows(model), torrent_rows(model, theme))
}

/// `value`, which only the details hold, once they have been read (`loaded`); "—" until then.
fn read(loaded: bool, value: impl FnOnce() -> SharedString) -> SharedString {
    if loaded {
        value()
    } else {
        t!("details-pending").into()
    }
}

/// The left column: where the data is, its size and hash, and what moved.
fn data_rows(model: &GeneralModel) -> Vec<Row> {
    let loaded = model.details_loaded;
    vec![
        Row {
            copy: Some(("copy-path", t!("general-copy-path").into(), "path")),
            ..Row::new(
                "general-path",
                t!("general-path").into(),
                model.path.clone().into(),
            )
        },
        Row {
            copy: Some(("copy-hash", t!("general-copy-hash").into(), "hash")),
            ..Row::new(
                "general-hash",
                t!("general-hash").into(),
                model.hash.as_str().to_owned().into(),
            )
        },
        Row::new(
            "general-size",
            t!("general-size").into(),
            size_with_wanted_text(model.size_total, model.size_wanted),
        ),
        Row::new(
            "general-pieces",
            t!("general-pieces").into(),
            read(loaded, || {
                model
                    .pieces
                    .map_or_else(|| t!("general-none").into(), pieces_text)
            }),
        ),
        Row::new(
            "general-downloaded",
            t!("general-downloaded").into(),
            read(loaded, || size_text(model.downloaded)),
        ),
        Row::new(
            "general-uploaded",
            t!("general-uploaded").into(),
            read(loaded, || size_text(model.uploaded)),
        ),
        Row::new(
            "general-ratio",
            t!("general-ratio").into(),
            ratio_text(model.ratio),
        ),
    ]
}

/// The right column: how the trackers are doing, when the torrent came and was made, by what,
/// and what its metainfo says.
fn torrent_rows(model: &GeneralModel, theme: &Theme) -> Vec<Row> {
    let loaded = model.details_loaded;
    let mut rows = vec![
        Row {
            note: model.trackers.problem.as_ref().map(|problem| {
                let color = match problem.kind {
                    TrackerProblemKind::AnnounceFailed | TrackerProblemKind::AnnounceTimedOut => {
                        theme.warning
                    }
                    // Many trackers never answer scrapes, so the mildest kind is said quietly.
                    TrackerProblemKind::ScrapeFailed | TrackerProblemKind::ScrapeTimedOut => {
                        theme.muted_foreground
                    }
                };
                ("tracker-problem", tracker_problem_text(problem), color)
            }),
            ..Row::new(
                "general-trackers",
                t!("general-trackers").into(),
                read(loaded, || trackers_text(&model.trackers)),
            )
        },
        Row::new(
            "general-added",
            t!("general-added").into(),
            model
                .added
                .map_or_else(|| t!("general-none").into(), date_time_text),
        ),
        Row::new(
            "general-done",
            t!("general-done").into(),
            model
                .done
                .map_or_else(|| t!("general-not-done").into(), date_time_text),
        ),
        Row::new(
            "general-created",
            t!("general-created").into(),
            read(loaded, || created_text(&model.creator, model.created)),
        ),
    ];
    // Only private trackers set a source, so it is shown only when there is one.
    if let Some(source) = &model.source {
        rows.push(Row::new(
            "general-source",
            t!("general-source").into(),
            source.clone().into(),
        ));
    }
    rows.push(Row::new(
        "general-comment",
        t!("general-comment").into(),
        read(loaded, || {
            model
                .comment
                .clone()
                .map_or_else(|| t!("general-none").into(), SharedString::from)
        }),
    ));
    rows.push(Row::new(
        "general-private",
        t!("general-private").into(),
        read(loaded, || {
            if model.private {
                t!("general-yes")
            } else {
                t!("general-no")
            }
            .into()
        }),
    ));
    rows
}

/// The row of the problem the daemon reports, in the color of an error or a warning.
fn error_row(error: &ErrorView, theme: &Theme) -> Row {
    let (label, value) = error_texts(error);
    let color = match error {
        ErrorView::LocalError(_) | ErrorView::TrackerError(_) => theme.danger,
        ErrorView::TrackerWarning(_) => theme.warning,
    };
    Row {
        color: Some(color),
        ..Row::new("general-error", label, value)
    }
}

/// `row` as drawn: the label, muted, then the value, with its copy button and its note.
fn draw(row: Row, theme: &Theme, cx: &Context<'_, DetailsPanel>) -> AnyElement {
    let copy = row.copy.map(|(id, name, what)| {
        let text = row.value.to_string();
        Button::new(id)
            .ghost()
            .xsmall()
            .icon(IconName::Copy)
            .accessibility_label(name.clone())
            .tooltip(name)
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(DetailsPanelEvent::Copy(CopyRequest {
                    text: text.clone(),
                    what,
                }));
            }))
    });
    let note = row.note.map(|(id, text, color)| {
        div()
            .id(id)
            .text_color(color)
            .aria_label(text.clone())
            .child(text)
            .test_support()
    });
    h_flex()
        .id(row.id)
        .items_start()
        .gap_2()
        .aria_label(row.label.clone())
        .aria_value(row.value.clone())
        .child(
            div()
                .w(px(LABEL_WIDTH))
                .flex_none()
                .text_color(theme.muted_foreground)
                .child(row.label),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    h_flex()
                        .items_start()
                        .gap_1()
                        .child(
                            div()
                                .min_w_0()
                                .text_color(row.color.unwrap_or(theme.foreground))
                                .child(row.value),
                        )
                        .children(copy),
                )
                .children(note),
        )
        .test_support()
        .into_any_element()
}
