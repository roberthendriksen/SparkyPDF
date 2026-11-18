//! The reader's own pages: the start screen and the reports shown for files that
//! cannot be displayed as a PDF.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use crate::pdfcheck::Report;

const STYLE: &str = r#"
:root { color-scheme: light dark; --bg:#f3f3f1; --fg:#1d1d1f; --dim:#6b6b70; --card:#fff; --line:#dcdcd8;
        --accent:#c8321e; --accent-fg:#fff; --warn-bg:#fff6dc; --warn-line:#e6c86a; }
@media (prefers-color-scheme: dark) {
  :root { --bg:#1f1f21; --fg:#ececee; --dim:#a0a0a8; --card:#2a2a2d; --line:#3a3a3e;
          --accent:#e2533f; --warn-bg:#3a3320; --warn-line:#6e5f2a; }
}
* { box-sizing: border-box; }
html, body { margin:0; height:100%; }
body { background:var(--bg); color:var(--fg); font:15px/1.5 "Segoe UI Variable Text","Segoe UI",system-ui,sans-serif; }
h1 { font-size:26px; font-weight:600; margin:0 0 6px; }
h2 { font-size:12px; font-weight:600; letter-spacing:.06em; text-transform:uppercase; color:var(--dim); margin:0 0 8px; }
p { margin:0 0 10px; }
button { font:inherit; color:inherit; cursor:pointer; border-radius:6px; border:1px solid var(--line);
         background:var(--card); padding:7px 16px; }
button:hover { border-color:var(--dim); }
button:focus-visible { outline:2px solid var(--accent); outline-offset:2px; }
button.primary { background:var(--accent); border-color:var(--accent); color:var(--accent-fg); font-weight:600; padding:10px 26px; }
button.link { border:0; background:none; padding:0; color:var(--dim); text-decoration:underline; }

.home main { max-width:560px; margin:0 auto; padding:14vh 24px 40px; text-align:center; }
.home .hint { color:var(--dim); margin:12px 0 44px; }
.home ul { list-style:none; margin:0 0 40px; padding:0; text-align:left; background:var(--card);
           border:1px solid var(--line); border-radius:8px; overflow:hidden; }
.home li + li { border-top:1px solid var(--line); }
.home li button { display:block; width:100%; text-align:left; border:0; border-radius:0; background:none; padding:9px 14px; }
.home li button:hover { background:var(--bg); }
.home li b { display:block; font-weight:600; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.home li span { display:block; font-size:12.5px; color:var(--dim); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.home h2 { text-align:left; }

.note { max-width:860px; margin:24px auto 8px; padding:20px 24px; background:var(--warn-bg);
        border:1px solid var(--warn-line); border-radius:10px; }
.note h1 { font-size:21px; }
.note .name { word-break:break-all; }
.note .bar { display:flex; flex-wrap:wrap; gap:10px; margin-top:16px; }
.sheets { padding:8px 16px 40px; }
figure { margin:20px auto; max-width:min(1000px, 100%); }
figure img { display:block; width:100%; height:auto; background:#fff; box-shadow:0 1px 4px rgba(0,0,0,.25), 0 6px 22px rgba(0,0,0,.14); }
figcaption { font-size:13px; color:var(--dim); text-align:center; padding-top:8px; }

@page { margin:0; }
@page wide { size:landscape; margin:0; }
@media print {
  body { background:#fff; }
  .note, figcaption { display:none; }
  .sheets { padding:0; }
  figure { margin:0; max-width:none; break-after:page; }
  figure.wide { page:wide; }
  figure img { box-shadow:none; max-height:100vh; object-fit:contain; object-position:top; }
}
"#;

const SCRIPT: &str = "function send(m){window.ipc.postMessage(m)}";

fn document(title: &str, class: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title>\
         <style>{STYLE}</style></head><body class=\"{class}\">{body}<script>{SCRIPT}</script></body></html>",
        escape(title)
    )
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn size_text(bytes: usize) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b >> 10),
        b => format!("{b} bytes"),
    }
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The start screen: an Open button and the recently opened files.
pub fn home(recents: &[PathBuf]) -> String {
    let mut body = String::from(
        "<main><h1>Sparky PDF</h1>\
         <p class=\"hint\" style=\"margin:0 0 28px\">A small, reliable PDF reader</p>\
         <button class=\"primary\" onclick=\"send('open')\" autofocus>Open a PDF&hellip;</button>\
         <p class=\"hint\">or drop a PDF anywhere in this window &nbsp;&middot;&nbsp; Ctrl+O</p>",
    );
    if !recents.is_empty() {
        body.push_str("<h2>Recent</h2><ul>");
        for (i, path) in recents.iter().enumerate() {
            let folder = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
            let _ = write!(
                body,
                "<li><button onclick=\"send('recent:{i}')\" title=\"{full}\"><b>{name}</b><span>{folder}</span></button></li>",
                full = escape(&path.display().to_string()),
                name = escape(&name_of(path)),
                folder = escape(&folder),
            );
        }
        body.push_str("</ul>");
    }
    body.push_str(
        "<button class=\"link\" onclick=\"send('default')\">Use Sparky PDF for all PDF files&hellip;</button></main>",
    );
    document("Sparky PDF", "home", &body)
}

/// Shown for a file that is not a PDF at all.
pub fn not_pdf(path: &Path, what: &str) -> String {
    let body = format!(
        "<div class=\"note\"><h1>This file is not a PDF</h1>\
         <p><b class=\"name\">{name}</b> has a PDF name, but its contents are {what}.</p>\
         <p>No PDF reader can show it. If you downloaded it, try downloading it again &mdash; \
         when a website needs you to sign in first, it sometimes saves its sign-in page instead of the document.</p>\
         <div class=\"bar\"><button onclick=\"send('open')\">Open another file&hellip;</button>\
         <button onclick=\"send('anyway')\">Try the normal viewer anyway</button></div></div>",
        name = escape(&name_of(path)),
        what = escape(what),
    );
    document("Not a PDF", "report", &body)
}

/// Shown for a damaged PDF: explains what is wrong and displays the recovered page
/// images, which are served as `/img/0`, `/img/1`, ...
///
/// `asked` is true when the user requested the recovery from the menu for a file
/// that the health check did not reject.
pub fn damaged(path: &Path, report: &Report, asked: bool) -> String {
    let name = escape(&name_of(path));
    let complete = report.images.iter().filter(|i| i.complete).count();
    let partial = report.images.len() - complete;

    // The user asked for a recovery, but the file's structure is intact.
    let intact = asked && report.has_index;

    let mut note = String::new();
    if intact && report.images.is_empty() {
        let _ = write!(
            note,
            "<h1>No page images found</h1><p><b class=\"name\">{name}</b> does not keep its pages as \
             pictures, so there is nothing to recover this way.</p>"
        );
    } else if intact {
        let _ = write!(
            note,
            "<h1>Recovered page images</h1><p>These are the page pictures stored inside \
             <b class=\"name\">{name}</b>. Text and drawings that are not stored as pictures cannot be \
             recovered this way.</p>"
        );
    } else {
        let _ = write!(
            note,
            "<h1>This PDF is incomplete</h1>\
             <p><b class=\"name\">{name}</b> stops partway through, after {size}. A PDF keeps its list of pages \
             at the very end of the file, and that part is missing &mdash; so no PDF reader can open it normally. \
             This usually happens when a download or a copy was interrupted.</p>\
             <p><b>To get the whole document, download or copy the file again from where it came from.</b></p>",
            size = size_text(report.file_len),
        );
    }

    let mut found = Vec::new();
    if complete > 0 {
        found.push(count(complete, "complete page", "complete pages"));
    }
    if partial > 0 {
        found.push(count(partial, "partial page", "partial pages"));
    }
    if !found.is_empty() {
        let lead = if intact { "Found" } else { "Recovered from the part that is there" };
        let _ = write!(note, "<p>{lead}: {}, shown below.</p>", found.join(" and "));
    } else if !intact {
        note.push_str("<p>Nothing readable could be recovered from the part of the file that is there.</p>");
    }
    if report.too_partial > 0 {
        let _ = write!(
            note,
            "<p>{} had only just begun where the file ends &mdash; too little to show.</p>",
            if report.too_partial == 1 { "One more page".to_string() } else { format!("{} more pages", report.too_partial) }
        );
    }
    if report.unsupported > 0 {
        let _ = write!(
            note,
            "<p>{} stored in a format that cannot be recovered this way.</p>",
            if report.unsupported == 1 { "One more picture is".to_string() } else { format!("{} more pictures are", report.unsupported) }
        );
    }

    note.push_str("<div class=\"bar\">");
    if !report.images.is_empty() {
        note.push_str("<button onclick=\"send('print')\">Print or save as PDF&hellip;</button>");
    }
    let _ = write!(
        note,
        "<button onclick=\"send('anyway')\">{}</button></div>",
        if intact { "Back to the normal viewer" } else { "Try the normal viewer anyway" }
    );

    let mut body = format!("<div class=\"note\">{note}</div><div class=\"sheets\">");
    let total = report.images.len();
    for (i, image) in report.images.iter().enumerate() {
        let state = if image.complete {
            "complete".to_string()
        } else {
            match image.percent {
                Some(p) => format!("incomplete: only about {p}% of this page is in the file"),
                None => "incomplete: the rest of this page is missing from the file".to_string(),
            }
        };
        let _ = write!(
            body,
            "<figure class=\"{class}\"><img src=\"/img/{i}\" width=\"{w}\" height=\"{h}\" alt=\"Recovered page {n}\">\
             <figcaption>Recovered page {n} of {total} &mdash; {state}</figcaption></figure>",
            class = if image.width > image.height { "wide" } else { "" },
            w = image.width,
            h = image.height,
            n = i + 1,
        );
    }
    body.push_str("</div>");
    document("Damaged PDF", "report", &body)
}
