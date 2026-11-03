//! The file dialog that chooses the `.torrent` files to add, behind the [`FilePicker`] seam.
//!
//! - On Linux the app uses the desktop's own file dialog through the xdg-desktop-portal
//!   ([`PortalPicker`]): several files at once, a "Torrent files" filter for `*.torrent` and
//!   `application/x-bittorrent`, and a title in the user's language. The call goes over D-Bus and
//!   waits for the user's answer on the background executor, never on the UI thread.
//! - Elsewhere, and in tests, gpui's own dialog ([`PromptPicker`]), which has no filter and gpui's
//!   English title; on the test platform the test answers it.
//!
//! Every path chosen joins the queue: a file that is no torrent is refused when its turn comes. A
//! cancelled dialog is no error. The portal's dialog is not attached to the window: that takes a
//! raw window handle, which gpui gives only through a crate and an ashpd feature 4x4 does not use.

use std::path::PathBuf;

use gpui_kit::{App, PathPromptOptions, Task};

/// What a file dialog answered: the paths chosen, in the order the dialog gave them; `None` when
/// the user cancelled it; or why it could not be shown.
pub type Picked = Result<Option<Vec<PathBuf>>, PickError>;

/// Asks the user for the `.torrent` files to add.
pub trait FilePicker {
    /// Shows the file dialog, which lets the user choose several files, and gives a task that
    /// resolves once the user answers. The task fails with a [`PickError`] when the dialog cannot
    /// be shown, as when the desktop has no file chooser.
    fn pick(&self, cx: &App) -> Task<Picked>;
}

/// Why a file dialog could not be shown, or failed, as the platform said it; for the log, never
/// for the user, who gets a message of their own.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the file dialog failed: {reason}")]
pub struct PickError {
    /// What the platform said.
    reason: String,
}

impl PickError {
    /// A failure of the file dialog, which the platform describes as `reason`.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// gpui's own file dialog: on Linux the same portal as [`PortalPicker`], without a filter and with
/// gpui's English title; on the test platform a prompt the test answers.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PromptPicker;

impl FilePicker for PromptPicker {
    fn pick(&self, cx: &App) -> Task<Picked> {
        let answer = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.foreground_executor().spawn(async move {
            match answer.await {
                Ok(Ok(paths)) => Ok(paths),
                Ok(Err(error)) => Err(PickError::new(format!("{error:#}"))),
                Err(error) => Err(PickError::new(format!(
                    "the file dialog ended without an answer: {error}"
                ))),
            }
        })
    }
}

#[cfg(target_os = "linux")]
pub(crate) use portal::PortalPicker;

#[cfg(target_os = "linux")]
mod portal {
    use ashpd::Uri;
    use ashpd::desktop::ResponseError;
    use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
    use gpui_kit::{App, Task};
    use x4_core::chosen_file;

    use super::{FilePicker, PickError, Picked};
    use crate::i18n::t;

    /// The MIME type of a `.torrent` file, which the dialog's filter shows.
    const TORRENT_MIME_TYPE: &str = "application/x-bittorrent";
    /// The names the dialog's filter shows, for files the desktop knows no type of.
    const TORRENT_GLOB: &str = "*.torrent";

    /// The desktop's own file dialog, through the `FileChooser` portal: several files at once, a
    /// "Torrent files" filter, chosen from the start, and a title in the user's language.
    #[derive(Debug, Clone, Copy, Default)]
    pub(crate) struct PortalPicker;

    impl FilePicker for PortalPicker {
        fn pick(&self, cx: &App) -> Task<Picked> {
            let title = t!("add-file-picker-title");
            let filter = t!("add-file-picker-filter");
            cx.background_executor()
                .spawn(async move { pick(&title, &filter).await })
        }
    }

    /// Asks the portal for `.torrent` files, in a dialog titled `title` whose filter is named
    /// `filter`, and waits for the user's answer.
    async fn pick(title: &str, filter: &str) -> Picked {
        let filter = FileFilter::new(filter)
            .mimetype(TORRENT_MIME_TYPE)
            .glob(TORRENT_GLOB);
        let answer = SelectedFiles::open_file()
            .title(title)
            .modal(true)
            .multiple(true)
            .filter(filter.clone())
            .current_filter(filter)
            .send()
            .await
            .and_then(|request| request.response());
        answered(answer.map(|files| files.uris().to_vec()))
    }

    /// What the portal's `answer` comes to: the local files of its URIs, in order; `None` when the
    /// user cancelled the dialog or it ended otherwise; or the error.
    fn answered(answer: Result<Vec<Uri>, ashpd::Error>) -> Picked {
        match answer {
            Ok(uris) => {
                let paths: Vec<_> = uris
                    .iter()
                    .filter_map(|uri| chosen_file(uri.as_str()))
                    .collect();
                let skipped = uris.len() - paths.len();
                if skipped > 0 {
                    tracing::warn!(skipped, "the file dialog chose files that are not local");
                    tracing::debug!(?uris, "the files the file dialog chose");
                }
                Ok(Some(paths))
            }
            Err(ashpd::Error::Response(ResponseError::Cancelled | ResponseError::Other)) => {
                Ok(None)
            }
            Err(error) => Err(PickError::new(error.to_string())),
        }
    }

    #[cfg(test)]
    mod tests {
        use std::path::PathBuf;

        use super::*;

        fn uri(text: &str) -> Uri {
            Uri::parse(text).unwrap()
        }

        #[test]
        fn the_portal_gives_the_local_files_it_chose_in_order() {
            assert_eq!(
                answered(Ok(vec![
                    uri("file:///home/user/b.torrent"),
                    uri("sftp://host/a.torrent"),
                    uri("file:///home/user/My%20Shows/a.torrent"),
                ])),
                Ok(Some(vec![
                    PathBuf::from("/home/user/b.torrent"),
                    PathBuf::from("/home/user/My Shows/a.torrent"),
                ]))
            );
        }

        #[test]
        fn a_cancelled_dialog_is_no_error_and_a_failed_one_is() {
            for ended in [ResponseError::Cancelled, ResponseError::Other] {
                assert_eq!(answered(Err(ashpd::Error::Response(ended))), Ok(None));
            }
            assert!(answered(Err(ashpd::Error::NoResponse)).is_err());
        }
    }
}
