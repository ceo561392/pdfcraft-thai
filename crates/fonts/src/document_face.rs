//! Bounded document-face selection, including TrueType collections. Runtime inputs only.
use skrifa::{FontRef, MetadataProvider, raw::TableProvider, string::StringId};

#[derive(Clone, Debug)]
pub struct FaceInfo {
    pub index: u32,
    pub family: String,
    pub postscript: String,
    pub bold: bool,
    pub italic: bool,
    pub open_license: bool,
}

/// Family identity from PDF/PostScript names, preserving distinct families and styles.
pub fn family_key(name: &str) -> String {
    let mut key = crate::embedding::face_key(name);
    if key.ends_with("mt") {
        key.truncate(key.len().saturating_sub(2));
    }
    for suffix in ["semibolditalic", "demibolditalic", "bolditalic", "boldoblique", "semibold", "demibold", "bold", "italic", "oblique", "regular"] {
        if key.len() > suffix.len() && key.ends_with(suffix) {
            key.truncate(key.len().saturating_sub(suffix.len()));
            break;
        }
    }
    key
}

fn name(face: &FontRef<'_>, id: StringId) -> Option<String> {
    face.localized_strings(id)
        .find(|n| n.language() == Some("en"))
        .or_else(|| face.localized_strings(id).next())
        .map(|v| v.chars().take(257).collect::<String>())
        .filter(|s| !s.is_empty() && s.len() <= 256)
}

/// Read at most 32 faces. Ineligible faces are excluded, never silently relaxed.
pub fn faces(bytes: &[u8]) -> Vec<FaceInfo> {
    if bytes.len() > 16 << 20 {
        return Vec::new();
    }
    let mut result = Vec::new();
    for index in 0..32 {
        let Ok(face) = FontRef::from_index(bytes, index) else { break };
        if face.fvar().is_ok() {
            continue;
        }
        let Some(os2) = face.os2().ok() else { continue };
        let flags = os2.fs_type();
        if flags & 0x0206 != 0 || flags & 0x000f != 0 && flags & 0x000f != 8 {
            continue;
        }
        let Some(family) = name(&face, StringId::FAMILY_NAME) else { continue };
        let Some(postscript) = name(&face, StringId::POSTSCRIPT_NAME) else { continue };
        let credits = [StringId::COPYRIGHT_NOTICE, StringId::MANUFACTURER]
            .into_iter()
            .flat_map(|id| face.localized_strings(id).take(16))
            .map(|v| v.chars().take(4096).collect::<String>())
            .collect::<String>()
            .to_lowercase();
        let key = family_key(&family);
        if credits.contains("adobe") || ["sourcehan", "sourceserif", "sourcesans", "notosanscjk", "notoserifcjk"].iter().any(|p| key.starts_with(p)) {
            continue;
        }
        let license =
            face.localized_strings(StringId::LICENSE_DESCRIPTION).take(16).map(|v| v.chars().take(4096).collect::<String>()).collect::<String>();
        let open_license = ["SIL OPEN FONT", "SIL Open Font", "Apache License", "Ubuntu Font Licence", "Bitstream Vera", "MIT License"]
            .iter()
            .any(|s| license.contains(s));
        let selection = os2.fs_selection().bits();
        result.push(FaceInfo {
            index,
            family,
            postscript,
            bold: os2.us_weight_class() >= 600 || selection & 0x20 != 0,
            italic: selection & 0x201 != 0,
            open_license,
        });
    }
    result
}

/// Copy one collection face into an sfnt stream for document embedding. Glyphs,
/// names, notices and layout tables are unchanged; only container offsets/checksums change.
/// No subsetting occurs, including for fonts carrying the no-subsetting flag.
pub fn select(bytes: &[u8], index: u32) -> Result<Vec<u8>, String> {
    if bytes.len() > 16 << 20 {
        return Err("font input exceeds 16 MiB".into());
    }
    let font = FontRef::from_index(bytes, index).map_err(|_| "invalid font face index or collection")?;
    if font.ttc_index().is_none() {
        crate::embedding::validate(bytes)?;
        return Ok(bytes.to_vec());
    }
    let tables: Vec<_> = font.table_directory().table_records().iter().filter(|t| t.tag.get().to_be_bytes() != *b"DSIG").collect();
    let count = u16::try_from(tables.len()).ok().filter(|n| *n > 0 && *n <= 256).ok_or("invalid font table count")?;
    let power = 1u16.checked_shl(u16::BITS - 1 - count.leading_zeros()).ok_or("font directory size")?;
    let mut out = Vec::new();
    out.extend(font.table_directory().sfnt_version().to_be_bytes());
    out.extend(count.to_be_bytes());
    out.extend((power * 16).to_be_bytes());
    out.extend((power.ilog2() as u16).to_be_bytes());
    out.extend((count * 16 - power * 16).to_be_bytes());
    let directory = 12 + usize::from(count) * 16;
    out.resize(directory, 0);
    let mut head = None;
    for (i, record) in tables.iter().enumerate() {
        let tag = record.tag.get();
        let data = font.table_data(tag).ok_or("font table outside input")?;
        let data = data.as_bytes();
        let at = out.len();
        let end = at
            .checked_add(data.len())
            .and_then(|n| n.checked_add(3))
            .map(|n| n & !3)
            .filter(|n| *n <= 16 << 20)
            .ok_or("selected font exceeds 16 MiB")?;
        out.extend_from_slice(data);
        out.resize(end, 0);
        if tag.to_be_bytes() == *b"head" {
            out.get_mut(at + 8..at + 12).ok_or("short font head")?.fill(0);
            head = Some(at + 8);
        }
        let sum = checksum(out.get(at..end).ok_or("font table bounds")?);
        let mut entry = Vec::with_capacity(16);
        entry.extend(tag.to_be_bytes());
        entry.extend(sum.to_be_bytes());
        entry.extend((at as u32).to_be_bytes());
        entry.extend((data.len() as u32).to_be_bytes());
        out.get_mut(12 + i * 16..28 + i * 16).ok_or("font directory bounds")?.copy_from_slice(&entry);
    }
    let adjustment = 0xb1b0afbau32.wrapping_sub(checksum(&out));
    let head = head.ok_or("font has no head table")?;
    out.get_mut(head..head + 4).ok_or("font checksum bounds")?.copy_from_slice(&adjustment.to_be_bytes());
    crate::embedding::validate(&out)?;
    Ok(out)
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        for (to, from) in word.iter_mut().zip(chunk) {
            *to = *from;
        }
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn collection() -> Vec<u8> {
        let inputs = [
            include_bytes!("../../../assets/fonts/Inter-Regular.ttf").as_slice(),
            include_bytes!("../../../assets/fonts/Inter-Medium.ttf").as_slice(),
        ];
        let mut out = b"ttcf\0\x01\0\0\0\0\0\x02".to_vec();
        out.resize(20, 0);
        for (index, input) in inputs.into_iter().enumerate() {
            let offset = out.len();
            out[12 + index * 4..16 + index * 4].copy_from_slice(&(offset as u32).to_be_bytes());
            out.extend(input);
            let count = u16::from_be_bytes(input[4..6].try_into().unwrap()) as usize;
            for i in 0..count {
                let record = offset + 12 + i * 16 + 8;
                let old = u32::from_be_bytes(out[record..record + 4].try_into().unwrap());
                out[record..record + 4].copy_from_slice(&(old + offset as u32).to_be_bytes());
            }
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
        }
        out
    }
    #[test]
    fn collection_selection_preserves_faces_tables_permissions_and_checksums() {
        let bytes = collection();
        let infos = faces(&bytes);
        assert_eq!(infos.len(), 2);
        assert!(infos.iter().all(|f| f.open_license));
        for (index, wanted) in ["Inter-Regular", "Inter-Medium"].into_iter().enumerate() {
            let selected = select(&bytes, index as u32).unwrap();
            assert_eq!(crate::embedding::names(&selected).unwrap().1, wanted);
            assert_eq!(checksum(&selected), 0xb1b0afba);
            let a = FontRef::from_index(&bytes, index as u32).unwrap();
            let b = FontRef::new(&selected).unwrap();
            for tag in [b"glyf", b"GSUB", b"GPOS", b"name", b"OS/2"] {
                let tag = skrifa::raw::types::Tag::new(tag);
                assert_eq!(a.table_data(tag).map(|d| d.as_bytes()), b.table_data(tag).map(|d| d.as_bytes()));
            }
        }
        assert!(select(&bytes, 2).is_err());
    }
    #[test]
    fn invalid_collections_and_restricted_permissions_are_not_selected() {
        let bytes = collection();
        for end in [0, 4, 12, 19, 32, 100] {
            assert!(select(&bytes[..end], 0).is_err());
        }
        let mut broken = bytes.clone();
        broken[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(select(&broken, 0).is_err());
        let mut restricted = include_bytes!("../../../assets/fonts/Inter-Regular.ttf").to_vec();
        let font = FontRef::new(&restricted).unwrap();
        let offset = font.table_directory().table_records().iter().find(|r| r.tag.get().to_be_bytes() == *b"OS/2").unwrap().offset.get() as usize;
        restricted[offset + 8..offset + 10].copy_from_slice(&2u16.to_be_bytes());
        assert!(faces(&restricted).is_empty());
        assert!(select(&restricted, 0).is_err());
        restricted[offset + 8..offset + 10].copy_from_slice(&0x0108u16.to_be_bytes());
        assert_eq!(faces(&restricted).len(), 1);
        let full = select(&restricted, 0).unwrap();
        assert_eq!(full, restricted, "editable/no-subsetting input must remain complete");
    }
    #[test]
    fn family_identity_keeps_regular_bold_and_italic_together() {
        for name in ["ABCDEF+CordiaNew", "CordiaNew-Bold", "CordiaNew-Italic", "CordiaNew-BoldItalic"] {
            assert_eq!(family_key(name), "cordianew");
        }
        assert_ne!(family_key("CordiaUPC"), family_key("Cordia New"));
        assert_ne!(family_key("Sarabun"), family_key("Cordia New"));
    }
}
