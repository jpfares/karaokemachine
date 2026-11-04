//! The cover-art picker inside the upload dialog: a preview tile, a popup
//! for choosing a link, a file or a clipboard image, and the preview
//! download that runs in the background.

use super::lobby::load_texture;
use super::upload::paint_placeholder;
use crate::jobs::Job;
use crate::theme::{alpha, GOLD, MINT, MUTED, MUTED2, PINK};
use crate::tools::node_command;
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle,
    Vec2,
};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(super) struct CoverPicker {
    /// What the importer receives: a local image path or an http(s) link.
    /// Empty means no cover.
    pub(super) value: String,
    pub(super) popup_open: bool,
    link: String,
    preview_path: Option<PathBuf>,
    preview_tex: Option<TextureHandle>,
    preview_loaded: Option<PathBuf>,
    fetch: Option<Job<PathBuf>>,
    message: String,
    error: bool,
}

impl CoverPicker {
    pub(super) fn fetching(&self) -> bool {
        self.fetch.is_some()
    }

    pub(super) fn has_value(&self) -> bool {
        !self.value.trim().is_empty()
    }

    fn fail(&mut self, message: impl Into<String>) {
        self.error = true;
        self.message = message.into();
    }

    /// Forget everything, including any message.
    pub(super) fn reset(&mut self) {
        let popup_open = self.popup_open;
        *self = Self::default();
        self.popup_open = popup_open;
    }

    /// Take a link or path straight from fetched metadata. The preview is
    /// cleared; call [`Self::start_fetch`] to download one.
    pub(super) fn set_value(&mut self, value: String) {
        self.value = value;
        self.preview_path = None;
        self.preview_tex = None;
        self.preview_loaded = None;
        self.message.clear();
        self.error = false;
    }

    /// Use a local image file as the cover and preview it.
    pub(super) fn set_from_path(&mut self, path: PathBuf, message: impl Into<String>) {
        if !path.exists() {
            self.fail(format!("Cover image not found: {}", path.display()));
            return;
        }
        self.value = path.to_string_lossy().to_string();
        self.preview_path = Some(path);
        self.preview_loaded = None;
        self.preview_tex = None;
        self.fetch = None;
        self.error = false;
        self.message = message.into();
    }

    /// Download `url` for preview on a worker, or use it directly if it is a
    /// local path. The downloaded file becomes the cover on success.
    pub(super) fn start_fetch(&mut self, root: &Path, ctx: &egui::Context, url: String) {
        let url = url.trim().to_string();
        if url.is_empty() {
            return;
        }
        self.link = url.clone();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.set_from_path(PathBuf::from(&url), "Cover image selected.");
            return;
        }
        if self.fetching() {
            return;
        }
        let script = root.join("dist-tools/fetch-cover.js");
        if !script.exists() {
            self.fail("Cover preview tools are not built. Run `npm install && npm run assets`.");
            return;
        }
        let work = root.join(".work");
        if let Err(error) = std::fs::create_dir_all(&work) {
            self.fail(format!("Could not create the work directory: {error}"));
            return;
        }
        let out = work.join("upload-cover-preview.bin");
        let root = root.to_path_buf();
        self.error = false;
        self.message = "Downloading cover preview…".into();
        self.fetch = Some(Job::run("The cover preview helper", ctx, move || {
            run_cover_download(&root, &url, &out)
        }));
    }

    /// Drop an in-flight preview download. The worker itself is not cancelled.
    pub(super) fn cancel_fetch(&mut self) {
        self.fetch = None;
    }

    /// Remove the cover and say so.
    pub(super) fn clear(&mut self) {
        self.reset();
        self.message = "Cover removed.".into();
    }

    pub(super) fn pick_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Image files", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
        {
            self.set_from_path(path, "Cover image selected. Click DONE to continue.");
        }
    }

    pub(super) fn paste_image(&mut self, root: &Path) {
        let image = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_image());
        let image = match image {
            Ok(image) => image,
            Err(error) => {
                self.fail(format!("Clipboard image unavailable: {error}"));
                return;
            }
        };
        let work = root.join(".work");
        if let Err(error) = std::fs::create_dir_all(&work) {
            self.fail(format!("Could not create the work directory: {error}"));
            return;
        }
        let path = work.join("upload-cover-paste.png");
        let Some(buffer) = image::RgbaImage::from_raw(
            image.width as u32,
            image.height as u32,
            image.bytes.into_owned(),
        ) else {
            self.fail("The clipboard image was not readable.");
            return;
        };
        if let Err(error) = buffer.save(&path) {
            self.fail(format!("Could not save the clipboard image: {error}"));
            return;
        }
        self.set_from_path(
            path,
            "Cover pasted from the clipboard. Click DONE to continue.",
        );
    }

    /// Collect a finished preview download.
    pub(super) fn poll(&mut self) {
        let Some(result) = self.fetch.as_mut().and_then(Job::poll) else {
            return;
        };
        self.fetch = None;
        match result {
            Ok(path) => {
                self.value = path.to_string_lossy().to_string();
                self.preview_path = Some(path);
                self.preview_loaded = None;
                self.message = "Cover art ready.".into();
                self.error = false;
            }
            Err(error) => self.fail(error),
        }
    }

    /// Load the preview texture when the preview file changes.
    pub(super) fn update_preview(&mut self, ctx: &egui::Context) {
        if self.preview_path != self.preview_loaded {
            self.preview_loaded = self.preview_path.clone();
            self.preview_tex = self
                .preview_path
                .as_ref()
                .and_then(|path| load_texture(ctx, path));
        }
    }

    /// The cover row inside the upload form. Clicking the tile opens the popup.
    pub(super) fn draw_tile(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("COVER ART")
                    .monospace()
                    .size(10.0)
                    .color(MUTED2),
            );
            if self.fetching() {
                ui.spinner();
                ui.label(RichText::new("Loading preview…").size(10.0).color(GOLD));
            } else {
                ui.label(
                    RichText::new("Click the image area to choose, link, or paste artwork.")
                        .size(10.0)
                        .color(MUTED2),
                );
            }
        });
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 150.0), Sense::click());
        paint_cover_preview(
            ui,
            rect,
            self.preview_tex.as_ref(),
            self.has_value(),
            self.fetching(),
        );
        if response.clicked() {
            self.popup_open = true;
        }
        self.draw_message(ui);
    }

    fn draw_message(&self, ui: &mut egui::Ui) {
        if !self.message.is_empty() {
            ui.label(
                RichText::new(&self.message)
                    .size(10.0)
                    .color(if self.error { PINK } else { MUTED }),
            );
        }
    }

    /// The "Cover art" window: link, file, clipboard, remove.
    pub(super) fn draw_popup(&mut self, ctx: &egui::Context, root: &Path) {
        if !self.popup_open {
            return;
        }
        let mut open = self.popup_open;
        let mut load_link = false;
        let mut pick_file = false;
        let mut paste_image = false;
        let mut remove_cover = false;
        let mut done = false;
        egui::Window::new("Cover art")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_size(Vec2::new(560.0, 780.0))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(520.0);
                egui::ScrollArea::vertical()
                    .max_height(720.0)
                    .show(ui, |ui| {
                        let (preview, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 270.0),
                            Sense::hover(),
                        );
                        paint_cover_preview(
                            ui,
                            preview,
                            self.preview_tex.as_ref(),
                            self.has_value(),
                            self.fetching(),
                        );
                        if !self.message.is_empty() {
                            ui.add_space(4.0);
                            self.draw_message(ui);
                        }
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("PASTE AN IMAGE LINK")
                                .monospace()
                                .size(10.0)
                                .color(MUTED2),
                        );
                        ui.horizontal(|ui| {
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut self.link).desired_width(380.0),
                            );
                            if self.link.is_empty() {
                                paint_placeholder(ui, response.rect, "https://…/cover.jpg", false);
                            }
                            if ui
                                .add_enabled(
                                    !self.fetching() && !self.link.trim().is_empty(),
                                    egui::Button::new("LOAD LINK"),
                                )
                                .clicked()
                            {
                                load_link = true;
                            }
                        });
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("UPLOAD A FILE")
                                .monospace()
                                .size(10.0)
                                .color(MUTED2),
                        );
                        if ui.button("SELECT AN IMAGE FILE").clicked() {
                            pick_file = true;
                        }
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("PASTE FROM CLIPBOARD")
                                .monospace()
                                .size(10.0)
                                .color(MUTED2),
                        );
                        if ui.button("PASTE IMAGE").clicked() {
                            paste_image = true;
                        }
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(
                                    self.preview_tex.is_some(),
                                    egui::Button::new("USE THIS COVER"),
                                )
                                .clicked()
                            {
                                done = true;
                            }
                            if ui.button("REMOVE COVER").clicked() {
                                remove_cover = true;
                            }
                        });
                    });
            });
        if !open || done {
            self.popup_open = false;
        }
        if load_link {
            self.start_fetch(root, ctx, self.link.clone());
        }
        if pick_file {
            self.pick_file();
        }
        if paste_image {
            self.paste_image(root);
        }
        if remove_cover {
            self.clear();
        }
    }
}

/// Download `url` with the Node helper, decode it, and save a PNG preview.
fn run_cover_download(root: &Path, url: &str, out: &Path) -> Result<PathBuf, String> {
    let script = root.join("dist-tools/fetch-cover.js");
    if !script.exists() {
        return Err(
            "Cover preview tools are not built. Run `npm install && npm run assets`.".into(),
        );
    }
    let output = node_command(root, &script)
        .arg("--url")
        .arg(url)
        .arg("--out")
        .arg(out)
        .output()
        .map_err(|error| format!("Could not start the cover downloader: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Cover download failed: {}",
            stderr.lines().next_back().unwrap_or("unknown error")
        ));
    }
    let bytes = std::fs::read(out)
        .map_err(|error| format!("Could not read the downloaded cover: {error}"))?;
    let image = image::load_from_memory(&bytes)
        .map_err(|error| format!("The downloaded cover was not an image: {error}"))?;
    let png = out.with_extension("png");
    image
        .save(&png)
        .map_err(|error| format!("Could not save the cover preview: {error}"))?;
    let _ = std::fs::remove_file(out);
    Ok(png)
}

/// The cover tile: the image when there is one, otherwise an upload prompt.
fn paint_cover_preview(
    ui: &egui::Ui,
    rect: Rect,
    texture: Option<&TextureHandle>,
    has_cover: bool,
    loading: bool,
) {
    let p = ui.painter();
    p.rect_filled(rect, 12.0, Color32::from_rgb(8, 12, 21));
    if let Some(texture) = texture {
        let size = texture.size_vec2();
        let scale = (rect.width() / size.x.max(1.0)).min(rect.height() / size.y.max(1.0));
        let target = Rect::from_center_size(rect.center(), size * scale);
        p.image(
            texture.id(),
            target,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        let label = Rect::from_min_size(
            Pos2::new(rect.right() - 126.0, rect.bottom() - 36.0),
            Vec2::new(114.0, 24.0),
        );
        p.rect_filled(label, 7.0, Color32::from_rgba_unmultiplied(5, 8, 15, 210));
        p.text(
            label.center(),
            Align2::CENTER_CENTER,
            "CHANGE COVER",
            FontId::monospace(10.0),
            MINT,
        );
    } else {
        let center = rect.center() + Vec2::new(0.0, -20.0);
        let tray = Rect::from_center_size(center + Vec2::new(0.0, 12.0), Vec2::new(42.0, 10.0));
        p.rect_stroke(
            tray,
            3.0,
            Stroke::new(1.6f32, alpha(MUTED, 180)),
            StrokeKind::Inside,
        );
        p.line_segment(
            [
                Pos2::new(center.x, center.y - 16.0),
                Pos2::new(center.x, center.y + 10.0),
            ],
            Stroke::new(1.7f32, MUTED),
        );
        p.line_segment(
            [
                Pos2::new(center.x - 9.0, center.y - 7.0),
                Pos2::new(center.x, center.y - 16.0),
            ],
            Stroke::new(1.7f32, MUTED),
        );
        p.line_segment(
            [
                Pos2::new(center.x + 9.0, center.y - 7.0),
                Pos2::new(center.x, center.y - 16.0),
            ],
            Stroke::new(1.7f32, MUTED),
        );
        let title = if loading {
            "Loading cover art…"
        } else if has_cover {
            "Cover linked — click to preview or replace"
        } else {
            "Click to upload cover art"
        };
        p.text(
            center + Vec2::new(0.0, 48.0),
            Align2::CENTER_CENTER,
            title,
            FontId::proportional(14.0),
            if loading { GOLD } else { MUTED },
        );
        p.text(
            center + Vec2::new(0.0, 68.0),
            Align2::CENTER_CENTER,
            "PNG, JPG, image link, or clipboard paste",
            FontId::monospace(10.0),
            MUTED2,
        );
    }
    p.rect_stroke(
        rect,
        12.0,
        Stroke::new(
            1.0f32,
            if has_cover {
                alpha(MINT, 80)
            } else {
                Color32::from_white_alpha(28)
            },
        ),
        StrokeKind::Inside,
    );
}
