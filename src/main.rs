//! Sparky PDF: a small PDF reader.
//!
//! PDFs are rendered by the WebView2 engine that ships with Windows, so nothing
//! extra has to be installed. This program supplies the window, the menu, recent
//! files, and a recovery view for files that are damaged.

#![windows_subsystem = "windows"]

mod pages;
mod pdfcheck;

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tao::dpi::{PhysicalPosition, PhysicalSize};
use tao::event::{ElementState, Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::keyboard::KeyCode;
use tao::platform::windows::{IconExtWindows, WindowExtWindows};
use tao::window::{Fullscreen, Icon, Window, WindowBuilder};
use webview2_com::AcceleratorKeyPressedEventHandler;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
    COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN, COREWEBVIEW2_PHYSICAL_KEY_STATUS,
    COREWEBVIEW2_PRINT_DIALOG_KIND_BROWSER, ICoreWebView2_16,
};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegDeleteKeyW, RegDeleteTreeW, RegSetKeyValueW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_ESCAPE, VK_F11};
use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, Interface, PCWSTR, w};
use wry::http::{Request, Response};
use wry::{
    DragDropEvent, NewWindowResponse, WebContext, WebView, WebViewBuilder, WebViewExtWindows,
};

const APP: &str = "Sparky PDF";
/// Where the reader's own pages live (wry maps the `sparky` protocol to this origin).
const INTERNAL: &str = "http://sparky.localhost/";
const MAX_RECENT: usize = 10;
/// The Windows Settings page where the user picks this reader as the default.
const DEFAULT_APPS_PAGE: &str = "ms-settings:defaultapps?registeredAppUser=SparkyPDF";

/// Everything that can be asked of the app, from the menu, the keyboard, a page
/// button, or a dropped file.
#[derive(Debug, Clone)]
enum Cmd {
    OpenDialog,
    OpenPath(PathBuf),
    OpenRecent(usize),
    ClearRecent,
    /// Show the current file in the PDF engine even though it looks damaged.
    OpenAnyway,
    /// Look for page images inside the current file.
    Recover,
    Print,
    CloseFile,
    ToggleFullscreen,
    LeaveFullscreen,
    MakeDefault,
    About,
    External(String),
    Quit,
}

impl Cmd {
    /// Menu ids and page messages share one vocabulary.
    fn parse(text: &str) -> Option<Cmd> {
        Some(match text {
            "open" => Cmd::OpenDialog,
            "clear-recent" => Cmd::ClearRecent,
            "anyway" => Cmd::OpenAnyway,
            "recover" => Cmd::Recover,
            "print" => Cmd::Print,
            "close" => Cmd::CloseFile,
            "fullscreen" => Cmd::ToggleFullscreen,
            "default" => Cmd::MakeDefault,
            "about" => Cmd::About,
            "exit" => Cmd::Quit,
            other => Cmd::OpenRecent(other.strip_prefix("recent:")?.parse().ok()?),
        })
    }
}

/// What the reader's own pages are currently made of; read by the protocol handler.
#[derive(Default)]
struct Served {
    html: String,
    images: Vec<Vec<u8>>,
}

struct App {
    window: Window,
    webview: WebView,
    menu: Menu,
    recent_menu: Submenu,
    file_items: Vec<MenuItem>,
    served: Arc<Mutex<Served>>,
    /// The one local file the PDF engine may navigate to.
    allowed: Rc<RefCell<Option<PathBuf>>>,
    fullscreen: Rc<Cell<bool>>,
    current: Option<PathBuf>,
    recents: Vec<PathBuf>,
    data_dir: PathBuf,
    page_serial: u32,
}

fn main() {
    // For installers and scripts: add the reader to (or remove it from) Windows' list
    // of PDF apps and exit. `--register` also opens the Settings page where the user
    // picks the default; `--register-quiet` does not, and reports failure only through
    // the exit code.
    match std::env::args_os().nth(1) {
        Some(flag) if flag == "--register" => {
            match register_as_pdf_app() {
                Ok(()) => shell_open(DEFAULT_APPS_PAGE),
                Err(error) => fatal(&format!("Could not register {APP} with Windows.\n\n{error}")),
            }
            return;
        }
        Some(flag) if flag == "--register-quiet" => {
            std::process::exit(if register_as_pdf_app().is_ok() { 0 } else { 1 });
        }
        Some(flag) if flag == "--unregister" => return unregister_pdf_app(),
        _ => {}
    }
    if let Err(message) = run() {
        fatal(&format!("{APP} could not start.\n\n{message}"));
    }
}

fn fatal(message: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(APP)
        .set_description(message)
        .show();
}

fn run() -> Result<(), String> {
    let data_dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("SparkyPDF");
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("{}: {e}", data_dir.display()))?;

    let event_loop = EventLoopBuilder::<Cmd>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // A tall window, since most documents are portrait pages.
    let mut builder = WindowBuilder::new()
        .with_title(APP)
        .with_visible(false)
        .with_window_icon(Icon::from_resource(1, None).ok());
    if let Some(monitor) = event_loop.primary_monitor() {
        let screen = monitor.size();
        let height = (screen.height as f64 * 0.86) as u32;
        let width = ((height as f64 * 0.82) as u32).min((screen.width as f64 * 0.92) as u32);
        let origin = monitor.position();
        builder = builder
            .with_inner_size(PhysicalSize::new(width, height))
            .with_position(PhysicalPosition::new(
                origin.x + (screen.width.saturating_sub(width) / 2) as i32,
                origin.y + (screen.height as f64 * 0.02) as i32,
            ));
    }
    let window = builder.build(&event_loop).map_err(|e| e.to_string())?;

    let (menu, recent_menu, file_items) = build_menu(&window)?;
    {
        let proxy = proxy.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(cmd) = Cmd::parse(&event.id.0) {
                let _ = proxy.send_event(cmd);
            }
        }));
    }

    let served = Arc::new(Mutex::new(Served::default()));
    let allowed = Rc::new(RefCell::new(None::<PathBuf>));
    let fullscreen = Rc::new(Cell::new(false));

    let mut context = WebContext::new(Some(data_dir.join("WebView2")));
    let webview = {
        let served = served.clone();
        let allowed = allowed.clone();
        let (nav_proxy, new_proxy, ipc_proxy, drop_proxy) =
            (proxy.clone(), proxy.clone(), proxy.clone(), proxy.clone());

        WebViewBuilder::new_with_web_context(&mut context)
            .with_custom_protocol("sparky".into(), move |_id, request| serve(&served, &request))
            .with_navigation_handler(move |url| may_navigate(&url, &allowed, &nav_proxy))
            .with_new_window_req_handler(move |url, _features| {
                if is_web_link(&url) {
                    let _ = new_proxy.send_event(Cmd::External(url));
                }
                NewWindowResponse::Deny
            })
            .with_ipc_handler(move |message: Request<String>| {
                // Only the reader's own pages may give commands.
                if message.uri().to_string().starts_with(INTERNAL)
                    && let Some(cmd) = Cmd::parse(message.body())
                {
                    let _ = ipc_proxy.send_event(cmd);
                }
            })
            .with_drag_drop_handler(move |event| {
                if let DragDropEvent::Drop { paths, .. } = event
                    && let Some(path) = paths.into_iter().next()
                {
                    let _ = drop_proxy.send_event(Cmd::OpenPath(path));
                }
                true
            })
            .build(&window)
            .map_err(|e| {
                format!("The WebView2 component of Windows could not be started.\n{e}")
            })?
    };
    watch_shortcuts(&webview, proxy.clone(), fullscreen.clone());

    // Debug builds only: run one command a few seconds after start, so features can
    // be exercised without sending keystrokes to the desktop.
    #[cfg(debug_assertions)]
    if let Some(cmd) = std::env::var("SPARKYPDF_TEST_CMD").ok().as_deref().and_then(Cmd::parse) {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let _ = proxy.send_event(cmd);
        });
    }

    let mut app = App {
        window,
        webview,
        menu,
        recent_menu,
        file_items,
        served,
        allowed,
        fullscreen,
        current: None,
        recents: load_recents(&data_dir),
        data_dir,
        page_serial: 0,
    };
    app.refresh_recent_menu();
    match std::env::args_os().nth(1) {
        Some(path) => app.open(PathBuf::from(path), false),
        None => app.show_home(),
    }
    app.window.set_visible(true);
    app.window.set_focus();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        // The web context must live as long as the webview.
        let _ = &context;
        match event {
            Event::UserEvent(cmd) => app.handle(cmd, control_flow),
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                // Shortcuts while the window frame, not the page, has the keyboard.
                WindowEvent::KeyboardInput { event: key, .. }
                    if key.state == ElementState::Pressed && !key.repeat =>
                {
                    let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
                    let cmd = match key.physical_key {
                        KeyCode::KeyO if ctrl => Some(Cmd::OpenDialog),
                        KeyCode::KeyW if ctrl => Some(Cmd::CloseFile),
                        KeyCode::KeyP if ctrl => Some(Cmd::Print),
                        KeyCode::F11 => Some(Cmd::ToggleFullscreen),
                        KeyCode::Escape => Some(Cmd::LeaveFullscreen),
                        _ => None,
                    };
                    if let Some(cmd) = cmd {
                        app.handle(cmd, control_flow);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    })
}

impl App {
    fn handle(&mut self, cmd: Cmd, control_flow: &mut ControlFlow) {
        match cmd {
            Cmd::OpenDialog => {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Open PDF")
                    .add_filter("PDF documents", &["pdf"])
                    .add_filter("All files", &["*"])
                    .set_parent(&self.window);
                if let Some(folder) = self.current.as_deref().and_then(Path::parent) {
                    dialog = dialog.set_directory(folder);
                }
                if let Some(path) = dialog.pick_file() {
                    self.open(path, false);
                }
            }
            Cmd::OpenPath(path) => self.open(path, false),
            Cmd::OpenRecent(index) => {
                if let Some(path) = self.recents.get(index).cloned() {
                    self.open(path, false);
                }
            }
            Cmd::ClearRecent => {
                self.recents.clear();
                self.recents_changed();
                if self.current.is_none() {
                    self.show_home();
                }
            }
            Cmd::OpenAnyway => {
                if let Some(path) = self.current.clone() {
                    self.open(path, true);
                }
            }
            Cmd::Recover => self.recover(),
            Cmd::Print => self.print(),
            Cmd::CloseFile => self.show_home(),
            Cmd::ToggleFullscreen => self.set_fullscreen(!self.fullscreen.get()),
            Cmd::LeaveFullscreen => self.set_fullscreen(false),
            Cmd::MakeDefault => self.make_default(),
            Cmd::About => self.about(),
            Cmd::External(url) => shell_open(&url),
            Cmd::Quit => *control_flow = ControlFlow::Exit,
        }
    }

    /// Opens a file: in the PDF engine when it looks sound (or when `force` is set),
    /// otherwise with a report of what is wrong and whatever can be recovered.
    fn open(&mut self, path: PathBuf, force: bool) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let verdict = match pdfcheck::check(&path) {
            Ok(verdict) => verdict,
            Err(error) => {
                self.recents.retain(|p| !same_path(p, &path));
                self.recents_changed();
                self.alert(&format!("Could not open this file:\n{}\n\n{error}", path.display()));
                return;
            }
        };
        match verdict {
            _ if force => self.show_pdf(&path),
            pdfcheck::Verdict::Healthy => self.show_pdf(&path),
            pdfcheck::Verdict::NotPdf(what) => {
                self.show_page(pages::not_pdf(&path, what), Vec::new())
            }
            pdfcheck::Verdict::Damaged(report) => self.show_report(&path, report, false),
        }
        self.set_current(Some(path));
    }

    fn show_pdf(&mut self, path: &Path) {
        let Ok(url) = url::Url::from_file_path(path) else {
            self.alert(&format!("This location cannot be opened:\n{}", path.display()));
            return;
        };
        *self.allowed.borrow_mut() = Some(path.to_path_buf());
        let _ = self.webview.load_url(url.as_str());
    }

    fn show_report(&mut self, path: &Path, report: pdfcheck::Report, asked: bool) {
        let html = pages::damaged(path, &report, asked);
        let images = report.images.into_iter().map(|image| image.data).collect();
        self.show_page(html, images);
    }

    /// Displays one of the reader's own pages.
    fn show_page(&mut self, html: String, images: Vec<Vec<u8>>) {
        *self.served.lock().unwrap() = Served { html, images };
        *self.allowed.borrow_mut() = None;
        self.page_serial += 1;
        let _ = self.webview.load_url(&format!("{INTERNAL}page?{}", self.page_serial));
    }

    fn show_home(&mut self) {
        self.show_page(pages::home(&self.recents), Vec::new());
        self.set_current(None);
    }

    fn recover(&mut self) {
        let Some(path) = self.current.clone() else {
            return;
        };
        match std::fs::read(&path) {
            Ok(data) => self.show_report(&path, pdfcheck::scan(&data), true),
            Err(error) => self.alert(&format!("Could not read this file:\n{}\n\n{error}", path.display())),
        }
    }

    fn set_current(&mut self, path: Option<PathBuf>) {
        match &path {
            Some(path) => {
                let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
                self.window.set_title(&format!("{name} - {APP}"));
                self.recents.retain(|p| !same_path(p, path));
                self.recents.insert(0, path.clone());
                self.recents.truncate(MAX_RECENT);
                self.recents_changed();
            }
            None => self.window.set_title(APP),
        }
        for item in &self.file_items {
            item.set_enabled(path.is_some());
        }
        self.current = path;
    }

    fn recents_changed(&mut self) {
        save_recents(&self.data_dir, &self.recents);
        self.refresh_recent_menu();
    }

    fn refresh_recent_menu(&mut self) {
        while self.recent_menu.remove_at(0).is_some() {}
        for (i, path) in self.recents.iter().enumerate() {
            let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
            let label = format!("&{}  {}", (i + 1) % 10, name.replace('&', "&&"));
            let _ = self
                .recent_menu
                .append(&MenuItem::with_id(format!("recent:{i}"), label, true, None));
        }
        if self.recents.is_empty() {
            let _ = self.recent_menu.append(&MenuItem::new("(none yet)", false, None));
        } else {
            let _ = self.recent_menu.append(&PredefinedMenuItem::separator());
            let _ = self
                .recent_menu
                .append(&MenuItem::with_id("clear-recent", "&Clear This List", true, None));
        }
    }

    fn print(&self) {
        if self.current.is_none() {
            return;
        }
        let shown = self
            .webview
            .webview()
            .cast::<ICoreWebView2_16>()
            .and_then(|view| unsafe { view.ShowPrintUI(COREWEBVIEW2_PRINT_DIALOG_KIND_BROWSER) });
        if shown.is_err() {
            let _ = self.webview.print();
        }
    }

    fn set_fullscreen(&mut self, on: bool) {
        if self.fullscreen.replace(on) == on {
            return;
        }
        let hwnd = self.window.hwnd();
        unsafe {
            let _ = if on { self.menu.hide_for_hwnd(hwnd) } else { self.menu.show_for_hwnd(hwnd) };
        }
        self.window.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
    }

    /// Adds the reader to Windows' list of PDF apps, then opens the Settings page
    /// where the user confirms the choice. Windows reserves that last step for the user.
    fn make_default(&self) {
        let answer = rfd::MessageDialog::new()
            .set_parent(&self.window)
            .set_title(APP)
            .set_description(format!(
                "{APP} will add itself to Windows' list of PDF apps and then open Windows Settings.\n\n\
                 There, type .pdf in the search box and pick {APP}. Windows only lets you make that \
                 final choice yourself.\n\nNote: this registers the program where it is now. \
                 If you move the .exe later, do this once more."
            ))
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show();
        if !matches!(answer, rfd::MessageDialogResult::Ok) {
            return;
        }
        match register_as_pdf_app() {
            Ok(()) => shell_open(DEFAULT_APPS_PAGE),
            Err(error) => self.alert(&format!("Could not register {APP} with Windows.\n\n{error}")),
        }
    }

    fn about(&self) {
        let engine = wry::webview_version().unwrap_or_else(|_| "unknown".into());
        rfd::MessageDialog::new()
            .set_parent(&self.window)
            .set_title(format!("About {APP}"))
            .set_description(format!(
                "{APP} {}\nA small, reliable PDF reader.\n\n\
                 Pages are drawn by the WebView2 engine that is part of Windows (version {engine}), \
                 which Windows keeps up to date.\n\n\
                 Ctrl+O  open        Ctrl+F  find        Ctrl+P  print\n\
                 Ctrl+W  close file  F11  full screen",
                env!("CARGO_PKG_VERSION")
            ))
            .show();
    }

    fn alert(&self, message: &str) {
        rfd::MessageDialog::new()
            .set_parent(&self.window)
            .set_level(rfd::MessageLevel::Warning)
            .set_title(APP)
            .set_description(message)
            .show();
    }
}

/// Builds the menu bar. Returns the bar, the "Open Recent" submenu, and the items
/// that only make sense while a file is open.
fn build_menu(window: &Window) -> Result<(Menu, Submenu, Vec<MenuItem>), String> {
    let key = |mods: Modifiers, code: Code| Some(Accelerator::new(mods, code));
    let ctrl = Modifiers::CONTROL;

    let recent = Submenu::new("Open &Recent", true);
    let recover = MenuItem::with_id("recover", "Recover Pages from &Damaged PDF", false, None);
    let print = MenuItem::with_id("print", "&Print\u{2026}", false, key(ctrl, Code::KeyP));
    let close = MenuItem::with_id("close", "&Close File", false, key(ctrl, Code::KeyW));

    let file = Submenu::new("&File", true);
    let view = Submenu::new("&View", true);
    let help = Submenu::new("&Help", true);
    let menu = Menu::new();
    (|| -> muda::Result<()> {
        file.append_items(&[
            &MenuItem::with_id("open", "&Open\u{2026}", true, key(ctrl, Code::KeyO)),
            &recent,
            &PredefinedMenuItem::separator(),
            &print,
            &recover,
            &close,
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("default", "&Use Sparky PDF for All PDF Files\u{2026}", true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("exit", "E&xit", true, None),
        ])?;
        view.append(&MenuItem::with_id("fullscreen", "&Full Screen", true, key(Modifiers::empty(), Code::F11)))?;
        help.append(&MenuItem::with_id("about", "&About Sparky PDF", true, None))?;
        menu.append_items(&[&file, &view, &help])?;
        unsafe { menu.init_for_hwnd(window.hwnd()) }
    })()
    .map_err(|e| e.to_string())?;
    Ok((menu, recent, vec![recover, print, close]))
}

/// Answers requests for the reader's own pages and the images shown on them.
fn serve(served: &Mutex<Served>, request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let served = served.lock().unwrap();
    let path = request.uri().path();
    let (status, kind, body) = match path.strip_prefix("/img/") {
        None => (200, "text/html; charset=utf-8", served.html.clone().into_bytes()),
        Some(index) => match index.parse::<usize>().ok().and_then(|i| served.images.get(i)) {
            Some(image) => (200, "image/jpeg", image.clone()),
            None => (404, "text/plain", Vec::new()),
        },
    };
    Response::builder()
        .status(status)
        .header("Content-Type", kind)
        .header("Cache-Control", "no-store")
        .header(
            "Content-Security-Policy",
            "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'",
        )
        .body(Cow::Owned(body))
        .unwrap()
}

/// Decides whether the page may go to `url`. The reader only ever shows its own
/// pages and the one PDF it was asked to open: web links go to the default browser,
/// a dropped or linked PDF is opened through the normal path, anything else is refused.
fn may_navigate(url: &str, allowed: &RefCell<Option<PathBuf>>, proxy: &EventLoopProxy<Cmd>) -> bool {
    let cmd = match classify(url, allowed.borrow().as_deref()) {
        Nav::Allow => return true,
        Nav::Browser => Cmd::External(url.to_string()),
        Nav::Open(path) => Cmd::OpenPath(path),
        Nav::Refuse => return false,
    };
    let _ = proxy.send_event(cmd);
    false
}

#[derive(Debug, PartialEq)]
enum Nav {
    /// Load it in the window.
    Allow,
    /// Hand it to the default browser or mail program.
    Browser,
    /// Another PDF: open it through the normal checks.
    Open(PathBuf),
    Refuse,
}

fn classify(url: &str, allowed: Option<&Path>) -> Nav {
    if url.starts_with(INTERNAL) || url == "about:blank" {
        return Nav::Allow;
    }
    if is_web_link(url) {
        return Nav::Browser;
    }
    let Some(path) = url::Url::parse(url)
        .ok()
        .filter(|u| u.scheme() == "file")
        .and_then(|u| u.to_file_path().ok())
    else {
        return Nav::Refuse;
    };
    if allowed.is_some_and(|ok| same_path(ok, &path)) {
        Nav::Allow
    } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
        Nav::Open(path)
    } else {
        Nav::Refuse
    }
}

fn is_web_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    !lower.starts_with(INTERNAL)
        && ["http://", "https://", "mailto:"].iter().any(|scheme| lower.starts_with(scheme))
}

fn same_path(a: &Path, b: &Path) -> bool {
    let fold = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    fold(a) == fold(b)
}

/// Catches the reader's shortcuts while the keyboard is inside the page. Everything
/// else (Ctrl+F, Ctrl+P, zoom, arrows) is left to the PDF engine.
fn watch_shortcuts(webview: &WebView, proxy: EventLoopProxy<Cmd>, fullscreen: Rc<Cell<bool>>) {
    let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else {
            return Ok(());
        };
        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
        let mut status = COREWEBVIEW2_PHYSICAL_KEY_STATUS::default();
        let mut key = 0u32;
        unsafe {
            args.KeyEventKind(&mut kind)?;
            args.PhysicalKeyStatus(&mut status)?;
            args.VirtualKey(&mut key)?;
        }
        let pressed = kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
            || kind == COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN;
        if !pressed || status.WasKeyDown.as_bool() {
            return Ok(());
        }
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let cmd = match key {
            0x4F if ctrl => Cmd::OpenDialog, // O
            0x57 if ctrl => Cmd::CloseFile,  // W
            k if k == VK_F11.0 as u32 => Cmd::ToggleFullscreen,
            k if k == VK_ESCAPE.0 as u32 && fullscreen.get() => Cmd::LeaveFullscreen,
            _ => return Ok(()),
        };
        unsafe { args.SetHandled(true)? };
        let _ = proxy.send_event(cmd);
        Ok(())
    }));
    let mut token = 0;
    unsafe {
        let _ = webview.controller().add_AcceleratorKeyPressed(&handler, &mut token);
    }
}

/// Opens a web link or a Settings page with whatever Windows uses for it.
fn shell_open(target: &str) {
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(target),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Registers this program, for the current user only, as an app that can open PDFs.
/// This makes it appear under "Open with" and in Settings > Default apps; it does not
/// change which app is the default.
fn register_as_pdf_app() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_name = exe.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let exe = exe.display().to_string();
    let command = format!("\"{exe}\" \"%1\"");
    let app_key = format!("Software\\Classes\\Applications\\{exe_name}");

    let entries: [(&str, Option<&str>, &str); 12] = [
        ("Software\\Classes\\SparkyPDF.Document", None, "PDF Document"),
        ("Software\\Classes\\SparkyPDF.Document\\DefaultIcon", None, &format!("\"{exe}\",0")),
        ("Software\\Classes\\SparkyPDF.Document\\shell\\open\\command", None, &command),
        ("Software\\Classes\\.pdf\\OpenWithProgids", Some("SparkyPDF.Document"), ""),
        (&app_key, Some("FriendlyAppName"), APP),
        (&format!("{app_key}\\shell\\open\\command"), None, &command),
        (&format!("{app_key}\\SupportedTypes"), Some(".pdf"), ""),
        ("Software\\SparkyPDF\\Capabilities", Some("ApplicationName"), APP),
        ("Software\\SparkyPDF\\Capabilities", Some("ApplicationDescription"), "A small, reliable PDF reader"),
        ("Software\\SparkyPDF\\Capabilities\\FileAssociations", Some(".pdf"), "SparkyPDF.Document"),
        ("Software\\RegisteredApplications", Some("SparkyPDF"), "Software\\SparkyPDF\\Capabilities"),
        ("Software\\Classes\\SparkyPDF.Document\\Application", Some("ApplicationName"), APP),
    ];
    for (key, name, value) in entries {
        set_registry_text(key, name, value)?;
    }
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    Ok(())
}

/// Undoes `register_as_pdf_app`. If the reader was the default PDF app, Windows falls
/// back to asking which app to use.
fn unregister_pdf_app() {
    let exe_name = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_name().map(|name| name.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "SparkyPDF.exe".into());
    delete_registry_key("Software\\Classes\\SparkyPDF.Document");
    delete_registry_key(&format!("Software\\Classes\\Applications\\{exe_name}"));
    delete_registry_key("Software\\SparkyPDF");
    unsafe {
        let _ = RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Classes\\.pdf\\OpenWithProgids"),
            w!("SparkyPDF.Document"),
        );
        let _ = RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            w!("Software\\RegisteredApplications"),
            w!("SparkyPDF"),
        );
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
}

/// Removes a key under HKEY_CURRENT_USER together with everything below it.
fn delete_registry_key(key: &str) {
    let key = HSTRING::from(key);
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &key);
        let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &key);
    }
}

/// Writes one text value under HKEY_CURRENT_USER, creating the key if needed.
/// `name` of `None` sets the key's default value.
fn set_registry_text(key: &str, name: Option<&str>, value: &str) -> Result<(), String> {
    let data: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let name = name.map(HSTRING::from).unwrap_or_default();
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &name,
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    };
    if status.is_err() {
        return Err(format!("Registry key {key}: error {}", status.0));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_text_round_trips() {
        let key = "Software\\SparkyPDF\\SelfTest";
        let command = "\"C:\\Some Folder\\SparkyPDF.exe\" \"%1\"";
        set_registry_text(key, None, command).unwrap();
        set_registry_text(key, Some(".pdf"), "").unwrap();
        let query = std::process::Command::new("reg")
            .args(["query", &format!("HKCU\\{key}")])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&query.stdout);
        assert!(text.contains(command), "{text}");
        assert!(text.contains(".pdf"), "{text}");

        delete_registry_key(key);
        let gone = std::process::Command::new("reg")
            .args(["query", &format!("HKCU\\{key}")])
            .output()
            .unwrap();
        assert!(!gone.status.success(), "the key should have been removed");
    }

    #[test]
    fn only_the_opened_file_and_own_pages_may_load() {
        let file = Path::new("C:\\Dev\\PDF\\Healthy test & sample #1.pdf");
        let url = url::Url::from_file_path(file).unwrap().to_string();
        let other = "file:///C:/Dev/PDF/Other%20one.pdf";

        assert_eq!(classify(&url, Some(file)), Nav::Allow);
        assert_eq!(classify(&url.replace("C:/Dev", "c:/dev"), Some(file)), Nav::Allow);
        assert_eq!(classify(&format!("{url}#page=2"), Some(file)), Nav::Allow);
        assert_eq!(classify(&format!("{INTERNAL}page?3"), None), Nav::Allow);

        // A PDF other than the one being shown goes back through the health check.
        assert_eq!(classify(other, Some(file)), Nav::Open(PathBuf::from("C:\\Dev\\PDF\\Other one.pdf")));
        assert_eq!(classify(&url, None), Nav::Open(file.to_path_buf()));

        assert_eq!(classify("https://example.com/", Some(file)), Nav::Browser);
        assert_eq!(classify("MAILTO:someone@example.com", Some(file)), Nav::Browser);

        assert_eq!(classify("file:///C:/Windows/notepad.exe", Some(file)), Nav::Refuse);
        assert_eq!(classify("javascript:alert(1)", Some(file)), Nav::Refuse);
        assert_eq!(classify("ms-settings:defaultapps", Some(file)), Nav::Refuse);
    }
}

fn load_recents(data_dir: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(data_dir.join("recent.txt"))
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(PathBuf::from)
        .take(MAX_RECENT)
        .collect()
}

fn save_recents(data_dir: &Path, recents: &[PathBuf]) {
    let text: String = recents.iter().map(|p| format!("{}\n", p.display())).collect();
    let _ = std::fs::write(data_dir.join("recent.txt"), text);
}
