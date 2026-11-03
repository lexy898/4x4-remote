//! The file dialog that chooses the `.torrent` files to add, behind the [`FilePicker`] seam.
//!
//! - On Linux the app uses the desktop's own file dialog through the xdg-desktop-portal
//!   ([`PortalPicker`]): several files at once, a "Torrent files" filter for `*.torrent` and
//!   `application/x-bittorrent`, and a title in the user's language. It is parented to the window
//!   and modal to it, so it stays over 4x4: the portal gets the window's X11 id, or, on Wayland,
//!   the handle of its surface exported through xdg-foreign. A window it cannot name gets a
//!   dialog without a parent. The call goes over D-Bus and is awaited without blocking the UI
//!   thread.
//! - Elsewhere, and in tests, gpui's own dialog ([`PromptPicker`]), which has no filter and gpui's
//!   English title; on the test platform the test answers it.
//!
//! Every local path chosen joins the queue: a file that is no torrent is refused when its turn
//! comes. A file the dialog names that is not on this computer is counted, for the user to be
//! told. A cancelled dialog is no error.

use gpui_kit::{App, PathPromptOptions, Task, Window};
use x4_core::ChosenFiles;

/// What a file dialog answered: the files chosen ([`ChosenFiles`]); `None` when the user
/// cancelled it; or why it could not be shown.
pub type Picked = Result<Option<ChosenFiles>, PickError>;

/// Asks the user for the `.torrent` files to add.
pub trait FilePicker {
    /// Shows the file dialog over `window`, which lets the user choose several files, and gives a
    /// task that resolves once the user answers. The task fails with a [`PickError`] when the
    /// dialog cannot be shown, as when the desktop has no file chooser.
    fn pick(&self, window: &Window, cx: &App) -> Task<Picked>;
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
/// gpui's English title, over the window that has the keyboard; on the test platform a prompt the
/// test answers. gpui gives only the local files it chose.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PromptPicker;

impl FilePicker for PromptPicker {
    fn pick(&self, _window: &Window, cx: &App) -> Task<Picked> {
        let answer = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.foreground_executor().spawn(async move {
            match answer.await {
                Ok(Ok(paths)) => Ok(paths.map(|paths| ChosenFiles {
                    paths,
                    not_local: 0,
                })),
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
    use ashpd::desktop::ResponseError;
    use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
    use ashpd::{Uri, WindowIdentifier};
    use gpui_kit::{App, Task, Window};
    use raw_window_handle::{
        HandleError, HasDisplayHandle as _, HasWindowHandle as _, RawDisplayHandle, RawWindowHandle,
    };
    use x4_core::chosen_files;

    use super::{FilePicker, PickError, Picked};
    use crate::i18n::t;

    /// The MIME type of a `.torrent` file, which the dialog's filter shows.
    const TORRENT_MIME_TYPE: &str = "application/x-bittorrent";
    /// The names the dialog's filter shows, for files the desktop knows no type of.
    const TORRENT_GLOB: &str = "*.torrent";

    /// The desktop's own file dialog, through the `FileChooser` portal: several files at once, a
    /// "Torrent files" filter, chosen from the start, and a title in the user's language, modal to
    /// the window it opens over.
    #[derive(Debug, Clone, Copy, Default)]
    pub(crate) struct PortalPicker;

    impl FilePicker for PortalPicker {
        fn pick(&self, window: &Window, cx: &App) -> Task<Picked> {
            let title = t!("add-file-picker-title");
            let filter = t!("add-file-picker-filter");
            // On the UI thread, as gpui does for its own dialog: the raw handles are the window's
            // own pointers, which must not leave it.
            window.spawn(cx, async move |cx| {
                let handles = cx.update(|window, _| {
                    (
                        window.window_handle().map(|handle| handle.as_raw()),
                        window.display_handle().map(|handle| handle.as_raw()),
                    )
                });
                let parent = match handles {
                    // In the poll that read the handles, while the window is sure to have them.
                    Ok((window, display)) => parent(window, display).await,
                    Err(error) => {
                        tracing::debug!(%error, "the file dialog opens without a parent: no window");
                        None
                    }
                };
                pick(parent, &title, &filter).await
            })
        }
    }

    /// What the portal parents the dialog to: the identifier of the window whose raw handles are
    /// `window` and `display`, the X11 id of the window, or on Wayland a handle of its surface
    /// that ashpd exports through xdg-foreign. `None`, so that the dialog opens without a parent, when
    /// the window has no handle or none the portal knows, or the export fails.
    ///
    /// Await it in the poll that read the handles: ashpd reads a Wayland window's pointers in its
    /// first poll.
    async fn parent(
        window: Result<RawWindowHandle, HandleError>,
        display: Result<RawDisplayHandle, HandleError>,
    ) -> Option<WindowIdentifier> {
        let window = match window {
            Ok(window) => window,
            Err(error) => {
                tracing::debug!(%error, "the file dialog opens without a parent: the window has no handle");
                return None;
            }
        };
        let identifier = WindowIdentifier::from_raw_handle(&window, display.ok().as_ref()).await;
        if let Some(identifier) = &identifier {
            tracing::debug!(%identifier, "the file dialog opens over the window");
        } else {
            tracing::debug!(
                ?window,
                "the file dialog opens without a parent: the window cannot be named to the portal"
            );
        }
        identifier
    }

    /// Asks the portal for `.torrent` files, in a dialog titled `title` whose filter is named
    /// `filter`, modal to `parent` where there is one, and waits for the user's answer. The
    /// request owns `parent` until then: a Wayland handle is unexported when it is dropped.
    async fn pick(parent: Option<WindowIdentifier>, title: &str, filter: &str) -> Picked {
        let filter = FileFilter::new(filter)
            .mimetype(TORRENT_MIME_TYPE)
            .glob(TORRENT_GLOB);
        let answer = SelectedFiles::open_file()
            .identifier(parent)
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

    /// What the portal's `answer` comes to: the files of its URIs, in order, with a count of
    /// those not on this computer; `None` when the user cancelled the dialog or it ended
    /// otherwise; or the error.
    fn answered(answer: Result<Vec<Uri>, ashpd::Error>) -> Picked {
        match answer {
            Ok(uris) => {
                let chosen = chosen_files(uris.iter().map(Uri::as_str));
                if chosen.not_local > 0 {
                    tracing::warn!(
                        not_local = chosen.not_local,
                        "the file dialog chose files that are not on this computer"
                    );
                    tracing::debug!(?uris, "the files the file dialog chose");
                }
                Ok(Some(chosen))
            }
            Err(ashpd::Error::Response(ResponseError::Cancelled | ResponseError::Other)) => {
                Ok(None)
            }
            Err(error) => Err(PickError::new(error.to_string())),
        }
    }

    #[cfg(test)]
    mod tests {
        use std::num::{NonZeroIsize, NonZeroU32};
        use std::path::PathBuf;
        use std::pin::pin;
        use std::ptr::NonNull;
        use std::task::{Context, Poll, Waker};

        use raw_window_handle::{
            WaylandWindowHandle, Win32WindowHandle, XcbWindowHandle, XlibDisplayHandle,
            XlibWindowHandle,
        };
        use x4_core::ChosenFiles;

        use super::*;

        fn uri(text: &str) -> Uri {
            Uri::parse(text).unwrap()
        }

        /// The output of `future`, which must be ready in its first poll, as the identifier of an
        /// X11 window, or none, is.
        fn at_once<F: Future>(future: F) -> F::Output {
            let mut future = pin!(future);
            match future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
            {
                Poll::Ready(output) => output,
                Poll::Pending => panic!("the future waits"),
            }
        }

        /// What the portal is told to parent the dialog to, for the raw handles `window` and
        /// `display`.
        fn parent_text(
            window: Result<RawWindowHandle, HandleError>,
            display: Result<RawDisplayHandle, HandleError>,
        ) -> Option<String> {
            at_once(parent(window, display)).map(|identifier| identifier.to_string())
        }

        #[test]
        fn the_dialog_is_parented_to_an_x11_window_by_its_id() {
            let display = Ok(XlibDisplayHandle::new(None, 0).into());
            // gpui names its X11 windows through XCB.
            let xcb = XcbWindowHandle::new(NonZeroU32::new(0x2a0_0007).unwrap());
            assert_eq!(
                parent_text(Ok(xcb.into()), display),
                Some("x11:2a00007".to_owned())
            );
            let xlib = XlibWindowHandle::new(0x2a);
            assert_eq!(
                parent_text(Ok(xlib.into()), Err(HandleError::Unavailable)),
                Some("x11:2a".to_owned()),
                "an X11 window needs no display handle"
            );
        }

        #[test]
        fn a_window_the_portal_cannot_name_gets_a_dialog_without_a_parent() {
            let display = || Ok(XlibDisplayHandle::new(None, 0).into());
            assert_eq!(parent_text(Err(HandleError::Unavailable), display()), None);
            let win32 = Win32WindowHandle::new(NonZeroIsize::new(1).unwrap());
            assert_eq!(parent_text(Ok(win32.into()), display()), None);
            // A Wayland surface without a Wayland display is never read.
            let wayland = WaylandWindowHandle::new(NonNull::dangling());
            assert_eq!(parent_text(Ok(wayland.into()), display()), None);
            assert_eq!(
                parent_text(Ok(wayland.into()), Err(HandleError::Unavailable)),
                None
            );
        }

        #[test]
        fn the_portal_gives_the_local_files_it_chose_in_order_and_counts_the_others() {
            assert_eq!(
                answered(Ok(vec![
                    uri("file:///home/user/b.torrent"),
                    uri("sftp://host/a.torrent"),
                    uri("file:///home/user/My%20Shows/a.torrent"),
                ])),
                Ok(Some(ChosenFiles {
                    paths: vec![
                        PathBuf::from("/home/user/b.torrent"),
                        PathBuf::from("/home/user/My Shows/a.torrent"),
                    ],
                    not_local: 1,
                }))
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
