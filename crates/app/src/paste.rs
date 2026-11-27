//! Pasting into a note editor. Both editors (the main window's and the
//! quick note's) hand every paste here (`ClipboardPolicy::Host`).
//!
//! Text goes straight back to the editor, as before. A picture — a
//! screenshot, an image copied in a browser, image files copied in Explorer
//! — is embedded in the note (`stickynotes_core::picture`) off the UI thread
//! and then inserted where the paste was asked for.
//!
//! Text wins when the clipboard holds both: Excel and Word put a picture of
//! the copied cells or text next to the text itself.

use std::path::PathBuf;

use gpui_kit::*;
use markdown_editor::{BlockEditorState, PasteRequest};
use stickynotes_core::picture;

/// Shown while pictures are being prepared.
pub(crate) const PREPARING: &str = "正在插入图片…";

/// How a picture paste ended, for the window's status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Every picture is in the note.
    Inserted,
    /// Some or all pictures were left out; the message says why.
    Failed(String),
    /// The note or the caret moved on while the pictures were prepared, so
    /// they were not inserted anywhere.
    Stale,
}

enum Source {
    Bytes(Vec<u8>),
    File(PathBuf),
}

/// Resolve one intercepted paste. Text is pasted before this returns
/// `false`. `true` means pictures are being prepared; `done` runs once they
/// are in, or could not be.
pub(crate) fn resolve(
    editor: &Entity<BlockEditorState>,
    request: &PasteRequest,
    item: &ClipboardItem,
    done: impl FnOnce(Outcome, &mut App) + 'static,
    cx: &mut App,
) -> bool {
    let sources = if request.is_plain() || has_text(item) {
        Vec::new()
    } else {
        picture_sources(item)
    };
    if sources.is_empty() {
        // Files that are not pictures still paste as their paths (the text
        // gpui makes of them), as they always did.
        let text = item.text();
        editor.update(cx, |editor, cx| {
            editor.apply_paste(request, text.as_deref(), cx);
        });
        return false;
    }

    let editor = editor.downgrade();
    let request = request.clone();
    let prepared = cx
        .background_executor()
        .spawn(async move { prepare(sources) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let prepared = prepared.await;
        let _ = cx.update(move |cx| {
            let outcome = insert(&editor, &request, prepared, cx);
            done(outcome, cx);
        });
    })
    .detach();
    true
}

/// Text that is more than white space. Not `ClipboardItem::text`, which
/// turns copied files into their paths.
fn has_text(item: &ClipboardItem) -> bool {
    item.entries().iter().any(|entry| {
        matches!(entry, ClipboardEntry::String(string) if !string.text().trim().is_empty())
    })
}

fn picture_sources(item: &ClipboardItem) -> Vec<Source> {
    // Image files copied in Explorer come as their paths; a bitmap that a
    // program adds next to them is the same picture again.
    let files: Vec<Source> = item
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            ClipboardEntry::ExternalPaths(paths) => Some(paths.paths()),
            _ => None,
        })
        .flatten()
        .filter(|path| picture::is_picture_file(path))
        .map(|path| Source::File(path.clone()))
        .collect();
    if !files.is_empty() {
        return files;
    }
    item.entries()
        .iter()
        .filter_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(Source::Bytes(image.bytes().to_vec())),
            _ => None,
        })
        .collect()
}

struct Prepared {
    markdown: Vec<String>,
    problems: Vec<String>,
    /// Pictures past [`picture::MAX_PER_PASTE`].
    left_out: usize,
}

/// Runs on a background thread: decoding, scaling and compressing a large
/// screenshot takes a noticeable moment.
fn prepare(sources: Vec<Source>) -> Prepared {
    let left_out = sources.len().saturating_sub(picture::MAX_PER_PASTE);
    let mut markdown = Vec::new();
    let mut problems = Vec::new();
    for source in sources.into_iter().take(picture::MAX_PER_PASTE) {
        let embedded = match &source {
            Source::Bytes(bytes) => picture::embed(bytes),
            Source::File(path) => picture::embed_file(path),
        };
        match embedded {
            Ok(picture) => markdown.push(picture.markdown()),
            Err(problem) => problems.push(problem.to_string()),
        }
    }
    Prepared {
        markdown,
        problems,
        left_out,
    }
}

fn insert(
    editor: &WeakEntity<BlockEditorState>,
    request: &PasteRequest,
    prepared: Prepared,
    cx: &mut App,
) -> Outcome {
    let Some(editor) = editor.upgrade() else {
        return Outcome::Stale;
    };
    editor.update(cx, |editor, cx| {
        if !editor.is_paste_current(request) {
            return Outcome::Stale;
        }
        if prepared.markdown.is_empty() {
            // Settle the request; nothing is inserted.
            editor.apply_paste(request, None, cx);
            let reason = prepared.problems.into_iter().next();
            return Outcome::Failed(reason.unwrap_or_else(|| "没有可以插入的图片".into()));
        }
        // One picture per line: each gets a paragraph of its own.
        if !editor.apply_paste_markdown(request, &prepared.markdown.join("\n"), cx) {
            return Outcome::Failed("代码块里不能插入图片".into());
        }
        let missing = prepared.problems.len() + prepared.left_out;
        if missing == 0 {
            return Outcome::Inserted;
        }
        let reason = prepared
            .problems
            .into_iter()
            .next()
            .unwrap_or_else(|| format!("一次最多插入 {} 张", picture::MAX_PER_PASTE));
        Outcome::Failed(format!("有 {missing} 张图片没有插入：{reason}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    // The plain test attribute: `gpui_kit::*` brings gpui's own `test` along.
    use core::prelude::v1::test;
    use markdown_editor::{BlockEditorEvent, BlockPosition, BlockSelection, ClipboardPolicy};
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Root(Entity<BlockEditorState>);

    impl Render for Root {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(600.))
                .h(px(800.))
                .child(markdown_editor::BlockEditor::new(&self.0))
        }
    }

    type Outcomes = Rc<RefCell<Vec<Outcome>>>;

    /// An editor set up the way the app sets its own up: every paste goes
    /// through [`resolve`], whose outcomes are collected.
    fn host_editor<'a>(
        cx: &'a mut TestAppContext,
        source: &str,
    ) -> (Entity<BlockEditorState>, Outcomes, &'a mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            markdown_editor::init(cx);
        });
        let source = source.to_string();
        let (root, cx) = cx.add_window_view(move |window, cx| {
            Root(cx.new(|cx| {
                BlockEditorState::new(window, cx)
                    .with_source(&source)
                    .with_clipboard_policy(ClipboardPolicy::Host)
            }))
        });
        let editor = root.read_with(cx, |root, _| root.0.clone());
        let outcomes: Outcomes = Rc::default();
        let sink = outcomes.clone();
        cx.update(|_, cx| {
            cx.subscribe(&editor, move |editor, event: &BlockEditorEvent, cx| {
                if let BlockEditorEvent::PasteRequested { request, item } = event {
                    let sink = sink.clone();
                    resolve(
                        &editor,
                        request,
                        item,
                        move |outcome, _| sink.borrow_mut().push(outcome),
                        cx,
                    );
                }
            })
            .detach();
        });
        (editor, outcomes, cx)
    }

    /// Paste `item` at the end of the document and let the pictures be
    /// prepared and inserted.
    fn paste_at_end(
        editor: &Entity<BlockEditorState>,
        item: ClipboardItem,
        plain: bool,
        cx: &mut VisualTestContext,
    ) {
        editor.update(cx, |editor, cx| {
            let _ = editor.select_end(cx);
            editor.paste(item, plain, cx);
        });
        cx.run_until_parked();
    }

    fn source(editor: &Entity<BlockEditorState>, cx: &VisualTestContext) -> String {
        editor.read_with(cx, |editor, _| editor.source())
    }

    /// What a screenshot puts on the Windows clipboard (CF_DIB), as gpui
    /// reads it: a BMP.
    fn bitmap(width: u32, height: u32) -> Image {
        let picture =
            image::RgbImage::from_fn(width, height, |x, _| image::Rgb([(x % 256) as u8, 90, 160]));
        let mut bmp = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(picture)
            .write_to(&mut bmp, image::ImageFormat::Bmp)
            .unwrap();
        Image::from_bytes(ImageFormat::Bmp, bmp.into_inner())
    }

    fn pictures(markdown: &str) -> usize {
        markdown.matches("![图片](data:image/png;base64,").count()
    }

    #[test]
    fn a_screenshot_on_the_clipboard_is_embedded_each_time_it_is_pasted() {
        let mut cx = TestAppContext::single();
        let (editor, outcomes, cx) = host_editor(&mut cx, "截图：");
        let item = ClipboardItem::new_image(&bitmap(320, 200));
        paste_at_end(&editor, item.clone(), false, cx);
        assert_eq!(*outcomes.borrow(), vec![Outcome::Inserted]);
        let after_one = source(&editor, cx);
        assert!(after_one.starts_with("截图：![图片](data:image/png;base64,"));
        assert_eq!(pictures(&after_one), 1);
        // The same clipboard again: a second picture.
        paste_at_end(&editor, item, false, cx);
        assert_eq!(outcomes.borrow().len(), 2);
        assert_eq!(pictures(&source(&editor, cx)), 2);
        // One undo step per paste.
        editor.update(cx, |editor, cx| editor.undo(cx));
        assert_eq!(source(&editor, cx), after_one);
    }

    #[test]
    fn text_wins_when_a_picture_of_it_comes_along() {
        let mut cx = TestAppContext::single();
        let (editor, outcomes, cx) = host_editor(&mut cx, "");
        // Excel: the cells as text, and a picture of them.
        let item = ClipboardItem {
            entries: vec![
                ClipboardEntry::String(ClipboardString::new("A1 B1".into())),
                ClipboardEntry::Image(bitmap(64, 32)),
            ],
        };
        paste_at_end(&editor, item, false, cx);
        assert_eq!(source(&editor, cx), "A1 B1");
        assert!(outcomes.borrow().is_empty(), "no picture work at all");
    }

    #[test]
    fn plain_paste_takes_no_pictures() {
        let mut cx = TestAppContext::single();
        let (editor, outcomes, cx) = host_editor(&mut cx, "x");
        paste_at_end(&editor, ClipboardItem::new_image(&bitmap(64, 32)), true, cx);
        assert_eq!(source(&editor, cx), "x");
        assert!(outcomes.borrow().is_empty());
        // Text still pastes the usual way.
        paste_at_end(&editor, ClipboardItem::new_string("yz".into()), false, cx);
        assert_eq!(source(&editor, cx), "xyz");
    }

    #[test]
    fn image_files_are_embedded_and_other_files_paste_as_their_paths() {
        let dir = std::env::temp_dir().join(format!("stickynotes-paste-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("shot.png");
        image::RgbImage::from_pixel(40, 30, image::Rgb([200, 30, 30]))
            .save(&png)
            .unwrap();
        let doc = dir.join("readme.txt");
        std::fs::write(&doc, "hello").unwrap();
        let files = |paths: Vec<std::path::PathBuf>| ClipboardItem {
            entries: vec![ClipboardEntry::ExternalPaths(ExternalPaths(
                paths.into_iter().collect(),
            ))],
        };

        let mut cx = TestAppContext::single();
        let (editor, outcomes, cx) = host_editor(&mut cx, "");
        // A picture file and another file: the picture goes in.
        paste_at_end(&editor, files(vec![png.clone(), doc.clone()]), false, cx);
        assert_eq!(*outcomes.borrow(), vec![Outcome::Inserted]);
        assert_eq!(pictures(&source(&editor, cx)), 1);
        // No picture among them: the path, as before.
        editor.update(cx, |editor, cx| {
            let _ = editor.set_source("", cx);
        });
        paste_at_end(&editor, files(vec![doc.clone()]), false, cx);
        assert!(source(&editor, cx).contains("readme.txt"), "{}", source(&editor, cx));
        assert_eq!(pictures(&source(&editor, cx)), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_is_not_put_into_code_and_a_broken_one_says_why() {
        let mut cx = TestAppContext::single();
        let (editor, outcomes, cx) = host_editor(&mut cx, "```\ncode\n```");
        editor.update(cx, |editor, cx| {
            let leaf = editor.model().document().leaves().next().unwrap().id();
            editor
                .set_selection(BlockSelection::caret(BlockPosition::new(leaf, 2)), cx)
                .unwrap();
            editor.paste(ClipboardItem::new_image(&bitmap(16, 16)), false, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            *outcomes.borrow(),
            vec![Outcome::Failed("代码块里不能插入图片".into())]
        );
        assert_eq!(source(&editor, cx), "```\ncode\n```");

        outcomes.borrow_mut().clear();
        editor.update(cx, |editor, cx| {
            let _ = editor.set_source("x", cx);
        });
        let broken = Image::from_bytes(ImageFormat::Png, b"\x89PNG\r\n\x1a\nnot really".to_vec());
        paste_at_end(&editor, ClipboardItem::new_image(&broken), false, cx);
        let outcome = outcomes.borrow().first().cloned();
        assert!(
            matches!(&outcome, Some(Outcome::Failed(reason)) if reason.starts_with("图片无法读取")),
            "{outcome:?}"
        );
        assert_eq!(source(&editor, cx), "x");
    }
}
