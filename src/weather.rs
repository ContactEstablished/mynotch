//! Local weather from Open-Meteo (free; no account or key). The city or ZIP the user typed in
//! Settings is looked up once (geocoding) and the result saved; then the current conditions,
//! and the sunrises and sunsets around today, are fetched every 20 minutes. All network work
//! happens on a worker thread; the UI only exchanges messages with it. The moon's phase needs
//! no network at all (`moon_age`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use std::ffi::c_void;

use windows::Data::Json::JsonObject;
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Networking::WinHttp::*;
use windows::Win32::Globalization::{GetLocaleInfoEx, GetUserDefaultGeoName, LOCALE_IMEASURE};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::{Error, HSTRING, PCWSTR, w};

/// How often the conditions are fetched again.
const REFRESH: Duration = Duration::from_secs(20 * 60);
/// How soon to try again after the service couldn't be reached.
const RETRY: Duration = Duration::from_secs(2 * 60);

/// A place found for what the user typed.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    /// "Cambridge, Massachusetts".
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// What the sky is doing, for the icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sky {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Rain,
    Snow,
    Storm,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// In the user's units: °F with US measurements, else °C.
    pub temperature: f32,
    pub sky: Sky,
    /// "Partly cloudy", "Light rain", …
    pub summary: &'static str,
    pub windy: bool,
    /// Sunrise and sunset (Unix seconds) for yesterday, today and tomorrow, in order.
    pub sun: Vec<(i64, i64)>,
    /// South of the equator, where the moon's lit side is mirrored.
    pub south: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// No place set.
    Off,
    /// Looking a place up, or fetching its weather for the first time.
    Busy,
    /// Nothing matched what was typed.
    NotFound,
    /// The service couldn't be reached (the last report, if any, stays up).
    Offline,
    Ready,
}

enum Command {
    /// Look up what the user typed, then fetch its weather.
    Lookup(String),
    /// Fetch the weather for a known place, or stop (`None`).
    Use(Option<Place>),
}

enum Event {
    Found(String, Place),
    NotFound,
    Report(Report),
    Failed,
}

/// Handle to the weather worker, shared by the shell (which sets the place and saves it)
/// and the clock card (which shows the report). Dropping the last handle stops the worker.
pub struct Weather {
    commands: Sender<Command>,
    events: Receiver<Event>,
    status: RefCell<Status>,
    report: RefCell<Option<Report>>,
}

impl Weather {
    /// Starts with a place already found, or a lookup still to do, or nothing.
    pub fn spawn(place: Option<Place>, lookup: Option<String>) -> Rc<Self> {
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        let status = if place.is_some() || lookup.is_some() { Status::Busy } else { Status::Off };
        if let Some(query) = lookup {
            let _ = commands.send(Command::Lookup(query));
        }
        let _ = thread::Builder::new().name("weather".into()).spawn(move || worker(command_rx, event_tx, place));
        Rc::new(Self { commands, events, status: RefCell::new(status), report: RefCell::new(None) })
    }

    /// Looks up a newly typed place; its weather follows.
    pub fn lookup(&self, query: &str) {
        self.status.replace(Status::Busy);
        self.report.replace(None);
        let _ = self.commands.send(Command::Lookup(query.to_owned()));
    }

    /// No place: no weather.
    pub fn clear(&self) {
        self.status.replace(Status::Off);
        self.report.replace(None);
        let _ = self.commands.send(Command::Use(None));
    }

    pub fn status(&self) -> Status {
        *self.status.borrow()
    }

    pub fn report(&self) -> Option<Report> {
        self.report.borrow().clone()
    }

    /// Takes in what the worker sent. Returns whether anything shown changed, and a place
    /// found for a query (for the shell to save).
    pub fn poll(&self) -> (bool, Option<(String, Place)>) {
        let (mut changed, mut found) = (false, None);
        for event in self.events.try_iter() {
            changed = true;
            match event {
                Event::Found(query, place) => found = Some((query, place)),
                Event::NotFound => {
                    self.status.replace(Status::NotFound);
                }
                Event::Report(report) => {
                    self.status.replace(Status::Ready);
                    self.report.replace(Some(report));
                }
                Event::Failed => {
                    self.status.replace(Status::Offline);
                }
            }
        }
        (changed, found)
    }
}

fn worker(commands: Receiver<Command>, events: Sender<Event>, mut place: Option<Place>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let client = Session::open();
    let imperial = uses_us_units();
    let mut lookup: Option<String> = None;
    let mut due = Instant::now();
    loop {
        // Nothing to fetch: just wait to be told something.
        let command = if place.is_none() && lookup.is_none() {
            commands.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            commands.recv_timeout(due.saturating_duration_since(Instant::now()))
        };
        match command {
            Ok(Command::Lookup(query)) => (lookup, place, due) = (Some(query), None, Instant::now()),
            Ok(Command::Use(p)) => (lookup, place, due) = (None, p, Instant::now()),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if Instant::now() < due {
            continue;
        }
        let Some(client) = &client else {
            let _ = events.send(Event::Failed);
            due = Instant::now() + RETRY;
            continue;
        };
        if let Some(query) = lookup.clone() {
            match geocode(client, &query) {
                Ok(Some(found)) => {
                    lookup = None;
                    place = Some(found.clone());
                    let _ = events.send(Event::Found(query, found));
                }
                Ok(None) => {
                    lookup = None;
                    let _ = events.send(Event::NotFound);
                    continue;
                }
                Err(_) => {
                    let _ = events.send(Event::Failed);
                    due = Instant::now() + RETRY;
                    continue;
                }
            }
        }
        if let Some(p) = &place {
            match forecast(client, p, imperial) {
                Ok(report) => {
                    let _ = events.send(Event::Report(report));
                    due = Instant::now() + REFRESH;
                }
                Err(_) => {
                    let _ = events.send(Event::Failed);
                    due = Instant::now() + RETRY;
                }
            }
        }
    }
}

/// A WinHTTP handle, closed when dropped.
struct Handle(*mut c_void);

impl Handle {
    fn checked(raw: *mut c_void) -> windows::core::Result<Self> {
        if raw.is_null() { Err(Error::from_thread()) } else { Ok(Self(raw)) }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

/// The worker's WinHTTP session. (WinHTTP rather than WinRT's `HttpClient`, which sits on
/// WinINet: that refuses Open-Meteo's certificate on the dev PC, while WinHTTP, like curl,
/// accepts it with the revocation check on.)
struct Session(Handle);

impl Session {
    fn open() -> Option<Self> {
        unsafe {
            let raw = WinHttpOpen(w!("MyNotch"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0);
            let session = Handle::checked(raw).ok()?;
            let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 10_000, 15_000);
            Some(Self(session))
        }
    }
}

/// GETs `https://{host}{path}` and parses the body as a JSON object.
fn get(session: &Session, host: &str, path: &str) -> windows::core::Result<JsonObject> {
    unsafe {
        let connect = Handle::checked(WinHttpConnect(session.0.0, &HSTRING::from(host), INTERNET_DEFAULT_HTTPS_PORT, 0))?;
        let request = Handle::checked(WinHttpOpenRequest(
            connect.0,
            w!("GET"),
            &HSTRING::from(path),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        let revocation = WINHTTP_ENABLE_SSL_REVOCATION.to_ne_bytes();
        WinHttpSetOption(Some(request.0), WINHTTP_OPTION_ENABLE_FEATURE, Some(&revocation))?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())?;
        let (mut status, mut size) = (0u32, 4u32);
        let query = WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER;
        WinHttpQueryHeaders(request.0, query, PCWSTR::null(), Some((&raw mut status).cast()), &mut size, std::ptr::null_mut())?;
        if status != 200 {
            return Err(Error::new(E_FAIL, format!("HTTP {status}")));
        }
        let (mut body, mut chunk) = (Vec::new(), [0u8; 8192]);
        loop {
            let mut read = 0;
            WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read)?;
            // Nothing more, or far more than any answer here should be.
            if read == 0 || body.len() > 1 << 20 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
        }
        JsonObject::Parse(&HSTRING::from(String::from_utf8_lossy(&body).as_ref()))
    }
}

/// The best match for `query` (a city, "City, ST", or a postal code), or `None` if nothing
/// matches. Postal codes are looked for in the user's own country first.
fn geocode(client: &Session, query: &str) -> windows::core::Result<Option<Place>> {
    let mut url = format!("/v1/search?count=1&language=en&format=json&name={}", encode(query));
    if is_postal(query)
        && let Some(country) = user_country()
    {
        url += &format!("&countryCode={country}");
    }
    let json = get(client, "geocoding-api.open-meteo.com", &url)?;
    let Ok(results) = json.GetNamedArray(&HSTRING::from("results")) else { return Ok(None) };
    if results.Size()? == 0 {
        return Ok(None);
    }
    let hit = results.GetObjectAt(0)?;
    let text = |key: &str| hit.GetNamedString(&HSTRING::from(key)).map(|s| s.to_string_lossy()).unwrap_or_default();
    let (name, region) = (text("name"), text("admin1"));
    let name = if region.is_empty() || region == name { name } else { format!("{name}, {region}") };
    let lat = hit.GetNamedNumber(&HSTRING::from("latitude"))?;
    let lon = hit.GetNamedNumber(&HSTRING::from("longitude"))?;
    Ok(Some(Place { name, lat, lon }))
}

fn forecast(client: &Session, place: &Place, imperial: bool) -> windows::core::Result<Report> {
    let units = if imperial { "&temperature_unit=fahrenheit&wind_speed_unit=mph" } else { "" };
    let url = format!(
        "/v1/forecast?latitude={:.4}&longitude={:.4}\
         &current=temperature_2m,weather_code,wind_speed_10m,wind_gusts_10m&daily=sunrise,sunset\
         &timezone=auto&timeformat=unixtime&past_days=1&forecast_days=2{units}",
        place.lat, place.lon
    );
    let json = get(client, "api.open-meteo.com", &url)?;
    let key = |k: &str| HSTRING::from(k);
    let current = json.GetNamedObject(&key("current"))?;
    let number = |k: &str| current.GetNamedNumber(&key(k));
    let (sky, summary) = sky(number("weather_code")? as u32);
    let (wind, gusts) = (number("wind_speed_10m")?, number("wind_gusts_10m").unwrap_or(0.0));
    // 20 mph steady or 30 mph gusts (32 / 48 km/h).
    let k = if imperial { 1.0 } else { 1.609 };
    let windy = wind >= 20.0 * k || gusts >= 30.0 * k;
    let daily = json.GetNamedObject(&key("daily"))?;
    let (rises, sets) = (daily.GetNamedArray(&key("sunrise"))?, daily.GetNamedArray(&key("sunset"))?);
    let sun = (0..rises.Size()?.min(sets.Size()?))
        .filter_map(|i| Some((rises.GetNumberAt(i).ok()? as i64, sets.GetNumberAt(i).ok()? as i64)))
        .collect();
    let temperature = number("temperature_2m")? as f32;
    Ok(Report { temperature, sky, summary, windy, sun, south: place.lat < 0.0 })
}

/// The sky and a word or two for a WMO weather code, as Open-Meteo reports them.
pub fn sky(code: u32) -> (Sky, &'static str) {
    use Sky::*;
    match code {
        0 => (Clear, "Clear"),
        1 => (Clear, "Mostly clear"),
        2 => (PartlyCloudy, "Partly cloudy"),
        3 => (Cloudy, "Cloudy"),
        45 | 48 => (Fog, "Fog"),
        51 | 53 | 55 => (Rain, "Drizzle"),
        56 | 57 => (Rain, "Freezing drizzle"),
        61 => (Rain, "Light rain"),
        63 => (Rain, "Rain"),
        65 => (Rain, "Heavy rain"),
        66 | 67 => (Rain, "Freezing rain"),
        71 => (Snow, "Light snow"),
        73 => (Snow, "Snow"),
        75 => (Snow, "Heavy snow"),
        77 => (Snow, "Snow grains"),
        80 | 81 => (Rain, "Showers"),
        82 => (Rain, "Heavy showers"),
        85 | 86 => (Snow, "Snow showers"),
        95 => (Storm, "Thunderstorm"),
        96 | 99 => (Storm, "Thunderstorm, hail"),
        _ => (Cloudy, "Cloudy"),
    }
}

/// Seconds since the Unix epoch, now.
pub fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Whether `now` is in daylight, and how far through the day (or night) it is, 0..1, from
/// consecutive sunrises and sunsets. `None` if they don't cover `now`.
pub fn daylight(sun: &[(i64, i64)], now: i64) -> Option<(bool, f32)> {
    let through = |from: i64, to: i64| ((now - from) as f32 / (to - from).max(1) as f32).clamp(0.0, 1.0);
    for (i, &(rise, set)) in sun.iter().enumerate() {
        if (rise..set).contains(&now) {
            return Some((true, through(rise, set)));
        }
        if let Some(&(next_rise, _)) = sun.get(i + 1)
            && (set..next_rise).contains(&now)
        {
            return Some((false, through(set, next_rise)));
        }
    }
    None
}

/// How far through its cycle the moon is at `unix` seconds: 0 = new, 0.25 = first quarter,
/// 0.5 = full, 0.75 = last quarter. From the mean synodic month, good to within a day.
pub fn moon_age(unix: i64) -> f32 {
    const SYNODIC: f64 = 29.530_588_853 * 86_400.0;
    /// A new moon: 2000-01-06 18:14 UTC.
    const NEW_MOON: f64 = 947_182_440.0;
    ((unix as f64 - NEW_MOON) / SYNODIC).rem_euclid(1.0) as f32
}

/// Whether the user's regional settings measure in US units (°F, mph).
fn uses_us_units() -> bool {
    let mut buf = [0u16; 4];
    let n = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_IMEASURE, Some(&mut buf)) };
    n > 0 && buf[0] == u16::from(b'1')
}

/// The user's country (Settings > Time & language > Region), e.g. "US".
fn user_country() -> Option<String> {
    let mut buf = [0u16; 16];
    let n = unsafe { GetUserDefaultGeoName(&mut buf) };
    let code = String::from_utf16_lossy(&buf[..(n.max(1) as usize - 1).min(buf.len())]);
    (code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic())).then_some(code)
}

/// Digits (with an optional dash or space, as in "12345-6789"): a postal code.
fn is_postal(query: &str) -> bool {
    let q = query.trim();
    q.chars().any(|c| c.is_ascii_digit()) && q.chars().all(|c| c.is_ascii_digit() || c == '-' || c == ' ')
}

/// Percent-encodes `s` for a URL query value.
fn encode(s: &str) -> String {
    s.trim()
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Sky, daylight, encode, is_postal, moon_age, sky};

    #[test]
    fn moon_phase_matches_known_new_and_full_moons() {
        // New moon 2024-04-08 18:21 UTC; full moon 2024-04-23 23:49 UTC.
        let new = moon_age(1_712_600_460);
        assert!(!(0.03..=0.97).contains(&new), "new moon: {new}");
        let full = moon_age(1_713_916_140);
        assert!((full - 0.5).abs() < 0.03, "full moon: {full}");
        // First quarter 2024-04-15 19:13 UTC.
        assert!((moon_age(1_713_208_380) - 0.25).abs() < 0.03);
    }

    #[test]
    fn daylight_finds_day_and_night() {
        let sun = [(100, 200), (300, 400), (500, 600)];
        assert_eq!(daylight(&sun, 150), Some((true, 0.5)));
        assert_eq!(daylight(&sun, 250), Some((false, 0.5)), "between a sunset and the next sunrise");
        assert_eq!(daylight(&sun, 300), Some((true, 0.0)), "sunrise starts the day");
        assert_eq!(daylight(&sun, 50), None, "before the data");
        assert_eq!(daylight(&sun, 650), None, "after the last sunset");
    }

    #[test]
    fn weather_codes() {
        assert_eq!(sky(0), (Sky::Clear, "Clear"));
        assert_eq!(sky(3).0, Sky::Cloudy);
        assert_eq!(sky(63).0, Sky::Rain);
        assert_eq!(sky(75).0, Sky::Snow);
        assert_eq!(sky(95).0, Sky::Storm);
        assert_eq!(sky(1234).0, Sky::Cloudy, "unknown codes read as cloudy");
    }

    #[test]
    fn queries() {
        assert!(is_postal("02139") && is_postal("12345-6789") && is_postal(" 10001 "));
        assert!(!is_postal("Boston") && !is_postal("SW1A 1AA") && !is_postal("-"));
        assert_eq!(encode(" Boston, MA "), "Boston%2C%20MA");
        assert_eq!(encode("Zürich"), "Z%C3%BCrich");
    }
}
