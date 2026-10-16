//! Explicit document-font discovery. Interface fallbacks never enter this catalogue.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct FontFile {
    pub path: PathBuf,
    pub family: String,
    pub face: String,
    pub index: u32,
    pub bold: bool,
    pub italic: bool,
    pub open_license: bool,
}

/// Standard user-installed font folders; discovery is performed only on request.
pub fn installed_directories() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        let mut dirs = vec![PathBuf::from("/Library/Fonts")];
        if let Some(home) = home {
            dirs.push(home.join("Library/Fonts"));
        }
        dirs
    } else if cfg!(windows) {
        let mut dirs = std::env::var_os("WINDIR").map(|p| PathBuf::from(p).join("Fonts")).into_iter().collect::<Vec<_>>();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
        }
        dirs
    } else {
        let mut dirs = vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
        if let Some(home) = home {
            dirs.extend([home.join(".local/share/fonts"), home.join(".fonts")]);
        }
        dirs
    }
}

/// Bounded, symlink-free scan of editable document faces. Non-open faces may only
/// complete an existing document family; callers must not offer them as bundled fonts.
/// The caller controls roots (automation resolves confinement before calling this).
pub fn discover(roots: &[PathBuf]) -> Vec<FontFile> {
    use std::io::Read;
    let mut pending: Vec<_> = roots.iter().take(16).map(|r| (r.clone(), 0usize)).collect();
    let mut files = Vec::new();
    let mut entries = 0usize;
    let mut budget = 256u64 << 20;
    let mut seen = std::collections::HashSet::new();
    while let Some((path, depth)) = pending.pop() {
        if entries >= 4096 || budget == 0 {
            break;
        }
        entries += 1;
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.file_type().is_symlink() || !seen.insert(path.clone()) {
            continue;
        }
        if meta.is_dir() && depth < 4 {
            if let Ok(children) = std::fs::read_dir(&path) {
                let remaining = 4096usize.saturating_sub(entries.saturating_add(pending.len()));
                pending.extend(children.take(remaining).filter_map(Result::ok).map(|p| (p.path(), depth + 1)));
            }
        } else if meta.is_file() && meta.len() <= (16 << 20) && font_extension(&path) {
            if meta.len() > budget {
                continue;
            }
            budget = budget.saturating_sub(meta.len());
            let mut bytes = Vec::new();
            let read = std::fs::File::open(&path).and_then(|f| f.take((16 << 20) + 1).read_to_end(&mut bytes));
            if read.is_err() || bytes.len() > 16 << 20 {
                continue;
            }
            for face in pdfcraft_fonts::document_face::faces(&bytes) {
                files.push(FontFile {
                    path: path.clone(),
                    family: face.family,
                    face: face.postscript,
                    index: face.index,
                    bold: face.bold,
                    italic: face.italic,
                    open_license: face.open_license,
                });
            }
        }
    }
    files.sort_by(|a, b| a.face.cmp(&b.face).then(a.path.cmp(&b.path)));
    files
}
fn font_extension(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["ttf", "otf", "ttc", "otc"].iter().any(|v| e.eq_ignore_ascii_case(v)))
}

pub fn read_face(path: &Path, index: u32) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 16 << 20 {
        return Err("select a font file no larger than 16 MiB".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take((16 << 20) + 1).read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
    pdfcraft_fonts::document_face::select(&bytes, index)
}

pub fn select_face(bytes: &[u8], index: u32) -> Result<Vec<u8>, String> {
    pdfcraft_fonts::document_face::select(bytes, index)
}

#[derive(Clone)]
pub(crate) struct MatchedFace {
    pub info: pdfcraft_fonts::document_face::FaceInfo,
    pub bytes: std::sync::Arc<Vec<u8>>,
}

/// Reuse complete programs saved by this editor, including on other pages. This
/// reads only the open PDF; it never installs or exports an embedded font.
pub(crate) fn embedded_faces(doc: &pdfcraft_cos::Document) -> Vec<MatchedFace> {
    // Our full-program stream dictionaries use this literal key. Ordinary PDFs
    // need no object scan merely to open a document-font cache.
    if !doc.bytes().windows(b"/PCFullFont".len()).any(|part| part == b"/PCFullFont") {
        return Vec::new();
    }
    let mut result: Vec<MatchedFace> = Vec::new();
    let mut remaining = 32usize << 20;
    let mut decode_budget = 64usize << 20;
    for (_, object) in doc.scan_objects().take(8192) {
        if result.len() >= 64 || remaining == 0 || decode_budget == 0 {
            break;
        }
        let pdfcraft_cos::Object::Stream(stream) = object.as_ref() else { continue };
        if !matches!(stream.dict.get(b"PCFullFont"), Some(pdfcraft_cos::Object::Bool(true))) {
            continue;
        }
        let limit = remaining.min(16 << 20).min(decode_budget);
        let bytes = match stream.decoded_within(limit) {
            Ok(bytes) => {
                decode_budget = decode_budget.saturating_sub(bytes.len());
                bytes
            }
            Err(_) => {
                decode_budget = decode_budget.saturating_sub(limit);
                continue;
            }
        };
        let Some(info) = pdfcraft_fonts::document_face::faces(&bytes).into_iter().next() else { continue };
        if result.iter().any(|f| f.info.postscript == info.postscript) || pdfcraft_fonts::embedding::validate(&bytes).is_err() {
            continue;
        }
        remaining = remaining.saturating_sub(bytes.len());
        result.push(MatchedFace { info, bytes: std::sync::Arc::new(bytes) });
    }
    result
}

impl crate::Document {
    /// Resolve original families on explicit editing requests. This changes only a
    /// document-scoped memory cache: opening/cancelling the editor is not an edit.
    pub fn match_text_fonts(&mut self, page: usize, files: &[FontFile]) -> Result<Vec<String>, String> {
        use std::io::Read;
        if page >= self.info.pages.len() {
            return Err("page is out of range".into());
        }
        let wanted: std::collections::HashSet<_> =
            self.text_lines(page).iter().map(|l| pdfcraft_fonts::document_face::family_key(&l.base_font)).collect();
        let mut remaining = (32usize << 20).saturating_sub(self.matched_text_fonts.iter().map(|f| f.bytes.len()).sum::<usize>());
        let mut matched = Vec::new();
        for file in files.iter().take(2048).filter(|f| wanted.contains(&pdfcraft_fonts::document_face::family_key(&f.face))) {
            if self.matched_text_fonts.len() >= 64 || remaining == 0 {
                break;
            }
            if self.matched_text_fonts.iter().any(|f| f.info.postscript == file.face) {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(&file.path) else { continue };
            if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 16 << 20 {
                continue;
            }
            let mut bytes = Vec::new();
            if std::fs::File::open(&file.path).and_then(|f| f.take((16 << 20) + 1).read_to_end(&mut bytes)).is_err() {
                continue;
            }
            let Some(info) = pdfcraft_fonts::document_face::faces(&bytes).into_iter().find(|f| {
                f.index == file.index && f.postscript == file.face && wanted.contains(&pdfcraft_fonts::document_face::family_key(&f.postscript))
            }) else {
                continue;
            };
            let Ok(bytes) = pdfcraft_fonts::document_face::select(&bytes, file.index) else { continue };
            if bytes.len() > remaining || self.matched_text_fonts.len() >= 64 {
                continue;
            }
            remaining = remaining.saturating_sub(bytes.len());
            matched.push(info.postscript.clone());
            self.matched_text_fonts.push(MatchedFace { info, bytes: std::sync::Arc::new(bytes) });
        }
        Ok(matched)
    }
}

impl crate::Session {
    pub fn match_text_fonts(&mut self, id: crate::DocId, page: usize, files: &[FontFile]) -> Result<Vec<String>, String> {
        self.doc_mut(id).map_err(|e| e.to_string())?.match_text_fonts(page, files)
    }
    pub fn match_text_fonts_once(&mut self, id: crate::DocId, page: usize, files: &[FontFile]) -> Result<Vec<String>, String> {
        let doc = self.doc_mut(id).map_err(|e| e.to_string())?;
        if doc.font_match_checked == Some((page, doc.generation)) {
            return Ok(Vec::new());
        }
        let result = doc.match_text_fonts(page, files)?;
        doc.font_match_checked = Some((page, doc.generation));
        Ok(result)
    }
}

pub(crate) fn embed_matches(doc: &mut pdfcraft_cos::Document, page: usize, faces: &[MatchedFace]) -> Result<(), crate::EditError> {
    if faces.is_empty() {
        return Ok(());
    }
    let wanted: std::collections::HashSet<_> =
        pdfcraft_edit::text_lines(doc, page)?.iter().map(|l| pdfcraft_fonts::document_face::family_key(&l.base_font)).collect();
    for face in faces.iter().filter(|f| wanted.contains(&pdfcraft_fonts::document_face::family_key(&f.info.postscript))) {
        pdfcraft_edit::text::register_text_font(doc, page, &face.bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockStyle, Edit, Session};
    use std::sync::Arc;
    #[test]
    fn matching_is_read_only_and_fonts_join_the_text_edit_undo_step() {
        let bytes = crate::tests::fixture(2);
        let mut cos = pdfcraft_cos::Document::open(Arc::new(bytes)).unwrap();
        cos.update_dict(pdfcraft_cos::ObjRef::new(3, 0), |f| {
            f.set(b"BaseFont".to_vec(), pdfcraft_cos::Object::name("ABCDEF+Inter-Regular"));
            f.set(b"FirstChar".to_vec(), pdfcraft_cos::Object::Int(97));
            f.set(b"LastChar".to_vec(), pdfcraft_cos::Object::Int(99));
            f.set(b"Widths".to_vec(), pdfcraft_cos::Object::Array(vec![pdfcraft_cos::Object::Int(600); 3]));
        })
        .unwrap();
        let content = pdfcraft_cos::Object::Ref(cos.add(pdfcraft_cos::Object::Stream(pdfcraft_cos::Stream::from_raw(
            pdfcraft_cos::Dict::new(),
            b"BT /F1 16 Tf 20 150 Td (abc) Tj ET".to_vec(),
        ))));
        cos.update_dict(pdfcraft_cos::ObjRef::new(4, 0), |p| p.set(b"Contents".to_vec(), content)).unwrap();
        let original = Arc::new(pdfcraft_cos::write_full(&cos, &Default::default()).unwrap());
        let dir = std::env::temp_dir().join(format!("pdfcraft-font-match-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("inter.ttf"), include_bytes!("../../../assets/fonts/Inter-Regular.ttf")).unwrap();
        std::fs::write(dir.join("inter-semibold.ttf"), include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf")).unwrap();
        let mut files = discover(std::slice::from_ref(&dir));
        assert_eq!(files.len(), 2);
        let mut removed = files[0].clone();
        removed.path = dir.join("removed-after-discovery.ttf");
        files.insert(0, removed);
        let mut session = Session::new();
        let id = session.open("source.pdf", None, original.clone(), None).unwrap();
        assert_eq!(session.match_text_fonts(id, 0, &files).unwrap(), ["Inter-Regular", "Inter-SemiBold"]);
        let doc = session.get(id).unwrap();
        assert!(!doc.dirty);
        assert!(doc.can_undo().is_none());
        assert_eq!(doc.bytes, original);
        assert!(doc.text_editor_fonts(0).iter().any(|f| f.resource == "F1"));
        session.apply(id, Edit::EditTextBlock { page: 0, block: 0, text: "abcé".into(), style: BlockStyle::default() }).unwrap();
        let doc = session.get(id).unwrap();
        assert!(doc.text_lines(0).iter().all(|l| l.base_font.ends_with("Inter-Regular")));
        assert!(doc.text_editor_fonts(0).iter().any(|f| f.resource.starts_with("PCFull")));
        let saved = doc.bytes.clone();
        session.undo(id).unwrap();
        assert_eq!(session.get(id).unwrap().bytes, original);
        session.redo(id).unwrap();
        assert_eq!(session.get(id).unwrap().bytes, saved);
        std::fs::remove_file(dir.join("inter.ttf")).unwrap();
        std::fs::remove_file(dir.join("inter-semibold.ttf")).unwrap();
        std::fs::remove_dir(dir).unwrap();
        let other = session.open("second.pdf", None, saved, None).unwrap();
        assert_eq!(session.get(other).unwrap().matched_text_fonts.len(), 2);
        assert!(!session.get(other).unwrap().dirty);
        // Page two still has only the original subset resource. Complete programs
        // saved on page one must also supply its preview and subsequent edit.
        assert!(session.get(other).unwrap().text_editor_fonts(1).iter().any(|f| f.resource == "F1"));
        session.apply(other, Edit::EditTextBlock { page: 1, block: 0, text: "Page 2 Ω".into(), style: BlockStyle::default() }).unwrap();
        let lines = session.get(other).unwrap().text_lines(1);
        assert!(lines.iter().all(|l| l.base_font.ends_with("Inter-Regular")));
        assert!(lines.iter().any(|l| l.text.contains('Ω') && l.base_font == "Inter-Regular"));
        session.apply(other, Edit::EditTextBlock { page: 0, block: 0, text: "abcéz".into(), style: BlockStyle::default() }).unwrap();
        assert!(session.get(other).unwrap().text_lines(0).iter().all(|l| l.base_font.ends_with("Inter-Regular")));
        session
            .apply(
                other,
                Edit::EditTextBlock {
                    page: 0,
                    block: 0,
                    text: "abcéz".into(),
                    style: BlockStyle { family: Some((crate::FontFamily::Helvetica, true, false)), ..Default::default() },
                },
            )
            .unwrap();
        assert!(
            session.get(other).unwrap().text_lines(0).iter().all(|l| l.base_font.ends_with("Inter-SemiBold")),
            "{:?}",
            session.get(other).unwrap().text_lines(0).iter().map(|l| &l.base_font).collect::<Vec<_>>()
        );
    }
}
