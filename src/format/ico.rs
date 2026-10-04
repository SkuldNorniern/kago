//! `.ico` file to the PE icon resources: one `RT_ICON` per image plus the
//! `RT_GROUP_ICON` directory that points at them.

/// icon resources ready to write into an executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconGroup {
    /// image data, written as `RT_ICON` ids 1, 2, ...
    pub images: Vec<Vec<u8>>,
    /// `GRPICONDIR`, written as `RT_GROUP_ICON`.
    pub group: Vec<u8>,
}

/// # Errors
///
/// when `ico` is not a well-formed icon file.
pub fn parse(ico: &[u8]) -> Result<IconGroup, String> {
    let u16_at = |i: usize| ico.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |i: usize| {
        ico.get(i..i + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let bad = |what: &str| format!("not an icon file: {what}");
    if u16_at(0) != Some(0) || u16_at(2) != Some(1) {
        return Err(bad("wrong header"));
    }
    let count = u16_at(4).ok_or_else(|| bad("short header"))?;
    if count == 0 {
        return Err(bad("no images"));
    }
    let mut images = Vec::new();
    let mut group = Vec::with_capacity(6 + 14 * usize::from(count));
    group.extend_from_slice(&[0, 0, 1, 0]);
    group.extend_from_slice(&count.to_le_bytes());
    for n in 0..count {
        let e = 6 + 16 * usize::from(n);
        let entry = ico.get(e..e + 16).ok_or_else(|| bad("short directory"))?;
        let size = u32_at(e + 8).ok_or_else(|| bad("short directory"))?;
        let offset = u32_at(e + 12).ok_or_else(|| bad("short directory"))?;
        let start = usize::try_from(offset).map_err(|_| bad("offset"))?;
        let end = start
            .checked_add(usize::try_from(size).map_err(|_| bad("size"))?)
            .ok_or_else(|| bad("size"))?;
        let image = ico
            .get(start..end)
            .ok_or_else(|| bad("image past the end"))?;
        images.push(image.to_vec());
        // width, height, colors, reserved, planes (at least 1), bit count,
        // bytes, then the resource id in place of the file offset.
        group.extend_from_slice(&entry[..4]);
        let planes = u16::from_le_bytes([entry[4], entry[5]]).max(1);
        group.extend_from_slice(&planes.to_le_bytes());
        group.extend_from_slice(&entry[6..12]);
        group.extend_from_slice(&(n + 1).to_le_bytes());
    }
    Ok(IconGroup { images, group })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ico(images: &[&[u8]]) -> Vec<u8> {
        let mut out = vec![0, 0, 1, 0];
        out.extend_from_slice(&u16::try_from(images.len()).unwrap().to_le_bytes());
        let mut offset = 6 + 16 * images.len();
        for (i, image) in images.iter().enumerate() {
            let side = u8::try_from(16 * (i + 1)).unwrap();
            out.extend_from_slice(&[side, side, 0, 0, 0, 0, 32, 0]);
            out.extend_from_slice(&u32::try_from(image.len()).unwrap().to_le_bytes());
            out.extend_from_slice(&u32::try_from(offset).unwrap().to_le_bytes());
            offset += image.len();
        }
        for image in images {
            out.extend_from_slice(image);
        }
        out
    }

    #[test]
    fn builds_group_with_ids() {
        let g = parse(&ico(&[b"first", b"second!"])).unwrap();
        assert_eq!(g.images, [b"first".to_vec(), b"second!".to_vec()]);
        assert_eq!(g.group.len(), 6 + 2 * 14);
        assert_eq!(&g.group[6..8], &[16, 16]);
        // planes forced to 1, then 32 bpp, size 5, id 1.
        assert_eq!(&g.group[10..20], &[1, 0, 32, 0, 5, 0, 0, 0, 1, 0]);
        assert_eq!(&g.group[6 + 14 + 12..6 + 14 + 14], &[2, 0]);
    }

    #[test]
    fn rejects_broken_files() {
        assert!(parse(b"PNG").is_err());
        let mut short = ico(&[b"image"]);
        short.truncate(short.len() - 2);
        assert!(parse(&short).is_err());
    }
}
