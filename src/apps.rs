//! Installed apps — the same list as Start's "All apps", desktop and Store alike — enumerated
//! with their icons on a worker thread, and launching them.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use windows::{
    Win32::{
        Foundation::SIZE,
        Graphics::Gdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
            GetObjectW, HBITMAP, ReleaseDC,
        },
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree},
        UI::{
            Shell::{
                BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellItemImageFactory,
                KF_FLAG_DEFAULT, SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
                SIIGBF_ICONONLY, ShellExecuteW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
    core::{HSTRING, Interface, w},
};

use crate::gfx::Image;
use crate::imaging;

#[derive(Clone, Debug, PartialEq)]
pub struct AppEntry {
    /// Shell parsing name inside `shell:AppsFolder`: an AppUserModelID or a known-folder path.
    pub id: String,
    pub name: String,
}

pub enum Loaded {
    /// Every app, sorted by name. Sent first.
    List(Vec<AppEntry>),
    Icon(String, Arc<Image>),
}

/// Enumerates apps on a worker thread, then loads `icon_px`-pixel icons, those in
/// `first` before the rest.
pub fn spawn(icon_px: u32, first: Vec<String>) -> Receiver<Loaded> {
    let (tx, rx) = mpsc::channel();
    let _ = thread::Builder::new().name("apps".into()).spawn(move || worker(&tx, icon_px, &first));
    rx
}

fn worker(tx: &Sender<Loaded>, icon_px: u32, first: &[String]) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let mut apps = enumerate();
    apps.sort_by_key(|(a, _)| a.name.to_lowercase());
    if tx.send(Loaded::List(apps.iter().map(|(a, _)| a.clone()).collect())).is_err() {
        return;
    }
    apps.sort_by_key(|(a, _)| !first.contains(&a.id));
    for (app, item) in apps {
        if let Some(img) = icon(&item, icon_px)
            && tx.send(Loaded::Icon(app.id, Arc::new(img))).is_err()
        {
            return;
        }
    }
}

fn enumerate() -> Vec<(AppEntry, IShellItem)> {
    let mut apps = Vec::new();
    let mut seen = HashSet::new();
    unsafe {
        let Ok(folder) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) else {
            return apps;
        };
        let Ok(items) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return apps };
        loop {
            let mut batch: [Option<IShellItem>; 1] = [None];
            let mut fetched = 0;
            if items.Next(&mut batch, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = batch[0].take() else { break };
            let (Some(id), Some(name)) = (display_name(&item, SIGDN_PARENTRELATIVEPARSING), display_name(&item, SIGDN_NORMALDISPLAY))
            else {
                continue;
            };
            if is_launchable(&id, &name) && seen.insert(id.clone()) {
                apps.push((AppEntry { id, name }, item));
            }
        }
    }
    apps
}

/// Start lists uninstallers, readmes and web links too; they make poor shortcuts.
fn is_launchable(id: &str, name: &str) -> bool {
    const DOCS: [&str; 8] = [".txt", ".chm", ".pdf", ".htm", ".html", ".url", ".rtf", ".md"];
    let id = id.to_ascii_lowercase();
    !name.to_ascii_lowercase().starts_with("uninstall") && !DOCS.iter().any(|ext| id.ends_with(ext))
}

unsafe fn display_name(item: &IShellItem, kind: SIGDN) -> Option<String> {
    unsafe {
        let p = item.GetDisplayName(kind).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s.filter(|s| !s.is_empty())
    }
}

fn icon(item: &IShellItem, px: u32) -> Option<Image> {
    unsafe {
        let factory: IShellItemImageFactory = item.cast().ok()?;
        let size = px as i32;
        let bitmap = factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY).ok()?;
        let img = bitmap_image(bitmap, px);
        let _ = DeleteObject(bitmap.into());
        img
    }
}

/// Reads a 32bpp bitmap's pixels. Shell icon bitmaps come with either straight or
/// premultiplied alpha; any colour channel above its alpha means straight.
unsafe fn bitmap_image(bitmap: HBITMAP, max_side: u32) -> Option<Image> {
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(bitmap.into(), size_of::<BITMAP>() as i32, Some((&raw mut bm).cast())) == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth.unsigned_abs(), bm.bmHeight.unsigned_abs());
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        let dc = GetDC(None);
        let lines = GetDIBits(dc, bitmap, 0, h, Some(pixels.as_mut_ptr().cast()), &mut bmi, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        if lines == 0 {
            return None;
        }
        let straight = pixels.as_chunks::<4>().0.iter().any(|p| p[0] > p[3] || p[1] > p[3] || p[2] > p[3]);
        imaging::from_bgra(w, h, w as usize * 4, &pixels, straight, max_side)
    }
}

/// Starts the app. Runs on a short-lived thread, since the shell may need COM.
pub fn launch(id: &str) {
    let target = HSTRING::from(format!("shell:AppsFolder\\{id}"));
    let _ = thread::Builder::new().name("launch".into()).spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        ShellExecuteW(None, w!("open"), &target, None, None, SW_SHOWNORMAL);
    });
}

/// Apps worth pinning out of the box, by display name, in tile order.
const SUGGESTED: [&str; 16] = [
    "File Explorer",
    "Terminal",
    "Visual Studio Code",
    "Cursor",
    "ChatGPT",
    "Claude",
    "Brave",
    "Google Chrome",
    "Firefox",
    "Microsoft Edge",
    "Spotify",
    "Discord",
    "Slack",
    "Notepad",
    "Calculator",
    "Settings",
];

/// First-run pins: the suggested apps that are installed, up to `max`.
pub fn suggested(apps: &[AppEntry], max: usize) -> Vec<String> {
    SUGGESTED
        .iter()
        .filter_map(|want| apps.iter().find(|a| a.name.eq_ignore_ascii_case(want)))
        .map(|a| a.id.clone())
        .take(max)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{AppEntry, is_launchable, suggested};

    fn app(id: &str, name: &str) -> AppEntry {
        AppEntry { id: id.into(), name: name.into() }
    }

    #[test]
    fn filters_non_apps() {
        assert!(is_launchable("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App", "Terminal"));
        assert!(!is_launchable("{6D809377-6AF0-444B-8957-A3773F02200E}\\Foo\\uninst.exe", "Uninstall Foo"));
        assert!(!is_launchable("C:\\Program Files\\Foo\\README.TXT", "Readme"));
        assert!(!is_launchable("https://example.com/help.url", "Help"));
    }

    #[test]
    fn suggests_installed_apps_in_order() {
        let apps = [app("calc", "Calculator"), app("code", "Visual Studio Code"), app("x", "Paint"), app("ex", "file explorer")];
        assert_eq!(suggested(&apps, 8), ["ex", "code", "calc"]);
        assert_eq!(suggested(&apps, 2), ["ex", "code"]);
    }
}
