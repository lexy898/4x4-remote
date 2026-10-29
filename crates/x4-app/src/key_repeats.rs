//! Tells a press of a key from the repeats the system sends while the key is held, so that holding
//! the key of an action on torrents sends the action once.
//!
//! A key's binding runs its action before any key listener sees the key-down, and what runs before
//! the binding, a keystroke interceptor, is not told whether the key is held: the event's
//! `is_held` mark does not reach it (and only Wayland sets it; X11 never does). So the main view
//! pairs key-downs with key-ups instead: a key-down of the key that went down last, with no key-up
//! since, is a repeat. Neither Wayland nor X11 sends a key-up between the repeats of a held key
//! (GPUI drops the ones X11 makes up).

use gpui_kit::Keystroke;

/// The key held down now, and whether the last key-down repeated it.
#[derive(Debug, Default)]
pub(crate) struct KeyRepeats {
    /// The key that went down last and has not gone up since.
    held: Option<Keystroke>,
    /// Whether the last key-down repeated `held`.
    repeating: bool,
}

impl KeyRepeats {
    /// Takes a key-down of `keystroke`, before its binding runs its action.
    pub(crate) fn key_down(&mut self, keystroke: &Keystroke) {
        self.repeating = self
            .held
            .as_ref()
            .is_some_and(|held| same_key(held, keystroke));
        self.held = Some(keystroke.clone());
    }

    /// The key held down now: the one that went down last and has not gone up since.
    pub(crate) fn held(&self) -> Option<&Keystroke> {
        self.held.as_ref()
    }

    /// Forgets the held key: it went up, or its key-up may never arrive, as when the key was let go
    /// while the focus was elsewhere.
    pub(crate) fn release(&mut self) {
        self.held = None;
        self.repeating = false;
    }

    /// Whether the last key-down repeated a held key, so that the action its binding runs must
    /// not run again.
    pub(crate) fn repeating(&self) -> bool {
        self.repeating
    }
}

/// Whether `first` and `second` are the same key with the same modifiers, whatever text they type.
pub(crate) fn same_key(first: &Keystroke, second: &Keystroke) -> bool {
    first.key == second.key && first.modifiers == second.modifiers
}

#[cfg(test)]
mod tests {
    use gpui_kit::OperatingSystem;

    use super::*;

    fn key(text: &str) -> Keystroke {
        match Keystroke::parse_with_os(text, OperatingSystem::Linux) {
            Ok(keystroke) => keystroke,
            Err(error) => panic!("{text:?} is not a key: {error}"),
        }
    }

    /// Whether each key-down of `keys`, in turn, is a repeat; `"up"` lets the held key go.
    fn repeats(keys: &[&str]) -> Vec<bool> {
        let mut repeats = KeyRepeats::default();
        let mut seen = Vec::new();
        for text in keys {
            if *text == "up" {
                repeats.release();
                continue;
            }
            repeats.key_down(&key(text));
            seen.push(repeats.repeating());
        }
        seen
    }

    #[test]
    fn a_key_down_again_without_a_key_up_is_a_repeat() {
        assert_eq!(
            repeats(&["space", "space", "space", "up", "space"]),
            [false, true, true, false]
        );
    }

    #[test]
    fn another_key_or_other_modifiers_are_a_new_press() {
        assert_eq!(
            repeats(&["ctrl-s", "s", "ctrl-shift-s", "ctrl-shift-s", "space"]),
            [false, false, false, true, false]
        );
    }

    #[test]
    fn the_held_key_is_the_last_one_down_until_a_key_goes_up() {
        let mut repeats = KeyRepeats::default();
        assert!(repeats.held().is_none());
        repeats.key_down(&key("space"));
        repeats.key_down(&key("ctrl-l"));
        assert!(
            repeats
                .held()
                .is_some_and(|held| same_key(held, &key("ctrl-l")))
        );
        repeats.release();
        assert!(repeats.held().is_none());
    }

    #[test]
    fn the_text_of_a_key_does_not_matter() {
        let mut repeats = KeyRepeats::default();
        repeats.key_down(&key("space"));
        repeats.key_down(&key("space").with_simulated_ime());
        assert!(repeats.repeating());
    }
}
