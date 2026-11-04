//! The country flags of the Peers tab: the pictures of country-flag-icons 1.6.20 (MIT; its notice
//! is `assets/flags/LICENSE`), the `3x2` set, embedded as they were published, and [`flag`], the
//! small element that draws one.
//!
//! A flag is drawn with `img()` from an [`Image`] of the SVG. GPUI rasterizes such a picture once,
//! at twice the size of its root tag (`SMOOTH_SVG_SCALE_FACTOR`), and the files of the set give no
//! size, so a flag would take its `viewBox` (513 by 342): a raster of 1026 by 684 pixels, 2.8 MB,
//! for a flag that is shown 15 by 10. The table therefore writes the size of the slot into each
//! root tag once, when it is made, and a test renders every file at that size. The table holds the
//! countries the lookup can give ([`known_countries`]). A region (EU) or a code outside them has no
//! flag, and [`flag`] draws a globe in its place, as it does when the lookup found no country.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use gpui_kit::component::{ActiveTheme as _, Icon, IconName};
use gpui_kit::{
    App, Image, ImageFormat, ImageSource, IntoElement, ParentElement as _, Styled as _, div, img,
    px,
};
use rust_embed::RustEmbed;
use x4_core::{CountryCode, known_countries};

/// The width and the height, in logical pixels, of the slot a flag is drawn in. They map to whole
/// device pixels at the scales 1, 1.6 and 2 (15 by 10, 24 by 16, 30 by 20).
pub(crate) const FLAG_SIZE: (f32, f32) = (15.0, 10.0);

/// The pictures of the flags, `XX.svg` for the code `XX`, embedded into release builds and read
/// from disk in debug builds. The notice of the set is not among them: the package installs it.
#[derive(RustEmbed)]
#[folder = "assets/flags/"]
#[include = "*.svg"]
struct Flags;

/// `svg` with the size of the slot written into its root tag, or `None` when it does not start
/// with a root tag followed by a space.
fn sized(svg: &[u8]) -> Option<Vec<u8>> {
    let attributes = svg.strip_prefix(b"<svg ")?;
    let size = format!(" width=\"{}\" height=\"{}\" ", FLAG_SIZE.0, FLAG_SIZE.1);
    let mut sized = Vec::with_capacity(svg.len() + size.len());
    sized.extend_from_slice(b"<svg");
    sized.extend_from_slice(size.as_bytes());
    sized.extend_from_slice(attributes);
    Some(sized)
}

/// The picture of the flag of `code` as `img()` takes it, made from the embedded file.
fn load(code: CountryCode) -> Option<Arc<Image>> {
    let name = format!("{}.svg", code.as_str());
    let Some(file) = Flags::get(&name) else {
        tracing::warn!(
            country = code.as_str(),
            "no flag file for a country the lookup can give"
        );
        return None;
    };
    let Some(svg) = sized(&file.data) else {
        tracing::warn!(
            country = code.as_str(),
            "a flag file without a root tag to size"
        );
        return None;
    };
    Some(Arc::new(Image::from_bytes(ImageFormat::Svg, svg)))
}

/// The flags by country, made when the first one is asked for. The pictures are small, about
/// 180 KB together. What GPUI rasterizes of the ones shown stays until the app ends: nothing
/// evicts it in this stage, and a few dozen countries at most are shown at once.
static FLAGS: LazyLock<HashMap<CountryCode, Arc<Image>>> = LazyLock::new(|| {
    known_countries()
        .filter_map(|code| Some((code, load(code)?)))
        .collect()
});

/// The flag of `code`, ready for `img()`, or `None` for a code without one: a region such as EU,
/// or a code the lookup does not give.
pub(crate) fn flag_source(code: CountryCode) -> Option<ImageSource> {
    FLAGS.get(&code).cloned().map(ImageSource::from)
}

/// What the slot of a flag holds.
pub(crate) enum Slot {
    /// The flag of the country.
    Flag(ImageSource),
    /// The globe: there is no country, or no flag for it.
    Globe,
}

/// What the slot of `code` holds: its flag, or the globe when there is no country or no flag
/// for it (a region such as EU, or a code the lookup does not give).
pub(crate) fn slot(code: Option<CountryCode>) -> Slot {
    code.and_then(flag_source).map_or(Slot::Globe, Slot::Flag)
}

/// The slot of a flag: the flag of `code`, with a hairline in the theme's border color so that a
/// mostly white flag keeps its edge, or a muted globe when there is no country or no flag for it
/// ([`slot`]).
pub(crate) fn flag(code: Option<CountryCode>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let room = div()
        .relative()
        .flex_none()
        .w(px(FLAG_SIZE.0))
        .h(px(FLAG_SIZE.1));
    match slot(code) {
        Slot::Flag(source) => room
            .child(img(source).w(px(FLAG_SIZE.0)).h(px(FLAG_SIZE.1)))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .border_1()
                    .border_color(theme.border),
            ),
        // As tall as the slot, and centered in it: the globe is round, the slot is not.
        Slot::Globe => room.flex().items_center().justify_center().child(
            Icon::new(IconName::Globe)
                .size(px(FLAG_SIZE.1))
                .text_color(theme.muted_foreground),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Context, InteractiveElement as _, Render, SvgRenderer, TestAppContext,
        TestSupportExt as _, Window, size,
    };

    use super::*;
    use crate::app;

    /// How many files the set of country-flag-icons 1.6.20 has in its `3x2` directory.
    const FILES: usize = 265;

    /// The name and the bytes of every embedded file.
    fn files() -> Vec<(String, Vec<u8>)> {
        Flags::iter()
            .filter_map(|name| {
                let file = Flags::get(&name)?;
                Some((name.into_owned(), file.data.into_owned()))
            })
            .collect()
    }

    #[test]
    fn the_whole_set_is_embedded_and_nothing_else() {
        let names: Vec<String> = Flags::iter().map(Cow::into_owned).collect();
        assert_eq!(names.len(), FILES, "{names:?}");
        let odd: Vec<_> = names
            .iter()
            .filter(|name| {
                name.strip_suffix(".svg")
                    .is_none_or(|stem| stem.contains('/'))
            })
            .collect();
        assert!(odd.is_empty(), "files that are not flags: {odd:?}");
        assert!(
            Flags::get("LICENSE").is_none(),
            "the notice is not embedded"
        );
        for name in ["DE.svg", "EU.svg", "XK.svg", "GB-ENG.svg"] {
            assert!(Flags::get(name).is_some(), "{name}");
        }
    }

    #[test]
    fn every_country_of_the_geoip_table_has_a_flag_file() {
        let missing: Vec<_> = known_countries()
            .filter(|code| Flags::get(&format!("{}.svg", code.as_str())).is_none())
            .collect();
        assert!(missing.is_empty(), "countries without a flag: {missing:?}");
    }

    #[test]
    fn every_country_of_the_geoip_table_gets_a_flag() {
        let without: Vec<_> = known_countries()
            .filter(|code| flag_source(*code).is_none())
            .collect();
        assert!(without.is_empty(), "countries without a flag: {without:?}");
    }

    #[test]
    fn a_country_gets_its_file_with_the_size_written_into_the_root_tag() {
        let germany = CountryCode::new("DE").expect("a code");
        let Some(ImageSource::Image(image)) = flag_source(germany) else {
            panic!("DE has no flag as an image");
        };
        assert_eq!(image.format, ImageFormat::Svg);
        let file = Flags::get("DE.svg").expect("the file").data;
        let (root, rest) = file.split_at("<svg ".len());
        assert_eq!(root, b"<svg ");
        let mut expected = b"<svg width=\"15\" height=\"10\" ".to_vec();
        expected.extend_from_slice(rest);
        assert_eq!(image.bytes, expected, "only the size is added");
    }

    #[test]
    fn a_region_and_a_code_the_set_lacks_have_no_flag() {
        // EU is a file of the set, but the lookup gives no region as a country.
        assert!(Flags::get("EU.svg").is_some());
        for code in ["EU", "AP", "ZZ"] {
            let code = CountryCode::new(code).expect("a code");
            assert!(flag_source(code).is_none(), "{code:?}");
        }
        // The codes of the table that are not countries are not even codes.
        for code in ["--", "A1", "A2", "O1", "de", "DEU"] {
            assert_eq!(CountryCode::new(code), None, "{code}");
        }
    }

    #[test]
    fn a_country_with_a_flag_gets_the_flag_and_anything_else_the_globe() {
        let code = |letters: &str| CountryCode::new(letters).expect("a code");
        assert!(matches!(slot(Some(code("DE"))), Slot::Flag(_)), "DE");
        assert!(matches!(slot(None), Slot::Globe), "no country");
        // A region of the set, and a code outside the table.
        for letters in ["EU", "ZZ"] {
            assert!(
                matches!(slot(Some(code(letters))), Slot::Globe),
                "{letters}"
            );
        }
    }

    /// The size, in device pixels, `img()` gives the picture `bytes`: GPUI renders at scale 1.
    fn rendered_size(bytes: &[u8]) -> (i32, i32) {
        let image = SvgRenderer::new(Arc::new(()))
            .render_single_frame(bytes, 1.0)
            .expect("the picture renders");
        let size = image.size(0);
        (i32::from(size.width), i32::from(size.height))
    }

    #[test]
    fn every_flag_renders_at_twice_the_size_of_its_slot() {
        let files = files();
        assert_eq!(files.len(), FILES);
        let wrong: Vec<_> = files
            .iter()
            .filter_map(|(name, bytes)| {
                // A file that cannot be sized is wrong too, not skipped.
                let size = sized(bytes).map(|svg| rendered_size(&svg));
                (size != Some((30, 20))).then(|| format!("{name}: {size:?}"))
            })
            .collect();
        assert!(wrong.is_empty(), "{wrong:?}");
    }

    /// The elements a flag must not have: text (its font differs from machine to machine), styles,
    /// references and scripts.
    const FORBIDDEN_ELEMENTS: [&str; 6] =
        ["text", "style", "use", "image", "script", "foreignObject"];

    /// What a flag must not hold anywhere: a link (`href`, of any kind), a `url()` reference, and
    /// a declaration or a CDATA section, where a DOCTYPE could define entities.
    const FORBIDDEN_TEXT: [&str; 3] = ["href", "url(", "<!"];

    /// What makes `svg` more than a plain picture of shapes, one problem per entry.
    fn plain_problems(svg: &str) -> Vec<String> {
        let mut problems = Vec::new();
        for tag in svg.split('<').skip(1) {
            let name: String = tag
                .chars()
                .take_while(|c| !c.is_whitespace() && !matches!(c, '/' | '>'))
                .collect();
            // In any namespace: an XHTML <script> runs as well as an SVG one.
            let local = name.rsplit(':').next().unwrap_or_default();
            if FORBIDDEN_ELEMENTS.contains(&local) {
                problems.push(format!("a <{local}> element"));
            }
        }
        for text in FORBIDDEN_TEXT {
            if svg.contains(text) {
                problems.push(format!("a {text}"));
            }
        }
        problems
    }

    #[test]
    fn every_flag_is_a_plain_picture() {
        let files = files();
        assert_eq!(files.len(), FILES);
        let unfit: Vec<_> = files
            .iter()
            .filter_map(|(name, bytes)| {
                let problems = match std::str::from_utf8(bytes) {
                    Ok(svg) => plain_problems(svg),
                    Err(error) => vec![format!("not UTF-8: {error}")],
                };
                (!problems.is_empty()).then(|| format!("{name}: {problems:?}"))
            })
            .collect();
        assert!(unfit.is_empty(), "{unfit:?}");
    }

    #[test]
    fn the_picture_check_catches_what_does_not_belong_in_a_flag() {
        let svg = |inside: &str| {
            format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 3 2\">{inside}</svg>")
        };
        assert_eq!(
            plain_problems(&svg("<g><path d=\"M0 0h3v2H0z\"/><circle r=\"1\"/></g>")),
            Vec::<String>::new()
        );
        for (inside, problem) in [
            ("<text>DE</text>", "a <text> element"),
            ("<style>path { fill: red }</style>", "a <style> element"),
            ("<use href=\"#a\"/>", "a <use> element"),
            ("<image/>", "a <image> element"),
            ("<script>alert(1)</script>", "a <script> element"),
            (
                "<script xmlns=\"http://www.w3.org/1999/xhtml\"/>",
                "a <script> element",
            ),
            ("<foreignObject/>", "a <foreignObject> element"),
            ("<path fill=\"url(#g)\"/>", "a url("),
            ("<a href=\"https://example.invalid\"/>", "a href"),
            ("<!DOCTYPE svg>", "a <!"),
        ] {
            assert!(
                plain_problems(&svg(inside)).contains(&problem.to_owned()),
                "{inside}"
            );
        }
    }

    /// The width over the height of the `viewBox` of `svg`.
    fn view_box_ratio(svg: &str) -> Option<f32> {
        let value = svg.split("viewBox=\"").nth(1)?.split('"').next()?;
        let numbers: Vec<f32> = value
            .split_ascii_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        let [_, _, width, height] = *numbers.as_slice() else {
            return None;
        };
        Some(width / height)
    }

    #[test]
    fn every_flag_is_three_by_two_like_its_slot() {
        let files = files();
        assert_eq!(files.len(), FILES);
        let odd: Vec<_> = files
            .iter()
            .filter_map(|(name, bytes)| {
                let ratio = std::str::from_utf8(bytes).ok().and_then(view_box_ratio);
                // 512 by 341.3 is the set's own rounding of 3 by 2.
                let fits = ratio.is_some_and(|ratio| (ratio - 1.5).abs() < 0.005);
                (!fits).then(|| format!("{name}: {ratio:?}"))
            })
            .collect();
        assert!(odd.is_empty(), "{odd:?}");
    }

    #[test]
    fn every_file_starts_with_a_root_tag_without_a_size() {
        let files = files();
        assert_eq!(files.len(), FILES);
        let odd: Vec<_> = files
            .iter()
            .filter(|(_, bytes)| {
                let root = bytes.split(|byte| *byte == b'>').next().unwrap_or_default();
                !bytes.starts_with(b"<svg ")
                    || root.windows(7).any(|w| w == b" width=" || w == b"height=")
            })
            .map(|(name, _)| name)
            .collect();
        assert!(odd.is_empty(), "{odd:?}");
    }

    /// A window of flag slots, one per code, each in a box that takes the size of its content.
    struct Slots(Vec<Option<CountryCode>>);

    impl Render for Slots {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            div()
                .flex()
                .items_start()
                .gap_1()
                .children(self.0.iter().enumerate().map(|(index, code)| {
                    div()
                        .id(("slot", index))
                        .test_support()
                        .child(flag(*code, cx))
                }))
        }
    }

    #[gpui_kit::test]
    fn a_flag_and_the_globe_take_the_same_slot(cx: &mut TestAppContext) {
        cx.update(app::init);
        let codes = ["DE", "RU", "EU", "ZZ"].map(CountryCode::new);
        let slots = [codes[0], codes[1], None, codes[2], codes[3]];
        let window = cx.add_window(|_, _| Slots(slots.to_vec()));
        // The window's own view is drawn, so it is not updated meanwhile.
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            // The first two have a flag; the others have the globe (no country, a region and a
            // code without a flag).
            for index in 0..slots.len() {
                assert_eq!(
                    window.find(("slot", index)).bounds().size,
                    size(px(FLAG_SIZE.0), px(FLAG_SIZE.1)),
                    "slot {index}: {:?}",
                    slots.get(index)
                );
            }
        })
        .expect("the window is open");
    }
}
