//! Colour book files (`.acb`): the binary swatch books colour vendors publish for design apps
//! (read only). A book is a header (`8BCB`, version 1, a book id, then title, name prefix, name
//! postfix and description as UTF-16 strings), a record per colour (name, a six-character code and
//! 8-bit components in the book's colour space: RGB, CMYK or Lab) and an optional `spflspot` /
//! `spflproc` trailer telling spot books from process ones. Everything is big-endian.
//!
//! VectorCraft ships no colour books: users load the ones they are licensed for.

use std::collections::HashSet;

use crate::{Color, Paint, Swatch, SwatchLibrary};

const SIGNATURE: &[u8] = b"8BCB";
/// The trailer of a process book (spot ones end in `spflspot`).
const PROCESS: &[u8] = b"spflproc";

/// The colour spaces books come in (the header's colour space id).
#[derive(Clone, Copy)]
enum Space {
    Rgb,
    Cmyk,
    Lab,
}

impl Space {
    fn from_id(id: u16) -> Option<Self> {
        match id {
            0 => Some(Space::Rgb),
            2 => Some(Space::Cmyk),
            7 => Some(Space::Lab),
            _ => None,
        }
    }
    fn components(self) -> usize {
        match self {
            Space::Cmyk => 4,
            Space::Rgb | Space::Lab => 3,
        }
    }
    /// A record's components as a colour: RGB as is, CMYK inverted (255 is no ink), Lab with L over
    /// 0..=255 and a and b offset by 128.
    fn color(self, v: &[u8]) -> Option<Color> {
        let f = |i: usize| v.get(i).map(|&b| f32::from(b));
        Some(match self {
            Space::Rgb => Color::rgb(f(0)? / 255.0, f(1)? / 255.0, f(2)? / 255.0),
            Space::Cmyk => Color::cmyk(1.0 - f(0)? / 255.0, 1.0 - f(1)? / 255.0, 1.0 - f(2)? / 255.0, 1.0 - f(3)? / 255.0),
            Space::Lab => Color::lab(f(0)? / 255.0 * 100.0, f(1)? - 128.0, f(2)? - 128.0),
        })
    }
}

/// Big-endian reads that fail at the end of the data.
struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let (head, tail) = self.rest.split_at_checked(n).ok_or("colour book is cut short")?;
        self.rest = tail;
        Ok(head)
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from(self.u16()?) << 16 | u32::from(self.u16()?))
    }
    /// A string: a count of UTF-16 code units, then the units. The count can't pass the end of the
    /// data, which caps the allocation.
    fn string(&mut self) -> Result<String, String> {
        let units = self.u32()? as usize;
        let bytes = self.take(units.checked_mul(2).ok_or("colour book is cut short")?)?;
        let utf16: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|&c| u16::from_be_bytes(c)).collect();
        Ok(String::from_utf16_lossy(&utf16))
    }
}

/// A header string's text: without the `$$$/…/key=` localization key and the quotes around it,
/// with `^C` and `^R` as © and ®.
fn text(s: &str) -> String {
    let s = if s.starts_with("$$$") { s.split_once('=').map_or("", |(_, v)| v) } else { s };
    let s = s.strip_prefix('"').and_then(|t| t.strip_suffix('"')).unwrap_or(s);
    s.replace("^C", "©").replace("^R", "®").replace('\0', "")
}

/// Is `bytes` a colour book?
pub fn sniff(bytes: &[u8]) -> bool {
    bytes.starts_with(SIGNATURE)
}

/// Read a colour book. Its colours come as swatches named prefix + name + postfix in the book's
/// order (the empty records that pad a book's pages are skipped; repeated names get a number), spot
/// and global unless the book says it is a process one. `name` names the library when the book has
/// no title.
pub fn read(bytes: &[u8], name: &str) -> Result<SwatchLibrary, String> {
    let mut r = Reader { rest: bytes };
    if r.take(SIGNATURE.len()).ok() != Some(SIGNATURE) {
        return Err("not a colour book (.acb)".into());
    }
    let version = r.u16()?;
    if version != 1 {
        return Err(format!("unsupported colour book version {version}"));
    }
    let _book_id = r.u16()?;
    let title = text(&r.string()?);
    let prefix = text(&r.string()?);
    let postfix = text(&r.string()?);
    let _description = r.string()?;
    let count = r.u16()?;
    let (_page_size, _page_offset) = (r.u16()?, r.u16()?);
    let space = r.u16()?;
    let space = Space::from_id(space).ok_or_else(|| format!("unsupported colour book colour space {space} (RGB, CMYK and Lab books are read)"))?;

    let mut colors: Vec<(String, Color)> = vec![];
    let mut names: HashSet<String> = HashSet::new();
    for _ in 0..count {
        let own = text(&r.string()?);
        let _code = r.take(6)?;
        let color = space.color(r.take(space.components())?).ok_or("colour book is cut short")?;
        if own.trim().is_empty() {
            continue;
        }
        let base = format!("{prefix}{own}{postfix}").trim().to_string();
        let name = (1..).map(|i| if i == 1 { base.clone() } else { format!("{base} {i}") }).find(|n| !names.contains(n)).unwrap_or(base);
        names.insert(name.clone());
        colors.push((name, color));
    }
    // Books without the trailer are spot ones (it came with the process books).
    let spot = !r.rest.starts_with(PROCESS);

    // The title is often the file's name.
    let title = title.trim();
    let title = if title.to_ascii_lowercase().ends_with(".acb") { title.get(..title.len() - 4).unwrap_or(title) } else { title };
    Ok(SwatchLibrary {
        name: if title.is_empty() { name.into() } else { title.into() },
        swatches: colors.into_iter().map(|(name, color)| Swatch { name, paint: Paint::solid(color), global: spot, spot }).collect(),
        groups: vec![],
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const SPOT: &[u8] = b"spflspot";

    fn string(out: &mut Vec<u8>, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        out.extend((units.len() as u32).to_be_bytes());
        out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
    }

    /// A synthetic colour book: `space` id, records of (name, components), an optional trailer.
    pub(crate) fn book(title: &str, prefix: &str, postfix: &str, space: u16, colors: &[(&str, &[u8])], trailer: &[u8]) -> Vec<u8> {
        let mut out = b"8BCB".to_vec();
        out.extend([0, 1, 0x0b, 0xb8]);
        string(&mut out, title);
        string(&mut out, prefix);
        string(&mut out, postfix);
        string(&mut out, "$$$/colorbook/Test/description=\"Copyright^C Nobody\"");
        out.extend((colors.len() as u16).to_be_bytes());
        out.extend([0, 7, 0, 1]);
        out.extend(space.to_be_bytes());
        for (i, (name, v)) in colors.iter().enumerate() {
            string(&mut out, name);
            out.extend(format!("{i:06}").as_bytes());
            out.extend(*v);
        }
        out.extend(trailer);
        out
    }

    #[test]
    fn lab_spot_book_reads_names_colours_and_skips_padding() {
        let bytes = book(
            "$$$/colorbook/Test/title=Test Solid.acb",
            "$$$/colorbook/Test/prefix=TEST ",
            "$$$/colorbook/Test/postfix= C",
            7,
            &[("100", &[255, 128, 128]), ("", &[0, 0, 0]), ("Red 032", &[128, 208, 178]), ("100", &[0, 0, 255])],
            SPOT,
        );
        assert!(sniff(&bytes));
        let lib = read(&bytes, "file").unwrap();
        assert_eq!(lib.name, "Test Solid");
        let names: Vec<&str> = lib.swatches.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["TEST 100 C", "TEST Red 032 C", "TEST 100 C 2"]);
        assert!(lib.swatches.iter().all(|w| w.spot && w.global));
        assert_eq!(lib.swatches[0].paint.color(), Some(Color::lab(100.0, 0.0, 0.0)));
        let Some(Color::Lab { l, a, b }) = lib.swatches[1].paint.color() else { panic!("Lab") };
        assert!((l - 50.196).abs() < 0.01 && a == 80.0 && b == 50.0, "{l} {a} {b}");
        assert_eq!(lib.swatches[2].paint.color(), Some(Color::lab(0.0, -128.0, 127.0)));
    }

    #[test]
    fn cmyk_process_and_rgb_books() {
        let lib = read(&book("", "", "", 2, &[("Sky", &[0, 127, 255, 204])], PROCESS), "Fallback").unwrap();
        assert_eq!(lib.name, "Fallback", "an untitled book takes the file's name");
        let w = &lib.swatches[0];
        assert!(!w.spot && !w.global, "a process book");
        let Some(Color::Cmyk { c, m, y, k }) = w.paint.color() else { panic!("CMYK") };
        assert!(c == 1.0 && (m - 0.502).abs() < 0.001 && y == 0.0 && (k - 0.2).abs() < 0.001, "{c} {m} {y} {k}");
        let lib = read(&book("Plain", "", "", 0, &[("Orange", &[255, 102, 0])], b""), "x").unwrap();
        assert_eq!(lib.swatches[0].paint.color(), Some(Color::rgb8(255, 102, 0)));
        assert!(lib.swatches[0].spot, "no trailer: a spot book");
    }

    #[test]
    fn damaged_books_are_errors() {
        let bytes = book("T", "", "", 7, &[("A", &[1, 2, 3]), ("B", &[4, 5, 6])], SPOT);
        for cut in 0..bytes.len() - SPOT.len() {
            assert!(read(&bytes[..cut], "x").is_err(), "cut at {cut}");
        }
        assert!(read(&bytes[..bytes.len() - SPOT.len()], "x").is_ok(), "the trailer is optional");
        assert!(read(b"8BCX", "x").is_err());
        assert!(read(&book("T", "", "", 1, &[], b""), "x").unwrap_err().contains("colour space 1"));
        let mut v2 = bytes.clone();
        v2[5] = 2;
        assert!(read(&v2, "x").unwrap_err().contains("version 2"));
        // A string length far past the end of the data.
        let mut huge = b"8BCB\0\x01\0\0".to_vec();
        huge.extend([0xff; 4]);
        assert!(read(&huge, "x").is_err());
    }
}
