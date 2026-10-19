//! The lesson pane (placeholder): a header, then headings, paragraphs with bold words, a figure
//! and bullet lists in a scroll area, and an action button. Content comes from lesson Markdown
//! later; here the caller passes blocks.

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Frame, Layout, Response, ScrollArea, TextureId, Ui, Vec2};

use crate::theme::{Theme, glyph};

/// A run of text inside a paragraph.
#[derive(Clone, Copy)]
pub struct Span<'a> {
    pub text: &'a str,
    pub strong: bool,
}

#[derive(Clone, Copy)]
pub enum Block<'a> {
    Heading(&'a str),
    Paragraph(&'a [Span<'a>]),
    Figure {
        texture: TextureId,
        size: Vec2,
        caption: &'a str,
    },
    Bullets(&'a [&'a str]),
}

pub struct LessonPane<'a> {
    theme: &'a Theme,
    id_salt: &'a str,
    header: &'a str,
    blocks: &'a [Block<'a>],
    action: Option<&'a str>,
}

impl<'a> LessonPane<'a> {
    pub fn new(
        theme: &'a Theme,
        id_salt: &'a str,
        header: &'a str,
        blocks: &'a [Block<'a>],
    ) -> Self {
        Self {
            theme,
            id_salt,
            header,
            blocks,
            action: None,
        }
    }

    /// A button under the lesson ("Start challenge").
    #[must_use]
    pub fn action(mut self, label: &'a str) -> Self {
        self.action = Some(label);
        self
    }

    /// Fills the available height. Returns the action button's response, if there is one.
    pub fn show(self, ui: &mut Ui) -> Option<Response> {
        let t = self.theme;
        let c = &t.color;
        Frame::new()
            .fill(c.surface.card)
            .corner_radius(t.radius.medium)
            .inner_margin(t.space.pad)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(ui.available_height());
                ui.with_layout(Layout::top_down(Align::Min), |ui| self.contents(ui))
                    .inner
            })
            .inner
    }

    fn contents(&self, ui: &mut Ui) -> Option<Response> {
        let t = self.theme;
        let c = &t.color;
        ui.label(
            egui::RichText::new(self.header.to_uppercase())
                .font(t.text.label.clone())
                .color(c.text.label),
        );
        ui.add_space(t.space.gap);
        let mut action = None;
        ScrollArea::vertical()
            .id_salt(self.id_salt)
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = t.space.gap;
                for block in self.blocks {
                    self.block(ui, block);
                }
                if let Some(label) = self.action {
                    ui.add_space(t.space.gap);
                    action = Some(ui.button(label));
                }
            });
        action
    }

    fn block(&self, ui: &mut Ui, block: &Block<'_>) {
        let t = self.theme;
        let c = &t.color;
        match *block {
            Block::Heading(text) => {
                ui.add_space(t.space.tight);
                ui.label(
                    egui::RichText::new(text)
                        .font(t.text.title.clone())
                        .color(c.text.primary),
                );
            }
            Block::Paragraph(spans) => {
                let mut job = LayoutJob::default();
                for span in spans {
                    let font = if span.strong {
                        &t.text.strong
                    } else {
                        &t.text.body
                    };
                    job.append(
                        span.text,
                        0.0,
                        TextFormat::simple(font.clone(), c.text.primary),
                    );
                }
                job.wrap.max_width = ui.available_width();
                ui.label(job);
            }
            Block::Figure {
                texture,
                size,
                caption,
            } => {
                let width = size.x.min(ui.available_width());
                let scaled = size * (width / size.x);
                ui.add(
                    egui::Image::from_texture(egui::load::SizedTexture::new(texture, scaled))
                        .corner_radius(t.radius.small),
                );
                ui.label(
                    egui::RichText::new(caption)
                        .font(t.text.label.clone())
                        .color(c.text.label),
                );
            }
            Block::Bullets(items) => {
                for item in items {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = t.space.gap;
                        ui.label(
                            egui::RichText::new(glyph::BULLET)
                                .font(t.text.body.clone())
                                .color(c.accent.value),
                        );
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(*item)
                                    .font(t.text.body.clone())
                                    .color(c.text.primary),
                            )
                            .wrap(),
                        );
                    });
                }
            }
        }
    }
}
