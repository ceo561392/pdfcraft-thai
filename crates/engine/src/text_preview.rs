//! The page beneath an inline text editor, with the source glyphs removed on a COS clone.

use std::sync::Arc;

use pdfcraft_render::{PageRenderer, RenderConfig, RenderPool, RenderRequest, RenderedPage};

use crate::{Document, guard};

pub struct TextEditPreview {
    page: usize,
    bytes: Arc<Vec<u8>>,
    config: RenderConfig,
}

impl TextEditPreview {
    pub fn renderer(&self) -> RenderPool {
        RenderPool::new(self.bytes.clone(), 1, self.config.clone())
    }

    pub fn render_background(&self, scale: f32) -> Result<RenderedPage, String> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err("preview scale must be finite and positive".into());
        }
        guard(|| {
            let mut renderer = PageRenderer::new(self.bytes.clone(), self.config.clone());
            let out = renderer.render(RenderRequest { page: self.page, scale, ..Default::default() });
            if let Some(e) = &out.error {
                return Err(e.clone());
            }
            Ok(out)
        })?
    }
}

impl Document {
    /// In-memory font matches can change while an editor is open, without changing the PDF.
    pub fn text_font_generation(&self) -> usize {
        self.matched_text_fonts.len()
    }
    /// Background for retyping an added text item, without changing its saved content.
    pub fn added_text_preview(&self, page: usize, index: usize) -> Result<TextEditPreview, String> {
        guard(|| {
            if !self.added().iter().filter(|a| a.page == page).nth(index).is_some_and(|a| matches!(a.content, crate::AddedContent::Text(_))) {
                return Err("the selected added item is not text".into());
            }
            let editor = self.editor.as_ref().ok_or("the document can't be read")?;
            let mut cos = editor.cos.clone();
            pdfcraft_edit::delete_content(&mut cos, page, index).map_err(|e| e.to_string())?;
            let bytes = Arc::new(pdfcraft_cos::write_incremental(&cos, &pdfcraft_cos::SaveOptions::default()).map_err(|e| e.to_string())?);
            Ok(TextEditPreview { page, bytes, config: self.config.clone() })
        })?
    }

    /// Measure a draft without changing bytes, dirty state, or undo history.
    pub fn text_edit_layout(
        &self,
        page: usize,
        block: usize,
        text: &str,
        style: &pdfcraft_edit::BlockStyle,
    ) -> Result<pdfcraft_edit::text::TextEditLayout, String> {
        guard(|| {
            let editor = self.editor.as_ref().ok_or("the document can't be read")?;
            let mut cos = editor.cos.clone();
            crate::text_fonts::embed_matches(&mut cos, page, &self.matched_text_fonts).map_err(|e| e.to_string())?;
            pdfcraft_edit::text::text_edit_layout(&cos, page, block, text, style).map_err(|e| e.to_string())
        })?
    }

    /// Embedded source faces for a text editor, bounded and validated as sfnt programs.
    /// This is read-only; no source font is installed or exported.
    pub fn text_editor_fonts(&self, page: usize) -> Vec<pdfcraft_edit::text::TextEditorFont> {
        let mut programs = self.editor.as_ref().and_then(|e| pdfcraft_edit::text::text_editor_fonts(&e.cos, page).ok()).unwrap_or_default();
        let lines = self.text_lines(page);
        let mut total = programs.iter().map(|p| p.bytes.len()).sum::<usize>();
        let mut seen = std::collections::HashSet::new();
        for line in lines.iter().filter(|line| seen.insert(line.font.as_str())).take(128) {
            if let Some(face) = self
                .matched_text_fonts
                .iter()
                .find(|f| pdfcraft_fonts::embedding::face_key(&f.info.postscript) == pdfcraft_fonts::embedding::face_key(&line.base_font))
            {
                if let Some(program) = programs.iter_mut().find(|p| p.resource == line.font) {
                    let next = total.saturating_sub(program.bytes.len()).saturating_add(face.bytes.len());
                    if next > 64 << 20 {
                        continue;
                    }
                    total = next;
                    program.bytes = face.bytes.as_ref().clone();
                } else {
                    if programs.len() >= 128 || total.saturating_add(face.bytes.len()) > 64 << 20 {
                        continue;
                    }
                    total += face.bytes.len();
                    programs.push(pdfcraft_edit::text::TextEditorFont {
                        resource: line.font.clone(),
                        face: face.info.postscript.clone(),
                        bytes: face.bytes.as_ref().clone(),
                    });
                }
            }
        }
        let families: std::collections::HashSet<_> = lines.iter().map(|l| pdfcraft_fonts::document_face::family_key(&l.base_font)).collect();
        for face in self.matched_text_fonts.iter().filter(|f| families.contains(&pdfcraft_fonts::document_face::family_key(&f.info.postscript))) {
            if programs.len() >= 128 || total.saturating_add(face.bytes.len()) > 64 << 20 {
                break;
            }
            total += face.bytes.len();
            programs.push(pdfcraft_edit::text::TextEditorFont {
                resource: format!("PCMatched{}", face.info.postscript),
                face: face.info.postscript.clone(),
                bytes: face.bytes.as_ref().clone(),
            });
        }
        programs
    }

    /// Preparing the background never changes working bytes, dirty state or undo history.
    pub fn text_edit_preview(&self, page: usize, block: usize) -> Result<TextEditPreview, String> {
        guard(|| {
            let editor = self.editor.as_ref().ok_or("the document can't be read")?;
            let mut cos = editor.cos.clone();
            pdfcraft_edit::clear_block(&mut cos, page, block).map_err(|e| e.to_string())?;
            let bytes = Arc::new(pdfcraft_cos::write_incremental(&cos, &pdfcraft_cos::SaveOptions::default()).map_err(|e| e.to_string())?);
            Ok(TextEditPreview { page, bytes, config: self.config.clone() })
        })?
    }

    /// The page beneath an added item while it is dragged: `index` counts the page's added
    /// items, as `Edit::DeleteContent` does. Read-only, like [`Self::text_edit_preview`].
    pub fn added_content_preview(&self, page: usize, index: usize) -> Result<TextEditPreview, String> {
        guard(|| {
            let editor = self.editor.as_ref().ok_or("the document can't be read")?;
            let mut cos = editor.cos.clone();
            pdfcraft_edit::delete_content(&mut cos, page, index).map_err(|e| e.to_string())?;
            let bytes = Arc::new(pdfcraft_cos::write_incremental(&cos, &pdfcraft_cos::SaveOptions::default()).map_err(|e| e.to_string())?);
            Ok(TextEditPreview { page, bytes, config: self.config.clone() })
        })?
    }
}
