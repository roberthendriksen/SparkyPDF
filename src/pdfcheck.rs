//! Quick health check for PDF files, and recovery of page images from damaged ones.
//!
//! A PDF keeps its index (the cross-reference table and the list of pages) at the
//! very end of the file. When a download or copy is cut short that index is gone and
//! no PDF engine can open the file. Scanned documents, however, store every page as
//! one JPEG image near the start of the file, and those can be lifted out directly.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Files larger than this are handed to the viewer without a full scan.
const MAX_SCAN: u64 = 1 << 30;
/// An incomplete image needs at least this share of its data to be worth showing.
const MIN_PERCENT: u8 = 10;

pub enum Verdict {
    /// Structurally plausible: let the PDF engine open it.
    Healthy,
    /// Not a PDF at all; the text says what it looks like instead.
    NotPdf(&'static str),
    /// A PDF whose index is missing.
    Damaged(Report),
}

pub struct Image {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub complete: bool,
    /// For incomplete images: share of the image data that is present, if known.
    pub percent: Option<u8>,
}

#[derive(Default)]
pub struct Report {
    pub file_len: usize,
    /// Page images that can be shown.
    pub images: Vec<Image>,
    /// Images that had only just begun when the file ends.
    pub too_partial: usize,
    /// Images stored in a format this recovery cannot decode.
    pub unsupported: usize,
    /// The file still contains an index the PDF engine could use.
    pub has_index: bool,
    /// The file ends in the middle of an object.
    pub truncated: bool,
}

/// Decides how a file should be opened. Cheap for healthy files: only the first and
/// last few kilobytes are read.
pub fn check(path: &Path) -> io::Result<Verdict> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(Verdict::NotPdf("an empty file (0 bytes)"));
    }

    let mut head = vec![0u8; len.min(1024) as usize];
    file.read_exact(&mut head)?;
    if find(&head, b"%PDF-", 0).is_none() {
        return Ok(Verdict::NotPdf(describe(&head)));
    }

    let tail_len = len.min(4096);
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    let mut tail = vec![0u8; tail_len as usize];
    file.read_exact(&mut tail)?;
    if find(&tail, b"startxref", 0).is_some() || len > MAX_SCAN {
        return Ok(Verdict::Healthy);
    }
    drop(file);

    let report = scan(&std::fs::read(path)?);
    Ok(if report.has_index {
        Verdict::Healthy
    } else {
        Verdict::Damaged(report)
    })
}

/// Walks every object in the file and collects the page images it can find.
pub fn scan(data: &[u8]) -> Report {
    let mut report = Report {
        file_len: data.len(),
        has_index: find(data, b"startxref", 0).is_some()
            || find(data, b"trailer", 0).is_some()
            || find(data, b"/XRef", 0).is_some(),
        ..Report::default()
    };

    let mut pos = 0;
    let mut last_object = None;
    while let Some(at) = find(data, b"obj", pos) {
        pos = at + 3;
        if !is_object_header(data, at) {
            continue;
        }
        last_object = Some(at);

        let dict_start = skip_ws(data, at + 3);
        if !data[dict_start..].starts_with(b"<<") {
            continue;
        }
        let Some(dict_stop) = dict_end(data, dict_start) else {
            continue;
        };
        let dict = &data[dict_start..dict_stop];

        let keyword = skip_ws(data, dict_stop);
        if !data[keyword..].starts_with(b"stream") {
            pos = dict_stop;
            continue;
        }
        let mut start = keyword + 6;
        if data[start..].starts_with(b"\r\n") {
            start += 2;
        } else if matches!(data.get(start), Some(b'\n' | b'\r')) {
            start += 1;
        }

        let length = int_value(dict, b"/Length").map(|n| n as usize);
        let (end, complete) = stream_extent(data, start, length);
        pos = end.max(pos);

        if has_name(dict, b"/Subtype", b"/Image") {
            collect_image(&mut report, dict, &data[start.min(end)..end], complete, length);
        }
    }

    report.truncated = match last_object {
        Some(at) => find(data, b"endobj", at).is_none(),
        None => false,
    };
    report
}

fn collect_image(report: &mut Report, dict: &[u8], body: &[u8], complete: bool, length: Option<usize>) {
    if has_name(dict, b"/ImageMask", b"true") {
        return;
    }
    let width = int_value(dict, b"/Width").unwrap_or(0) as u32;
    let height = int_value(dict, b"/Height").unwrap_or(0) as u32;
    // Logos, icons and thumbnails are not pages.
    if width < 200 && height < 200 {
        return;
    }

    let filters = filter_names(dict);
    let is_jpeg = filters.len() == 1 && filters[0] == b"DCTDecode" && body.starts_with(&[0xFF, 0xD8]);
    if !is_jpeg {
        report.unsupported += 1;
        return;
    }

    if complete {
        report.images.push(Image { data: body.to_vec(), width, height, complete, percent: None });
        return;
    }

    let percent = length
        .filter(|&expected| expected > 0)
        .map(|expected| (body.len() as u64 * 100 / expected as u64).min(99) as u8);
    if body.len() < 2048 || percent.is_some_and(|p| p < MIN_PERCENT) {
        report.too_partial += 1;
        return;
    }
    // An end-of-image marker lets decoders show the part that is there.
    let mut data = body.to_vec();
    data.extend_from_slice(&[0xFF, 0xD9]);
    report.images.push(Image { data, width, height, complete, percent });
}

/// Says what a file that is not a PDF appears to be.
fn describe(head: &[u8]) -> &'static str {
    let text = head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(head);
    let text = &text[skip_ws(text, 0)..];
    let lower: Vec<u8> = text.iter().take(64).map(u8::to_ascii_lowercase).collect();

    if head.iter().all(|&b| b == 0) {
        "blank data (nothing but zeros)"
    } else if lower.starts_with(b"<!doctype html") || lower.starts_with(b"<html") {
        "a web page (HTML) saved under a .pdf name"
    } else if lower.starts_with(b"<?xml") {
        "an XML document"
    } else if head.starts_with(b"PK\x03\x04") {
        "a ZIP archive or an Office document (Word, Excel, PowerPoint)"
    } else if head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        "an older Office document"
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "a JPEG picture"
    } else if head.starts_with(b"\x89PNG") {
        "a PNG picture"
    } else if lower.starts_with(b"{\\rtf") {
        "a Rich Text (RTF) document"
    } else {
        "some other kind of file (it does not start with the PDF signature)"
    }
}

// ---- low-level helpers ------------------------------------------------------

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let last = hay.len() - needle.len();
    let mut i = from;
    while i <= last {
        i += hay[i..=last].iter().position(|&b| b == needle[0])?;
        if &hay[i..i + needle.len()] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn is_ws(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

fn skip_ws(data: &[u8], mut i: usize) -> usize {
    while i < data.len() && is_ws(data[i]) {
        i += 1;
    }
    i
}

/// True when the "obj" at `at` is the keyword of an object header: `12 0 obj`.
fn is_object_header(data: &[u8], at: usize) -> bool {
    if data.get(at + 3).is_some_and(u8::is_ascii_alphanumeric) {
        return false;
    }
    let back = |mut i: usize, pred: fn(u8) -> bool| {
        let from = i;
        while i > 0 && pred(data[i - 1]) {
            i -= 1;
        }
        (i != from).then_some(i)
    };
    let digit = |b: u8| b.is_ascii_digit();
    back(at, is_ws)
        .and_then(|i| back(i, digit))
        .and_then(|i| back(i, is_ws))
        .and_then(|i| back(i, digit))
        .is_some()
}

/// `start` points at `<<`; returns the index just past the matching `>>`.
fn dict_end(data: &[u8], start: usize) -> Option<usize> {
    let limit = (start + 65536).min(data.len());
    let mut i = start;
    let mut depth = 0;
    while i < limit {
        match data[i] {
            b'<' if data.get(i + 1) == Some(&b'<') => {
                depth += 1;
                i += 2;
            }
            b'>' if data.get(i + 1) == Some(&b'>') => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'<' => {
                // hex string
                while i < limit && data[i] != b'>' {
                    i += 1;
                }
                i += 1;
            }
            b'(' => {
                // literal string, possibly with nested parentheses
                let mut nesting = 1;
                i += 1;
                while i < limit && nesting > 0 {
                    match data[i] {
                        b'\\' => i += 1,
                        b'(' => nesting += 1,
                        b')' => nesting -= 1,
                        _ => {}
                    }
                    i += 1;
                }
            }
            b'%' => {
                while i < limit && data[i] != b'\n' && data[i] != b'\r' {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    None
}

/// Every position in `dict` where `key` occurs as a whole name; yields what follows it.
fn values<'a>(dict: &'a [u8], key: &'a [u8]) -> impl Iterator<Item = &'a [u8]> {
    let mut from = 0;
    std::iter::from_fn(move || {
        loop {
            let at = find(dict, key, from)?;
            from = at + key.len();
            if !dict.get(from).is_some_and(u8::is_ascii_alphanumeric) {
                return Some(&dict[skip_ws(dict, from)..]);
            }
        }
    })
}

/// A direct integer value. Indirect references (`12 0 R`) yield `None`.
fn int_value(dict: &[u8], key: &[u8]) -> Option<u64> {
    let rest = values(dict, key).next()?;
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || digits > 12 {
        return None;
    }
    let next = skip_ws(rest, digits);
    let generation = rest[next..].iter().take_while(|b| b.is_ascii_digit()).count();
    if generation > 0 && rest.get(skip_ws(rest, next + generation)) == Some(&b'R') {
        return None;
    }
    std::str::from_utf8(&rest[..digits]).ok()?.parse().ok()
}

fn has_name(dict: &[u8], key: &[u8], name: &[u8]) -> bool {
    values(dict, key)
        .any(|rest| rest.starts_with(name) && !rest.get(name.len()).is_some_and(u8::is_ascii_alphanumeric))
}

/// The names listed under /Filter, whether written as one name or as an array.
fn filter_names(dict: &[u8]) -> Vec<&[u8]> {
    let Some(rest) = values(dict, b"/Filter").next() else {
        return Vec::new();
    };
    let (list, many) = match rest.first() {
        Some(b'[') => (&rest[1..rest.iter().position(|&b| b == b']').unwrap_or(rest.len())], true),
        _ => (rest, false),
    };
    let mut names = Vec::new();
    let mut i = 0;
    while i < list.len() {
        if list[i] == b'/' {
            let len = list[i + 1..]
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric())
                .count();
            names.push(&list[i + 1..i + 1 + len]);
            i += 1 + len;
        } else {
            i += 1;
        }
        if !many {
            break;
        }
    }
    names
}

/// Where a stream's data ends, and whether all of it is present.
fn stream_extent(data: &[u8], start: usize, length: Option<usize>) -> (usize, bool) {
    if let Some(end) = length.and_then(|len| start.checked_add(len)) {
        if end <= data.len() {
            let after = skip_ws(data, end);
            if after == data.len() || data[after..].starts_with(b"endstream") {
                return (end, true);
            }
        }
    }
    match find(data, b"endstream", start) {
        Some(mut end) => {
            if end > start && data[end - 1] == b'\n' {
                end -= 1;
            }
            if end > start && data[end - 1] == b'\r' {
                end -= 1;
            }
            (end, true)
        }
        None => (data.len(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(len: usize) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0];
        v.resize(len, 0x55);
        v
    }

    fn image_object(number: u32, body: &[u8], declared: usize) -> Vec<u8> {
        let mut v = format!(
            "{number} 0 obj\n<</Type /XObject /Subtype /Image /Width 1786 /Height 2527 \
             /ColorSpace /DeviceRGB /Filter /DCTDecode /Length {declared} >>\nstream\n"
        )
        .into_bytes();
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn recovers_complete_and_partial_images_from_a_cut_off_file() {
        let mut pdf = b"%PDF-1.7\n%\xA9\xCD\xC4\xD2\n".to_vec();
        pdf.extend(image_object(2, &jpeg(5000), 5000));
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
        pdf.extend(image_object(3, &jpeg(6000), 10000)); // 60% present
        let report = scan(&pdf);

        assert!(!report.has_index);
        assert!(report.truncated);
        assert_eq!(report.images.len(), 2);
        assert!(report.images[0].complete);
        assert_eq!(report.images[0].data.len(), 5000);
        assert_eq!((report.images[0].width, report.images[0].height), (1786, 2527));
        assert!(!report.images[1].complete);
        assert_eq!(report.images[1].percent, Some(60));
        assert!(report.images[1].data.ends_with(&[0xFF, 0xD9]));
    }

    #[test]
    fn an_image_that_has_barely_begun_is_counted_but_not_shown() {
        let mut pdf = b"%PDF-1.7\n".to_vec();
        pdf.extend(image_object(2, &jpeg(7000), 140000)); // 5% present
        let report = scan(&pdf);
        assert!(report.images.is_empty());
        assert_eq!(report.too_partial, 1);
    }

    #[test]
    fn small_images_masks_and_other_formats_are_not_pages() {
        let mut pdf = b"%PDF-1.4\n".to_vec();
        pdf.extend_from_slice(
            b"1 0 obj\n<< /Subtype /Image /Width 32 /Height 32 /Filter /DCTDecode /Length 4 >>\nstream\n\xFF\xD8\xFF\xE0\nendstream\nendobj\n",
        );
        pdf.extend_from_slice(
            b"2 0 obj\n<< /Subtype /Image /ImageMask true /Width 900 /Height 900 /Filter /FlateDecode /Length 3 >>\nstream\nabc\nendstream\nendobj\n",
        );
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Subtype /Image /Width 900 /Height 900 /Filter [/FlateDecode] /Length 3 >>\nstream\nabc\nendstream\nendobj\n",
        );
        let report = scan(&pdf);
        assert!(report.images.is_empty());
        assert_eq!(report.unsupported, 1);
        assert!(!report.truncated);
    }

    #[test]
    fn indirect_length_falls_back_to_the_endstream_keyword() {
        let mut pdf = b"%PDF-1.4\n4 0 obj\n<< /Subtype /Image /Width 800 /Height 600 /Filter [ /DCTDecode ] /Length 5 0 R >>\nstream\r\n".to_vec();
        pdf.extend(jpeg(3000));
        pdf.extend_from_slice(b"\r\nendstream\nendobj\n");
        let report = scan(&pdf);
        assert_eq!(report.images.len(), 1);
        assert!(report.images[0].complete);
        assert_eq!(report.images[0].data.len(), 3000);
    }

    #[test]
    fn a_file_with_an_index_is_left_to_the_viewer() {
        let pdf = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\nxref\n0 1\ntrailer\n<< /Root 1 0 R >>\nstartxref\n50\n%%EOF\n";
        assert!(scan(pdf).has_index);
    }

    #[test]
    fn names_the_real_kind_of_a_file_that_is_not_a_pdf() {
        assert!(describe(b"<!DOCTYPE html><html>").contains("web page"));
        assert!(describe(b"PK\x03\x04....").contains("ZIP"));
        assert!(describe(&[0u8; 64]).contains("zeros"));
    }

    /// Prints what would be recovered from a real file:
    /// `SPARKY_SAMPLE=path cargo test sample -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn sample() {
        let path = std::env::var("SPARKY_SAMPLE").expect("set SPARKY_SAMPLE to a file path");
        match check(Path::new(&path)).unwrap() {
            Verdict::Healthy => println!("healthy"),
            Verdict::NotPdf(what) => println!("not a PDF: {what}"),
            Verdict::Damaged(r) => {
                println!(
                    "damaged: {} bytes, truncated={}, too_partial={}, unsupported={}",
                    r.file_len, r.truncated, r.too_partial, r.unsupported
                );
                for i in &r.images {
                    println!(
                        "  image {}x{} {} bytes complete={} percent={:?}",
                        i.width,
                        i.height,
                        i.data.len(),
                        i.complete,
                        i.percent
                    );
                }
            }
        }
    }
}
