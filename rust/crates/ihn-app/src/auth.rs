//! Auth (ARCHITECTURE §2.3, §3.7): the Google token lifecycle of 5.18.0 — web sign-in through Google Identity
//! Services, the Android PKCE flow with refresh tokens, scheduled refresh, the reconnect state, sign-out and the
//! Drive profile. Storage keys and formats are 5.18.0's, so a signed-in 5.18.0 user stays signed in.
//!
//! [`ensure_token`] is 5.18.0's `ensureDriveToken`: it answers at once when it can, otherwise the waiter's module
//! receives `Event::Io { req: waiter, status: 0, data: token }` or `status: -1, data: error message`.

use crate::abi::{chan, listen, mode, tag, tag_mod, Event, Node};
use crate::dom::Prop;
use crate::drive::client;
use crate::shell::json::Json;
use crate::shell::{self, fx, sig, Status, View};
use crate::testapi::json_str;
use crate::{App, Host, LsOp, Mod, Ui};
use ihn_core::codec::b64_encode;

// v5: app-v5.js § 4.2 constants
pub const GOOGLE_CLIENT_ID: &str = "435784295430-cmug30o42f1vu4ijgor9sjb0ro4oo37o.apps.googleusercontent.com";
pub const ANDROID_OAUTH_CLIENT_ID: &str = "435784295430-9ta7t92s33r25bamvrnbmgt65iihqpfo.apps.googleusercontent.com";
pub const ANDROID_PKCE_REDIRECT_URI: &str = "com.local.inhousenotes:/oauth2redirect";
pub const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const TOKEN_KEY: &str = "drive-token-v1";
const REMEMBER_KEY: &str = "drive-remember-v1";
const REFRESH_TOKEN_KEY: &str = "drive-refresh-token-v1";
const PKCE_VERIFIER_KEY: &str = "drive-pkce-verifier-v1";
const HINT_KEY: &str = "drive-hint-email";
const PROFILE_KEY: &str = "drive-profile-v1";
const CONSENT_KEY: &str = "drive-consent-granted";

/// The signed-in Google account as the profile menus show it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub name: String,
    pub email: String,
    pub photo: String,
    /// The access token it was loaded with (empty for a profile restored from storage).
    token: String,
    /// Restored from `drive-profile-v1`, not yet confirmed by Drive.
    fallback: bool,
}

/// The answer of [`ensure_token`].
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Ready(String),
    /// The waiter is called back.
    Pending,
    Failed(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// `tokenClient.requestAccessToken` (web).
    Gis,
    /// Refresh-token POST (Android), through `requestDriveToken`.
    Refresh,
    /// The same POST made directly by `initDriveAuth`: a failure is only logged.
    BootRefresh,
}

struct Request {
    req: u32,
    interactive: bool,
    generation: u32,
    kind: Kind,
}

#[derive(Default)]
pub struct State {
    token: Option<String>,
    expiry: f64,
    signed_out: bool,
    needs_reconnect: bool,
    /// Bumped by sign-out: answers of older token requests are dropped.
    generation: u32,
    request: Option<Request>,
    waiters: Vec<(u32, bool)>,
    gis_ready: bool,
    gis_init: u32,
    retry_started: f64,
    retry_armed: bool,
    script_listening: bool,
    listing_started: bool,
    profile: Option<Profile>,
    profile_req: u32,
    local_library: bool,
    pkce_digest: u32,
    pkce_open: u32,
    pkce_url: String,
    exchange: u32,
}

const fn id(n: u32) -> u32 { tag(Mod::Auth, n) }
const T_RETRY: u32 = id(1);
const T_REFRESH: u32 = id(2);
const T_TOKEN_TIMEOUT: u32 = id(3);
const L_GIS_SCRIPT: u32 = id(4);
/// Waiters of this module's own `ensure_token` calls.
const W_SIGN_IN: u32 = id(10);
const W_BOOT: u32 = id(11);
const W_IGNORE: u32 = id(12);

// ---- accessors --------------------------------------------------------------------------------------------------

/// The access token in memory (it may be about to expire; requests go through [`ensure_token`]).
pub fn token(app: &App) -> Option<&str> { app.st.auth.token.as_deref() }
pub fn signed_out(app: &App) -> bool { app.st.auth.signed_out }
/// Drive needs a new sign-in and the user has not signed out (5.18.0 `driveAuthNeedsReconnect && !driveSignedOut`).
pub fn reconnecting(app: &App) -> bool { app.st.auth.needs_reconnect && !app.st.auth.signed_out }
pub fn profile(app: &App) -> Option<&Profile> { app.st.auth.profile.as_ref() }
/// 5.18.0 `InhouseOfflineLibrary.ready()`: this device holds a local library for a known account.
pub fn local_library(app: &App) -> bool { app.st.auth.local_library }
/// The store reports whether a local library is available; the auth UI follows.
pub fn set_local_library(app: &mut App, host: &mut dyn Host, ready: bool) {
    if std::mem::replace(&mut app.st.auth.local_library, ready) != ready {
        update_ui(app, host);
    }
}
/// The account e-mail remembered across sessions (`drive-hint-email`), lower-cased.
pub fn known_account(host: &mut dyn Host) -> String {
    host.ls(LsOp::Get, HINT_KEY, None).unwrap_or_default().trim().to_ascii_lowercase()
}

fn ls(host: &mut dyn Host, key: &str) -> Option<String> { host.ls(LsOp::Get, key, None).filter(|v| !v.is_empty()) }

// v5: app-v5.js:hasGoogleScope
fn has_scope(scopes: &str, required: &str) -> bool { scopes.split_whitespace().any(|s| s == required) }

/// A valid stored token, a refresh token, or the boot script's signed-in verdict.
// v5: app-v5.js:showFastInitialView hasStoredSession
pub fn has_stored_session(app: &App, host: &mut dyn Host) -> bool {
    let now = host.time();
    let stored = ls(host, TOKEN_KEY).and_then(|raw| Json::parse(&raw)).is_some_and(|t| {
        t.get("token").is_some_and(Json::truthy) && t.get("expiry").and_then(Json::as_f64).is_some_and(|e| e != 0.0 && now < e - 60000.0)
    });
    stored || ls(host, REFRESH_TOKEN_KEY).is_some() || app.cfg.has(fx::BOOT_SIGNED_IN)
}

// ---- token storage ----------------------------------------------------------------------------------------------

// v5: app-v5.js:saveDriveTokenToStorage
fn save_token(host: &mut dyn Host, token: &str, expiry: f64, scopes: &str) {
    let payload = format!("{{\"token\":{},\"expiry\":{expiry},\"grantedScopes\":{}}}", json_str(token), json_str(scopes));
    if host.ls(LsOp::Set, TOKEN_KEY, Some(&payload)).and(host.ls(LsOp::Set, REMEMBER_KEY, Some("1"))).is_none() {
        host.log(3, "Failed to save Drive token:");
    }
}

// v5: app-v5.js:loadDriveTokenFromStorage
fn load_token(app: &mut App, host: &mut dyn Host) -> bool {
    let Some(parsed) = ls(host, TOKEN_KEY).and_then(|raw| Json::parse(&raw)) else { return false };
    let (token, expiry) = (parsed.text("token"), parsed.get("expiry").and_then(Json::as_f64).unwrap_or(0.0));
    if token.is_empty() || expiry == 0.0 {
        return false;
    }
    // Tokens saved by older versions also carry the (no longer requested) Calendar scope; only Drive matters.
    if !has_scope(parsed.text("grantedScopes"), DRIVE_SCOPE) || host.time() >= expiry - 60000.0 {
        host.ls(LsOp::Remove, TOKEN_KEY, None);
        return false;
    }
    app.st.auth.token = Some(token.into());
    app.st.auth.expiry = expiry;
    schedule_refresh(app, host);
    true
}

// v5: app-v5.js:scheduleDriveTokenRefresh — five minutes before expiry. (At that moment `ensure_token` still
// answers from memory, exactly as in 5.18.0; the renewal happens with the first request of the last minute.)
fn schedule_refresh(app: &mut App, host: &mut dyn Host) {
    app.sched.clear(host.out(), T_REFRESH);
    let st = &app.st.auth;
    let refresh_in = st.expiry - host.time() - 300_000.0;
    if st.token.is_some() && st.expiry != 0.0 && refresh_in > 0.0 {
        app.sched.timer(host.out(), T_REFRESH, refresh_in);
    }
}

/// Forgets the access token after a 401 so the next [`ensure_token`] fetches a new one.
// v5: app-v5.js:driveFetch "Token expired server-side"
pub fn drop_token(app: &mut App) {
    app.st.auth.token = None;
    app.st.auth.expiry = 0.0;
}

// ---- boot -------------------------------------------------------------------------------------------------------

/// The shell drives auth at boot (5.18.0's `init` order); nothing happens here.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

// v5: app-v5.js:initDriveAuth
pub fn init_drive_auth(app: &mut App, host: &mut dyn Host) {
    let has_token = load_token(app, host);
    if app.cfg.has(mode::APP) {
        update_ui(app, host);
        if has_token {
            shell::signal(app, host, sig::REFRESH, "");
        } else if let Some(refresh) = ls(host, REFRESH_TOKEN_KEY) {
            app.st.auth.waiters.push((W_BOOT, false));
            post_refresh(app, host, &refresh, false, Kind::BootRefresh);
        }
        return;
    }
    if !app.st.auth.listing_started && has_token {
        app.st.auth.listing_started = true;
        update_ui(app, host);
        shell::signal(app, host, sig::REFRESH, "");
    }
    let req = app.req(Mod::Auth);
    app.st.auth.gis_init = req;
    fx::gis_init(host.out(), req, GOOGLE_CLIENT_ID, DRIVE_SCOPE);
}

// v5: app-v5.js:scheduleDriveAuthRetry — every 200 ms while the app starts, every 2 s after 30 s; the GIS script's
// load event retries at once.
fn schedule_retry(app: &mut App, host: &mut dyn Host) {
    let out = host.out();
    if !std::mem::replace(&mut app.st.auth.script_listening, true) {
        let script = out.query(Node::DOCUMENT, "script[src*=\"accounts.google.com/gsi/client\"]");
        out.listen(script, "load", L_GIS_SCRIPT, listen::ONCE);
        out.release(script);
    }
    let st = &mut app.st.auth;
    if st.retry_armed {
        return;
    }
    let now = host.time();
    if st.retry_started == 0.0 {
        st.retry_started = now;
    }
    st.retry_armed = true;
    let delay = if now - st.retry_started < 30000.0 { 200.0 } else { 2000.0 };
    app.sched.timer(host.out(), T_RETRY, delay);
}

// ---- token acquisition ------------------------------------------------------------------------------------------

/// 5.18.0 `ensureDriveToken(interactive)`. `waiter` is an id of the calling module (`app.req(Mod::X)`); it is
/// only used when the answer is [`Token::Pending`].
// v5: app-v5.js:ensureDriveToken
pub fn ensure_token(app: &mut App, host: &mut dyn Host, interactive: bool, waiter: u32) -> Token {
    let now = host.time();
    let st = &mut app.st.auth;
    if st.signed_out && !interactive {
        return Token::Failed("Signed out".into());
    }
    if interactive {
        st.signed_out = false;
        st.needs_reconnect = false;
    }
    if let Some(token) = &st.token {
        if now < st.expiry - 60000.0 {
            return Token::Ready(token.clone());
        }
    }
    // A running request is shared. An interactive caller lets a silent request finish first and, if Google
    // rejects it, is upgraded to one interactive request (see `finish`).
    if st.request.is_none() {
        if let Err(message) = request_token(app, host, interactive) {
            return Token::Failed(message);
        }
    }
    app.st.auth.waiters.push((waiter, interactive));
    Token::Pending
}

// v5: app-v5.js:requestDriveToken
fn request_token(app: &mut App, host: &mut dyn Host, interactive: bool) -> Result<(), String> {
    // Android PKCE path: silently refresh using the stored refresh token, no popup ever.
    if app.cfg.has(mode::APP) {
        if let Some(refresh) = ls(host, REFRESH_TOKEN_KEY) {
            post_refresh(app, host, &refresh, interactive, Kind::Refresh);
            return Ok(());
        }
        if interactive {
            start_pkce(app, host);
        } else {
            mark_needs_reconnect(app, host, "missing refresh token");
        }
        return Err("No refresh token; user must sign in".into());
    }
    if !app.st.auth.gis_ready {
        return Err("Drive auth not ready".into());
    }
    let req = app.req(Mod::Auth);
    app.st.auth.request = Some(Request { req, interactive, generation: app.st.auth.generation, kind: Kind::Gis });
    app.sched.timer(host.out(), T_TOKEN_TIMEOUT, if interactive { 60000.0 } else { 9000.0 });
    // A silent request passes a hint to select the account without showing the picker.
    let hint = if interactive {
        String::new()
    } else {
        app.st.auth.profile.as_ref().map(|p| p.email.clone()).filter(|e| !e.is_empty()).or_else(|| ls(host, HINT_KEY)).unwrap_or_default()
    };
    fx::gis_request(host.out(), req, interactive, DRIVE_SCOPE, &hint);
    Ok(())
}

fn post_token(host: &mut dyn Host, req: u32, body: &str) {
    // v5: fetchWithDeadline(…, 20000) reading the whole text.
    fx::fetch(host.out(), req, &fx::Fetch {
        method: "POST",
        url: TOKEN_URL,
        headers: "Content-Type: application/x-www-form-urlencoded\n",
        deadline_ms: 20000.0,
        body: fx::Src::Bytes(body.as_bytes()),
        ..Default::default()
    });
}

// v5: app-v5.js:refreshDriveAccessTokenWithRefreshToken
fn post_refresh(app: &mut App, host: &mut dyn Host, refresh: &str, interactive: bool, kind: Kind) {
    let req = app.req(Mod::Auth);
    app.st.auth.request = Some(Request { req, interactive, generation: app.st.auth.generation, kind });
    post_token(host, req, &fx::form(&[("client_id", ANDROID_OAUTH_CLIENT_ID), ("grant_type", "refresh_token"), ("refresh_token", refresh)]));
}

/// The message of a failed token POST. v5: runtime-core-v5.js:fetchWithDeadline
fn fetch_error(io: &crate::Io) -> String {
    match io.status {
        fx::status::TIMEOUT | fx::status::BODY_TIMEOUT => "Network request timed out".into(),
        _ => String::from_utf8_lossy(io.data).into_owned(),
    }
}

fn first_500(text: &str) -> String { text.chars().take(500).collect() }

// v5: app-v5.js:applyTokenResponse
fn apply_token_response(app: &mut App, host: &mut dyn Host, data: &Json) -> Result<(), String> {
    let access = data.text("access_token");
    if access.is_empty() {
        return Err("Token response missing access_token".into());
    }
    let st = &mut app.st.auth;
    st.signed_out = false;
    st.needs_reconnect = false;
    // `parseInt(data.expires_in || '3600', 10)`
    let expires_in = match data.get("expires_in") {
        Some(Json::Num(n)) if *n != 0.0 => n.trunc(),
        Some(Json::Str(s)) => Json::parse(s.trim()).and_then(|n| n.as_f64()).map_or(3600.0, f64::trunc),
        _ => 3600.0,
    };
    let scopes = match data.text("scope") {
        "" => DRIVE_SCOPE,
        s => s,
    };
    if !has_scope(scopes, DRIVE_SCOPE) {
        st.token = None;
        st.expiry = 0.0;
        host.ls(LsOp::Remove, TOKEN_KEY, None);
        return Err("Drive access was not granted by Google.".into());
    }
    st.token = Some(access.into());
    st.expiry = host.time() + expires_in.max(60.0) * 1000.0;
    save_token(host, access, st.expiry, scopes);
    let refresh = data.text("refresh_token");
    if !refresh.is_empty() {
        host.ls(LsOp::Set, REFRESH_TOKEN_KEY, Some(refresh));
    }
    host.ls(LsOp::Set, CONSENT_KEY, Some("1"));
    schedule_refresh(app, host);
    update_ui(app, host);
    Ok(())
}

/// The token response of the web client. v5: app-v5.js:requestDriveToken `driveTokenClient.callback`
fn gis_response(app: &mut App, host: &mut dyn Host, request: &Request, io: &crate::Io) -> Result<(), String> {
    if request.generation != app.st.auth.generation || app.st.auth.signed_out {
        return Err("Drive sign-in request was cancelled".into());
    }
    if io.status < 0 {
        return Err(String::from_utf8_lossy(io.data).into_owned());
    }
    let response = std::str::from_utf8(io.data).ok().and_then(Json::parse).unwrap_or(Json::Null);
    let error = response.text("error");
    if !error.is_empty() {
        if !request.interactive {
            mark_needs_reconnect(app, host, error);
        }
        // 5.18.0 rejects with the response object itself: its text form says nothing about auth.
        return Err("[object Object]".into());
    }
    let st = &mut app.st.auth;
    st.needs_reconnect = false;
    let scopes = match response.text("scope") {
        "" => DRIVE_SCOPE,
        s => s,
    };
    if !has_scope(scopes, DRIVE_SCOPE) {
        st.token = None;
        st.expiry = 0.0;
        return Err("Drive access was not granted by Google.".into());
    }
    let access = response.text("access_token").to_string();
    st.expiry = host.time() + response.get("expires_in").and_then(Json::as_f64).unwrap_or(0.0) * 1000.0;
    st.token = Some(access.clone());
    save_token(host, &access, st.expiry, scopes);
    schedule_refresh(app, host);
    update_ui(app, host);
    Ok(())
}

/// The answer of a refresh-token POST. v5: app-v5.js:refreshDriveAccessTokenWithRefreshToken
fn refresh_response(app: &mut App, host: &mut dyn Host, request: &Request, io: &crate::Io) -> Result<(), String> {
    if io.status < 0 {
        return Err(fetch_error(io));
    }
    let text = fx::Response::parse(io.data).text();
    if !(200..300).contains(&io.status) {
        if io.status == 400 || io.status == 401 {
            // Refresh token revoked or invalid — drop it so the user signs in again.
            host.ls(LsOp::Remove, REFRESH_TOKEN_KEY, None);
        }
        return Err(format!("Refresh failed: {} {}", io.status, first_500(text)));
    }
    let Some(data @ Json::Obj(_)) = Json::parse(text) else {
        return Err("Token refresh returned an invalid response".into());
    };
    if request.generation != app.st.auth.generation || app.st.auth.signed_out {
        return Err("Drive sign-in request was cancelled".into());
    }
    apply_token_response(app, host, &data)
}

/// Settles the running token request and everyone waiting for it.
fn finish(app: &mut App, host: &mut dyn Host, request: Request, result: Result<(), String>) {
    app.sched.clear(host.out(), T_TOKEN_TIMEOUT);
    let waiters = std::mem::take(&mut app.st.auth.waiters);
    let token = app.st.auth.token.clone().unwrap_or_default();
    for (waiter, interactive) in waiters {
        let outcome = match &result {
            Ok(()) => Token::Ready(token.clone()),
            // The silent request failed: upgrade the interactive callers to one interactive request.
            Err(_) if interactive && !request.interactive => ensure_token(app, host, true, waiter),
            Err(message) => Token::Failed(message.clone()),
        };
        match outcome {
            Token::Ready(token) => settle(app, host, waiter, Ok(&token)),
            Token::Failed(message) => settle(app, host, waiter, Err(&message)),
            Token::Pending => {}
        }
    }
}

fn settle(app: &mut App, host: &mut dyn Host, waiter: u32, result: Result<&str, &str>) {
    if tag_mod(waiter) != Mod::Auth as usize {
        let (status, data) = match result {
            Ok(token) => (0, token),
            Err(message) => (-1, message),
        };
        return shell::deliver(app, host, waiter, status, 0, data.as_bytes());
    }
    match (waiter, result) {
        // v5: signInDrive `.then(() => refreshDriveFiles())` / initDriveAuth
        (W_SIGN_IN | W_BOOT, Ok(_)) => {
            shell::signal(app, host, sig::REFRESH, "");
        }
        (W_SIGN_IN, Err(_)) => shell::home_status(host, "Sign-in failed. Try again."),
        (W_BOOT, Err(message)) => {
            if app.cfg.has(mode::APP) {
                host.log(2, &format!("Silent token refresh failed: {message}"));
            }
            update_ui(app, host);
        }
        _ => {}
    }
}

// ---- sign-in ----------------------------------------------------------------------------------------------------

// v5: app-v5.js:signInDrive
pub fn sign_in(app: &mut App, host: &mut dyn Host) {
    app.st.auth.signed_out = false;
    app.st.auth.needs_reconnect = false;
    if app.cfg.has(mode::APP) {
        return start_pkce(app, host);
    }
    match ensure_token(app, host, true, W_SIGN_IN) {
        Token::Ready(token) => settle(app, host, W_SIGN_IN, Ok(&token)),
        Token::Failed(message) => settle(app, host, W_SIGN_IN, Err(&message)),
        Token::Pending => {}
    }
}

// v5: app-v5.js:startAndroidPkceSignIn — the challenge is the SHA-256 of the verifier (crypto effect).
fn start_pkce(app: &mut App, host: &mut dyn Host) {
    host.ls(LsOp::Set, REMEMBER_KEY, Some("1"));
    let mut bytes = [0u8; 32];
    host.random(&mut bytes);
    let verifier = b64_encode(&bytes, true);
    host.ls(LsOp::Set, PKCE_VERIFIER_KEY, Some(&verifier));
    let req = app.req(Mod::Auth);
    app.st.auth.pkce_digest = req;
    fx::digest(host.out(), req, fx::Src::Bytes(verifier.as_bytes()));
}

fn pkce_url(host: &mut dyn Host, challenge: &str) -> String {
    let mut pairs = vec![
        ("client_id", ANDROID_OAUTH_CLIENT_ID.to_string()),
        ("redirect_uri", ANDROID_PKCE_REDIRECT_URI.into()),
        ("response_type", "code".into()),
        ("scope", DRIVE_SCOPE.into()),
        ("access_type", "offline".into()),
        ("include_granted_scopes", "true".into()),
        ("code_challenge", challenge.into()),
        ("code_challenge_method", "S256".into()),
        ("state", "inhouse_notes_pkce".into()),
    ];
    if let Some(hint) = ls(host, HINT_KEY) {
        pairs.push(("login_hint", hint));
    }
    if host.ls(LsOp::Get, CONSENT_KEY, None).as_deref() != Some("1") {
        pairs.push(("prompt", "consent".into()));
    }
    let pairs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    format!("https://accounts.google.com/o/oauth2/v2/auth?{}", fx::form(&pairs))
}

/// `window.handleInhouseNativeOAuth(payload)`: the redirect of the Android Custom Tab.
// v5: app-v5.js:handleInhouseNativeOAuth / exchangeAuthCodeForTokens
fn native_oauth(app: &mut App, host: &mut dyn Host, raw: &str) {
    match fx::query_param(raw, "code").filter(|c| !c.is_empty()) {
        Some(code) if app.cfg.has(mode::APP) => {
            let Some(verifier) = ls(host, PKCE_VERIFIER_KEY) else {
                host.log(3, "Auth code exchange failed: Missing PKCE verifier");
                return shell::home_status(host, "Sign-in failed. Try again.");
            };
            let req = app.req(Mod::Auth);
            app.st.auth.exchange = req;
            post_token(host, req, &fx::form(&[
                ("client_id", ANDROID_OAUTH_CLIENT_ID),
                ("code", &code),
                ("code_verifier", &verifier),
                ("grant_type", "authorization_code"),
                ("redirect_uri", ANDROID_PKCE_REDIRECT_URI),
            ]));
        }
        _ => {
            if let Some(error) = fx::query_param(raw, "error").filter(|e| !e.is_empty()) {
                shell::home_status(host, &format!("Sign-in failed: {error}"));
            }
        }
    }
}

fn exchange_response(app: &mut App, host: &mut dyn Host, io: &crate::Io) -> Result<(), String> {
    if io.status < 0 {
        return Err(fetch_error(io));
    }
    let text = fx::Response::parse(io.data).text();
    if !(200..300).contains(&io.status) {
        return Err(format!("Token exchange failed: {} {}", io.status, first_500(text)));
    }
    let Some(data @ Json::Obj(_)) = Json::parse(text) else {
        return Err("Token exchange returned an invalid response".into());
    };
    host.ls(LsOp::Remove, PKCE_VERIFIER_KEY, None);
    apply_token_response(app, host, &data)
}

// ---- reconnect and sign-out -------------------------------------------------------------------------------------

/// Drive needs a new sign-in: forget the token, show the reconnect state, keep local work.
// v5: app-v5.js:markDriveAuthNeedsReconnect
pub fn mark_needs_reconnect(app: &mut App, host: &mut dyn Host, reason: &str) {
    let st = &mut app.st.auth;
    if st.signed_out {
        return;
    }
    st.needs_reconnect = true;
    st.token = None;
    st.expiry = 0.0;
    app.sched.clear(host.out(), T_REFRESH);
    host.ls(LsOp::Remove, TOKEN_KEY, None);
    // Presence pings stop and a pending Drive save is deferred by their owners.
    shell::signal(app, host, sig::RECONNECT, reason);
    update_ui(app, host);
    shell::status(app, host, "Drive sign-in needed to keep syncing", Status::Phase("paused"));
}

// v5: app-v5.js:signOutDrive
pub fn sign_out(app: &mut App, host: &mut dyn Host) {
    // A module that still holds unsaved local work vetoes (5.18.0: the offline copy could not be captured).
    if shell::signal(app, host, sig::SIGNING_OUT, "") & crate::abi::ret::PREVENT_DEFAULT != 0 {
        return;
    }
    fx::boot_signed_in(host.out(), false);
    app.cfg.flags &= !fx::BOOT_SIGNED_IN;
    let st = &mut app.st.auth;
    st.signed_out = true;
    st.needs_reconnect = false;
    st.generation += 1;
    // In-flight token requests are cancelled: their answers fail the generation check.
    let waiters = std::mem::take(&mut st.waiters);
    if let Some(request) = st.request.take() {
        if request.kind != Kind::Gis {
            fx::fetch_abort(host.out(), request.req);
        }
        app.sched.clear(host.out(), T_TOKEN_TIMEOUT);
    }
    for (waiter, _) in waiters {
        settle(app, host, waiter, Err("Drive sign-in request was cancelled"));
    }
    let st = &mut app.st.auth;
    if let Some(token) = st.token.take() {
        fx::gis_revoke(host.out(), &token);
    }
    st.expiry = 0.0;
    st.profile = None;
    st.listing_started = false;
    let out = host.out();
    app.sched.clear(out, T_REFRESH);
    for list in [Ui::DRIVE_RECENTS, Ui::DRIVE_STARRED, Ui::DRIVE_SHARED, Ui::DRIVE_FOLDERS, Ui::DRIVE_FOLDER_FILES, Ui::DRIVE_BREADCRUMBS] {
        out.html(list, "");
    }
    shell::home_status(host, "Sign in to browse your Drive.");
    shell::set_pending_share(app, None);
    update_profile_ui(app, host);
    shell::close_profile_menus(app, host);
    for key in [TOKEN_KEY, REMEMBER_KEY, HINT_KEY, PROFILE_KEY, CONSENT_KEY, "google-scope-version-v1", "google-granted-scopes-v1", REFRESH_TOKEN_KEY, PKCE_VERIFIER_KEY] {
        host.ls(LsOp::Remove, key, None);
    }
    // Sessions, caches, timers, the offline library and the media cache are dropped by their owners.
    shell::signal(app, host, sig::SIGNED_OUT, "");
    update_ui(app, host);
    shell::status(app, host, "Signed out", Status::Info);
    shell::show_view(app, host, View::Welcome);
}

// ---- UI ---------------------------------------------------------------------------------------------------------

/// The auth-dependent parts of Home and the editor header.
// v5: app-v5.js:updateDriveAuthUI
pub fn update_ui(app: &mut App, host: &mut dyn Host) {
    if app.cfg.has(mode::HEADLESS) {
        return;
    }
    let st = &app.st.auth;
    let (signed_in, reconnecting, local) = (st.token.is_some(), st.needs_reconnect && !st.signed_out, st.local_library);
    let connected = signed_in || reconnecting || local || app.cfg.has(fx::BOOT_SIGNED_IN);
    let (offline, share) = (shell::offline(app), shell::pending_share(app));
    let out = host.out();
    let login = !connected || (reconnecting && !offline);
    out.style(Ui::BTN_DRIVE_LOGIN, "display", Some(if login { "inline-flex" } else { "none" }));
    out.text(Ui::BTN_DRIVE_LOGIN, if reconnecting { "Reconnect" } else { "Sign in" });
    out.class(Ui::DRIVE_PROFILE, "hidden", !connected);
    out.class(Ui::DRIVE_PROFILE_EDITOR, "hidden", !connected);
    for control in [Ui::DRIVE_SEARCH, Ui::BTN_DRIVE_NEW, Ui::BTN_DRIVE_NEW_FOLDER] {
        out.prop(control, "disabled", Prop::Bool(!signed_in && !local));
    }
    out.prop(Ui::BTN_DRIVE_BIN, "disabled", Prop::Bool(!signed_in));
    // Home enables Back when its folder stack is deeper than the root (it follows `sig::AUTH`).
    out.prop(Ui::BTN_DRIVE_BACK, "disabled", Prop::Bool(true));
    out.text(Ui::DRIVE_STATUS, match () {
        _ if local && offline => "Offline · changes stay on this device",
        _ if reconnecting => "Drive connection paused. Sign in again to continue syncing.",
        _ if signed_in => "Loading your PDFs...",
        _ if share.is_some_and(|s| s.open_failed) => "Opening public preview...",
        _ if share.is_some() => "Opening shared document...",
        _ if local => "Connecting to Drive…",
        _ => "Sign in to browse your Drive.",
    });
    if connected {
        if shell::view(app) == View::Welcome {
            shell::show_view(app, host, View::Home);
        }
    } else {
        for empty in [Ui::DRIVE_STARRED_EMPTY, Ui::DRIVE_SHARED_EMPTY, Ui::DRIVE_FOLDER_EMPTY] {
            out.class(empty, "hidden", true);
        }
        shell::close_profile_menus(app, host);
    }
    // Pending shared links, deferred saves, PDF backgrounds, exit uploads and collaboration follow this signal.
    shell::signal(app, host, sig::AUTH, "");
    if signed_in {
        load_profile(app, host);
    }
}

// v5: app-v5.js:restoreCachedDriveProfile
pub fn restore_cached_profile(app: &mut App, host: &mut dyn Host) {
    let email = host.ls(LsOp::Get, HINT_KEY, None).unwrap_or_default().to_ascii_lowercase();
    if let Some(p) = ls(host, PROFILE_KEY).and_then(|raw| Json::parse(&raw)) {
        if !p.text("email").is_empty() && p.text("email").to_ascii_lowercase() == email {
            app.st.auth.profile = Some(Profile { name: p.text("name").into(), email: p.text("email").into(), photo: p.text("photo").into(), token: String::new(), fallback: true });
        }
    }
    update_profile_ui(app, host);
}

// v5: app-v5.js:updateDriveProfileUI
fn update_profile_ui(app: &mut App, host: &mut dyn Host) {
    let p = app.st.auth.profile.as_ref();
    let photo = p.map(|p| p.photo.as_str()).filter(|s| !s.is_empty()).unwrap_or(shell::DEFAULT_AVATAR);
    let name = p.map(|p| p.name.as_str()).filter(|s| !s.is_empty()).unwrap_or("Signed in");
    let email = p.map_or("", |p| p.email.as_str());
    let out = host.out();
    for avatar in [Ui::DRIVE_PROFILE_AVATAR, Ui::DRIVE_PROFILE_AVATAR_MENU, Ui::DRIVE_PROFILE_AVATAR_EDITOR, Ui::DRIVE_PROFILE_AVATAR_MENU_EDITOR] {
        out.prop(avatar, "src", Prop::Str(photo));
    }
    out.text(Ui::DRIVE_PROFILE_NAME, name);
    out.text(Ui::DRIVE_PROFILE_EMAIL, email);
    out.text(Ui::DRIVE_PROFILE_NAME_EDITOR, name);
    out.text(Ui::DRIVE_PROFILE_EMAIL_EDITOR, email);
}

// v5: app-v5.js:loadDriveProfile
fn load_profile(app: &mut App, host: &mut dyn Host) {
    let st = &app.st.auth;
    let Some(token) = st.token.as_deref() else { return };
    if shell::offline(app) || st.profile_req != 0 {
        return;
    }
    if st.profile.as_ref().is_some_and(|p| p.token == token && !p.fallback) {
        return update_profile_ui(app, host);
    }
    app.st.auth.profile_req = client::request(app, host, Mod::Auth, client::Req::get("https://www.googleapis.com/drive/v3/about?fields=user(displayName,emailAddress,photoLink)"));
}

fn profile_response(app: &mut App, host: &mut dyn Host, io: &crate::Io) {
    if io.status < 0 {
        return host.log(3, &format!("Failed to load Drive profile: {}", String::from_utf8_lossy(io.data)));
    }
    let Some(data) = Json::parse(fx::Response::parse(io.data).text()) else { return };
    let Some(user) = data.get("user").filter(|u| u.truthy()) else { return };
    let or = |value: &str, fallback: &str| if value.is_empty() { fallback.to_string() } else { value.to_string() };
    let profile = Profile {
        name: or(user.text("displayName"), "Signed in"),
        email: user.text("emailAddress").into(),
        photo: or(user.text("photoLink"), shell::DEFAULT_AVATAR),
        token: app.st.auth.token.clone().unwrap_or_default(),
        fallback: false,
    };
    if !profile.email.is_empty() {
        host.ls(LsOp::Set, HINT_KEY, Some(&profile.email));
        let stored = format!("{{\"name\":{},\"email\":{},\"photo\":{}}}", json_str(&profile.name), json_str(&profile.email), json_str(&profile.photo));
        host.ls(LsOp::Set, PROFILE_KEY, Some(&stored));
    }
    let email = profile.email.clone();
    app.st.auth.profile = Some(profile);
    // The local library and the background authorisation switch to this account in their own modules.
    shell::signal(app, host, sig::ACCOUNT, &email);
    update_profile_ui(app, host);
}

// ---- events -----------------------------------------------------------------------------------------------------

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Timer(T_RETRY) => {
            app.st.auth.retry_armed = false;
            init_drive_auth(app, host);
        }
        Event::Dom(e) if e.listener == L_GIS_SCRIPT => {
            if std::mem::take(&mut app.st.auth.retry_armed) {
                app.sched.clear(host.out(), T_RETRY);
                init_drive_auth(app, host);
            }
        }
        // Silently refresh the token.
        Event::Timer(T_REFRESH) => {
            let _ = ensure_token(app, host, false, W_IGNORE);
        }
        Event::Timer(T_TOKEN_TIMEOUT) => {
            if let Some(request) = app.st.auth.request.take() {
                let cancelled = request.generation != app.st.auth.generation || app.st.auth.signed_out;
                let message = if cancelled { "Drive sign-in request was cancelled" } else { "Silent token refresh timed out" };
                if !cancelled && !request.interactive {
                    mark_needs_reconnect(app, host, message);
                }
                finish(app, host, request, Err(message.into()));
            }
        }
        Event::Io(io) => {
            let st = &mut app.st.auth;
            if io.req == st.gis_init {
                st.gis_init = 0;
                if io.status < 0 {
                    return {
                        schedule_retry(app, host);
                        0
                    };
                }
                st.gis_ready = true;
                update_ui(app, host);
                if !app.st.auth.listing_started && host.ls(LsOp::Get, REMEMBER_KEY, None).as_deref() == Some("1") {
                    match ensure_token(app, host, false, W_BOOT) {
                        Token::Ready(token) => settle(app, host, W_BOOT, Ok(&token)),
                        Token::Failed(message) => settle(app, host, W_BOOT, Err(&message)),
                        Token::Pending => {}
                    }
                }
            } else if st.request.as_ref().is_some_and(|r| r.req == io.req) {
                let Some(request) = st.request.take() else { return 0 };
                let result = match request.kind {
                    Kind::Gis => gis_response(app, host, &request, &io),
                    Kind::Refresh | Kind::BootRefresh => refresh_response(app, host, &request, &io),
                };
                if let (Err(message), Kind::Refresh, false) = (&result, request.kind, request.interactive) {
                    mark_needs_reconnect(app, host, message);
                }
                finish(app, host, request, result);
            } else if io.req == st.pkce_digest {
                st.pkce_digest = 0;
                let url = pkce_url(host, &b64_encode(io.data, true));
                let req = app.req(Mod::Auth);
                app.st.auth.pkce_open = req;
                fx::native_call(host.out(), req, "openAuthUrl", &[&url]);
                app.st.auth.pkce_url = url;
            } else if io.req == st.pkce_open {
                st.pkce_open = 0;
                let url = std::mem::take(&mut st.pkce_url);
                match io.status {
                    0 => shell::home_status(host, "Complete sign-in in the Google window."),
                    fx::status::FAILED => {
                        host.log(2, "PKCE sign-in: Android bridge unavailable, falling back to web flow.");
                        fx::navigate(host.out(), fx::Nav::Assign, &url);
                    }
                    _ => {
                        host.log(3, &format!("PKCE sign-in failed: {}", String::from_utf8_lossy(io.data)));
                        shell::home_status(host, "Sign-in failed. Try again.");
                    }
                }
            } else if io.req == st.exchange {
                st.exchange = 0;
                match exchange_response(app, host, &io) {
                    Ok(()) => {
                        shell::signal(app, host, sig::REFRESH, "");
                    }
                    Err(message) => {
                        host.log(3, &format!("Auth code exchange failed: {message}"));
                        shell::home_status(host, "Sign-in failed. Try again.");
                    }
                }
            } else if io.req == st.profile_req {
                st.profile_req = 0;
                profile_response(app, host, &io);
            }
        }
        Event::Msg(m) if m.chan == chan::OAUTH => native_oauth(app, host, &String::from_utf8_lossy(m.data)),
        _ => {}
    }
    0
}

pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    Some(match name {
        "state" => {
            let st = &app.st.auth;
            format!(
                "{{\"signedIn\":{},\"signedOut\":{},\"reconnecting\":{},\"expiry\":{},\"gis\":{},\"email\":{}}}",
                st.token.is_some(), st.signed_out, reconnecting(app), st.expiry, st.gis_ready,
                json_str(st.profile.as_ref().map_or("", |p| p.email.as_str()))
            )
        }
        "reconnect" => {
            mark_needs_reconnect(app, host, "test");
            "true".into()
        }
        "expire" => {
            drop_token(app);
            "true".into()
        }
        _ => return None,
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::testkit::*;

    fn stored_token(host: &mut crate::host::FakeHost, token: &str, expires_in: f64) {
        let payload = format!("{{\"token\":\"{token}\",\"expiry\":{},\"grantedScopes\":\"{DRIVE_SCOPE} https://www.googleapis.com/auth/calendar.events\"}}", host.time + expires_in);
        host.local.insert(TOKEN_KEY.into(), payload);
        host.local.insert(REMEMBER_KEY.into(), "1".into());
    }
    fn results(app: &mut App) -> Vec<(i32, String)> {
        std::mem::take(&mut app.st.shell.probe.results).into_iter().map(|r| (r.1, String::from_utf8_lossy(&r.3).into_owned())).collect()
    }
    fn gis_ready(app: &mut App, host: &mut crate::host::FakeHost) -> String {
        let text = out(host);
        io(app, host, req_of(&text, "gis_init"), 0, b"");
        text + &out(host)
    }

    #[test]
    fn web_sign_in_stores_the_token_shows_home_and_loads_the_profile() {
        let (mut app, mut host) = boot(0, |_| {});
        gis_ready(&mut app, &mut host);
        signals(&mut app);
        sign_in(&mut app, &mut host);
        let text = out(&mut host);
        let req = req_of(&text, "gis_request");
        assert_eq!(text, format!("timer_set {T_TOKEN_TIMEOUT} 60000 0\ngis_request {req} 1 \"{DRIVE_SCOPE}\" \"\"\n"));
        io(&mut app, &mut host, req, 0, TOKEN_JSON.as_bytes());
        let expiry = host.time + 3_600_000.0;
        assert_eq!(host.local[TOKEN_KEY], format!("{{\"token\":\"tok-1\",\"expiry\":{expiry},\"grantedScopes\":\"{DRIVE_SCOPE}\"}}"));
        assert_eq!(host.local[REMEMBER_KEY], "1");
        assert!(!host.local.contains_key(CONSENT_KEY), "the web flow does not record consent");
        assert_eq!((token(&app), shell::view(&app)), (Some("tok-1"), View::Home));
        let text = out(&mut host);
        assert!(text.starts_with(&format!("timer_clear {T_REFRESH}\ntimer_set {T_REFRESH} 3300000 0\n")), "{text}");
        assert!(text.contains(&format!("style {} \"display\" \"none\"", Ui::BTN_DRIVE_LOGIN.0)));
        assert!(text.contains(&format!("text {} \"Loading your PDFs...\"", Ui::DRIVE_STATUS.0)));
        assert!(text.contains(&format!("prop {} \"disabled\" false", Ui::DRIVE_SEARCH.0)));
        let about = line_of(&text, "fetch");
        assert!(about.ends_with("\"GET\" \"https://www.googleapis.com/drive/v3/about?fields=user(displayName,emailAddress,photoLink)\" \"Authorization: Bearer tok-1\\n\" 0 60000 0 -"), "{about}");
        let seen = signals(&mut app);
        assert_eq!(seen.iter().map(|s| s.0).collect::<Vec<_>>(), [sig::VIEW, sig::AUTH, sig::REFRESH]);
        // The profile answer fills both menus and is remembered for the next boot.
        io(&mut app, &mut host, req_of(&text, "fetch"), 200, &response("content-type: application/json\n", r#"{"user":{"displayName":"Ana","emailAddress":"Ana@Example.com","photoLink":"https://lh3.googleusercontent.com/a"}}"#));
        assert_eq!(host.local[HINT_KEY], "Ana@Example.com");
        assert_eq!(host.local[PROFILE_KEY], r#"{"name":"Ana","email":"Ana@Example.com","photo":"https://lh3.googleusercontent.com/a"}"#);
        let text = out(&mut host);
        assert!(text.contains(&format!("prop {} \"src\" \"https://lh3.googleusercontent.com/a\"", Ui::DRIVE_PROFILE_AVATAR_MENU_EDITOR.0)));
        assert!(text.contains(&format!("text {} \"Ana\"", Ui::DRIVE_PROFILE_NAME.0)) && text.contains(&format!("text {} \"Ana@Example.com\"", Ui::DRIVE_PROFILE_EMAIL_EDITOR.0)));
        assert_eq!(signals(&mut app), [(sig::ACCOUNT, "Ana@Example.com".to_string())]);
        assert_eq!(known_account(&mut host), "ana@example.com");
        // A second token check answers from memory until a minute before expiry.
        assert_eq!(ensure_token(&mut app, &mut host, false, W_IGNORE), Token::Ready("tok-1".into()));
        host.advance(3_600_000.0 - 60_000.0);
        assert_eq!(ensure_token(&mut app, &mut host, false, W_IGNORE), Token::Pending);
        assert!(out(&mut host).ends_with(&format!("1 0 \"{DRIVE_SCOPE}\" \"Ana@Example.com\"\n").replace("1 0 ", " 0 ")));
    }

    #[test]
    fn a_stored_token_lists_at_once_and_a_second_401_pauses_drive() {
        let (mut app, mut host) = boot(fx::BOOT_SIGNED_IN, |h| {
            stored_token(h, "old", 3_600_000.0);
            h.local.insert(HINT_KEY.into(), "ana@example.com".into());
            h.local.insert(PROFILE_KEY.into(), r#"{"name":"Ana","email":"Ana@example.com","photo":""}"#.into());
        });
        let boot_text = gis_ready(&mut app, &mut host);
        assert!(boot_text.contains(&format!("text {} \"Ana\"", Ui::DRIVE_PROFILE_NAME.0)), "the cached profile shows before Drive answers");
        assert!(boot_text.contains(&format!("timer_set {T_REFRESH} 3300000 0")) && !boot_text.contains("gis_request"));
        assert!(signals(&mut app).contains(&(sig::REFRESH, String::new())));
        assert_eq!(shell::view(&app), View::Home);

        let id = client::request(&mut app, &mut host, Mod::Shell, client::Req::get("https://www.googleapis.com/drive/v3/files/x"));
        let text = out(&mut host);
        assert!(line_of(&text, "fetch").contains("\"Authorization: Bearer old\\n\" 0 60000 0 -"));
        // 401: the token is dropped and the request retried once with a fresh one.
        io(&mut app, &mut host, req_of(&text, "fetch"), 401, &response("", "expired"));
        let text = out(&mut host);
        let gis = req_of(&text, "gis_request");
        assert!(text.contains(&format!("gis_request {gis} 0 \"{DRIVE_SCOPE}\" \"Ana@example.com\"")) && text.contains(&format!("timer_set {T_TOKEN_TIMEOUT} 9000 0")));
        io(&mut app, &mut host, gis, 0, TOKEN_JSON.as_bytes());
        let text = out(&mut host);
        assert!(line_of(&text, "fetch").contains("files/x\" \"Authorization: Bearer tok-1\\n\""), "{text}");
        assert!(results(&mut app).is_empty());
        io(&mut app, &mut host, req_of(&text, "fetch"), 401, &response("", "still expired"));
        assert_eq!(results(&mut app), [(-401, "Drive API error 401: still expired".to_string())]);
        assert!(reconnecting(&app) && token(&app).is_none() && !host.local.contains_key(TOKEN_KEY));
        let text = out(&mut host);
        assert!(text.contains(&format!("text {} \"Reconnect\"", Ui::BTN_DRIVE_LOGIN.0)) && text.contains(&format!("style {} \"display\" \"inline-flex\"", Ui::BTN_DRIVE_LOGIN.0)));
        assert!(text.contains(&format!("text {} \"Drive connection paused. Sign in again to continue syncing.\"", Ui::DRIVE_STATUS.0)));
        assert!(text.contains(&format!("class {} \"paused\" 1", Ui::SAVE_INDICATOR.0)));
        assert!(signals(&mut app).iter().any(|s| s.0 == sig::RECONNECT));
        assert_eq!(client::busy(&app), 1, "only the profile request is left");
        let _ = id;
    }

    #[test]
    fn a_silent_timeout_asks_to_reconnect_and_an_interactive_caller_is_upgraded() {
        let (mut app, mut host) = boot(fx::BOOT_SIGNED_IN, |h| {
            h.local.insert(REMEMBER_KEY.into(), "1".into());
        });
        let text = gis_ready(&mut app, &mut host);
        let silent = req_of(&text, "gis_request");
        assert!(text.contains(&format!("gis_request {silent} 0 ")));
        // The user taps "Sign in" while the silent request runs: no second request yet.
        sign_in(&mut app, &mut host);
        assert_eq!(out(&mut host), "");
        timer(&mut app, &mut host, T_TOKEN_TIMEOUT);
        let text = out(&mut host);
        let interactive = req_of(&text, "gis_request");
        assert!(interactive != silent && text.contains(&format!("gis_request {interactive} 1 ")) && text.contains(&format!("timer_set {T_TOKEN_TIMEOUT} 60000 0")));
        assert!(!reconnecting(&app), "an interactive request clears the reconnect state");
        // The late answer of the silent request is ignored.
        io(&mut app, &mut host, silent, 0, TOKEN_JSON.as_bytes());
        assert!(token(&app).is_none());
        // Google refuses: the Home status line says so.
        io(&mut app, &mut host, interactive, 0, br#"{"error":"access_denied"}"#);
        assert!(out(&mut host).ends_with(&format!("text {} \"Sign-in failed. Try again.\"\n", Ui::DRIVE_STATUS.0)));
        // A token without the Drive scope is refused.
        sign_in(&mut app, &mut host);
        let req = req_of(&out(&mut host), "gis_request");
        io(&mut app, &mut host, req, 0, br#"{"access_token":"t","expires_in":3600,"scope":"openid"}"#);
        assert!(token(&app).is_none() && !host.local.contains_key(TOKEN_KEY));
    }

    #[test]
    fn a_failed_silent_refresh_marks_reconnect() {
        let (mut app, mut host) = boot(fx::BOOT_SIGNED_IN, |h| stored_token(h, "old", 3_600_000.0));
        gis_ready(&mut app, &mut host);
        // v5 quirk kept: the refresh timer fires five minutes early, when `ensureDriveToken` still answers from
        // memory, so the token is really renewed by the first request of its last minute.
        host.advance(3_300_000.0);
        timer(&mut app, &mut host, T_REFRESH);
        assert_eq!(out(&mut host), "");
        host.advance(245_000.0);
        timer(&mut app, &mut host, T_REFRESH);
        let req = req_of(&out(&mut host), "gis_request");
        io(&mut app, &mut host, req, 0, br#"{"error":"interaction_required"}"#);
        assert!(reconnecting(&app) && !host.local.contains_key(TOKEN_KEY));
        // Without a token a Drive request fails with the auth error and no network call.
        app.st.auth.gis_ready = false;
        client::request(&mut app, &mut host, Mod::Shell, client::Req::get("https://www.googleapis.com/drive/v3/files"));
        assert_eq!(results(&mut app), [(client::err::AUTH, "Drive auth not ready".to_string())]);
    }

    #[test]
    fn sign_out_cancels_requests_clears_storage_and_returns_to_welcome() {
        let (mut app, mut host) = boot(fx::BOOT_SIGNED_IN, |h| {
            stored_token(h, "old", 30_000.0); // too close to expiry: dropped at boot
            for key in [HINT_KEY, PROFILE_KEY, CONSENT_KEY, REFRESH_TOKEN_KEY, PKCE_VERIFIER_KEY, "google-scope-version-v1", "google-granted-scopes-v1"] {
                h.local.insert(key.into(), "x".into());
            }
            h.local.insert("inhouse-theme".into(), "dark".into());
        });
        let text = gis_ready(&mut app, &mut host);
        assert!(!host.local.contains_key(TOKEN_KEY));
        let silent = req_of(&text, "gis_request");
        client::request(&mut app, &mut host, Mod::Shell, client::Req::get("https://www.googleapis.com/drive/v3/files"));
        out(&mut host);
        signals(&mut app);
        sign_out(&mut app, &mut host);
        assert_eq!(results(&mut app), [(client::err::AUTH, "Drive sign-in request was cancelled".to_string())]);
        assert_eq!(host.local.keys().map(String::as_str).collect::<Vec<_>>(), ["inhouse-theme"], "theme and app mode are kept");
        assert!(signed_out(&app) && !reconnecting(&app) && shell::view(&app) == View::Welcome);
        let text = out(&mut host);
        assert!(text.starts_with("boot_signed_in 0\n"));
        for list in [Ui::DRIVE_RECENTS, Ui::DRIVE_BREADCRUMBS] {
            assert!(text.contains(&format!("html {} \"\"", list.0)));
        }
        assert!(text.contains(&format!("text {} \"Sign in to browse your Drive.\"", Ui::DRIVE_STATUS.0)));
        assert!(text.contains(&format!("style {} \"display\" \"inline-flex\"", Ui::BTN_DRIVE_LOGIN.0)) && text.contains(&format!("text {} \"Sign in\"", Ui::BTN_DRIVE_LOGIN.0)));
        assert!(text.contains(&format!("prop {} \"title\" \"Signed out\"", Ui::SAVE_INDICATOR.0)));
        assert!(text.contains(&format!("class {} \"hidden\" 0", Ui::WELCOME_VIEW.0)));
        let seen: Vec<u8> = signals(&mut app).iter().map(|s| s.0).collect();
        // (The first AUTH is the cancelled boot request giving up.)
        assert_eq!(seen, [sig::SIGNING_OUT, sig::AUTH, sig::SIGNED_OUT, sig::AUTH, sig::STATUS, sig::VIEW]);
        // Answers of the cancelled request change nothing; silent requests are refused until the user signs in.
        io(&mut app, &mut host, silent, 0, TOKEN_JSON.as_bytes());
        assert!(token(&app).is_none());
        assert_eq!(ensure_token(&mut app, &mut host, false, W_IGNORE), Token::Failed("Signed out".into()));
        // With a token in memory Google is asked to revoke it.
        sign_in(&mut app, &mut host);
        let req = req_of(&out(&mut host), "gis_request");
        io(&mut app, &mut host, req, 0, TOKEN_JSON.as_bytes());
        out(&mut host);
        sign_out(&mut app, &mut host);
        assert!(out(&mut host).contains("gis_revoke \"tok-1\"\n"));
    }

    #[test]
    fn android_pkce_sign_in_exchange_and_refresh() {
        let (mut app, mut host) = boot(mode::APP, |h| {
            h.local.insert(HINT_KEY.into(), "ana@example.com".into());
        });
        let text = out(&mut host);
        assert!(!text.contains("gis_init"), "GIS is never initialised in the APK");
        assert!(text.contains(&format!("style {} \"display\" \"inline-flex\"", Ui::BTN_DRIVE_LOGIN.0)));
        sign_in(&mut app, &mut host);
        let verifier = host.local[PKCE_VERIFIER_KEY].clone();
        assert_eq!((verifier.len(), host.local[REMEMBER_KEY].as_str()), (43, "1"));
        let text = out(&mut host);
        assert_eq!(text, format!("digest {} {verifier:?}\n", req_of(&text, "digest")));
        io(&mut app, &mut host, req_of(&text, "digest"), 0, &[0xfb; 32]);
        let text = out(&mut host);
        let open = req_of(&text, "native_call");
        let url = "https://accounts.google.com/o/oauth2/v2/auth?client_id=435784295430-9ta7t92s33r25bamvrnbmgt65iihqpfo.apps.googleusercontent.com&redirect_uri=com.local.inhousenotes%3A%2Foauth2redirect&response_type=code&scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive&access_type=offline&include_granted_scopes=true&code_challenge=-_v7-_v7-_v7-_v7-_v7-_v7-_v7-_v7-_v7-_v7-_s&code_challenge_method=S256&state=inhouse_notes_pkce&login_hint=ana%40example.com&prompt=consent";
        assert_eq!(text, format!("native_call {open} \"openAuthUrl\" [{url:?}]\n"));
        io(&mut app, &mut host, open, 0, b"");
        assert_eq!(out(&mut host), format!("text {} \"Complete sign-in in the Google window.\"\n", Ui::DRIVE_STATUS.0));

        // Android hands the redirect back: the code is exchanged without a client secret.
        msg(&mut app, &mut host, chan::OAUTH, "state=inhouse_notes_pkce&code=4%2F0Ab_c&scope=x");
        let text = out(&mut host);
        let exchange = req_of(&text, "fetch");
        assert_eq!(
            text,
            format!("fetch {exchange} \"POST\" \"https://oauth2.googleapis.com/token\" \"Content-Type: application/x-www-form-urlencoded\\n\" 0 20000 0 \"client_id={ANDROID_OAUTH_CLIENT_ID}&code=4%2F0Ab_c&code_verifier={verifier}&grant_type=authorization_code&redirect_uri=com.local.inhousenotes%3A%2Foauth2redirect\"\n")
        );
        signals(&mut app);
        io(&mut app, &mut host, exchange, 200, &response("", &format!(r#"{{"access_token":"at-1","expires_in":"3599","refresh_token":"rt-1","scope":"{DRIVE_SCOPE}"}}"#)));
        assert_eq!((host.local[REFRESH_TOKEN_KEY].as_str(), host.local[CONSENT_KEY].as_str()), ("rt-1", "1"));
        assert!(!host.local.contains_key(PKCE_VERIFIER_KEY) && token(&app) == Some("at-1"));
        assert_eq!(app.st.auth.expiry, host.time + 3_599_000.0);
        assert_eq!(signals(&mut app).last(), Some(&(sig::REFRESH, String::new())));
        out(&mut host);

        // Later the access token is renewed with the refresh token, silently.
        host.advance(3_599_000.0);
        assert_eq!(ensure_token(&mut app, &mut host, false, tag(Mod::Shell, 77)), Token::Pending);
        let text = out(&mut host);
        let refresh = req_of(&text, "fetch");
        assert!(text.ends_with(&format!("\"client_id={ANDROID_OAUTH_CLIENT_ID}&grant_type=refresh_token&refresh_token=rt-1\"\n")), "{text}");
        io(&mut app, &mut host, refresh, 200, &response("", r#"{"access_token":"at-2","expires_in":3600}"#));
        assert_eq!(token(&app), Some("at-2"));
        assert_eq!(results(&mut app), [(0, "at-2".to_string())]);
        // Google revoked the refresh token: it is dropped and Drive pauses until the user signs in.
        host.advance(3_600_000.0);
        assert_eq!(ensure_token(&mut app, &mut host, false, tag(Mod::Shell, 78)), Token::Pending);
        let refresh = req_of(&out(&mut host), "fetch");
        io(&mut app, &mut host, refresh, 400, &response("", r#"{"error":"invalid_grant"}"#));
        assert_eq!(results(&mut app), [(-1, "Refresh failed: 400 {\"error\":\"invalid_grant\"}".to_string())]);
        assert!(!host.local.contains_key(REFRESH_TOKEN_KEY) && reconnecting(&app));
        // Without a refresh token a silent request fails at once; an interactive one starts PKCE again.
        assert_eq!(ensure_token(&mut app, &mut host, false, W_IGNORE), Token::Failed("No refresh token; user must sign in".into()));
        out(&mut host);
        assert_eq!(ensure_token(&mut app, &mut host, true, W_IGNORE), Token::Failed("No refresh token; user must sign in".into()));
        let text = out(&mut host);
        io(&mut app, &mut host, req_of(&text, "digest"), 0, &[0; 32]);
        let text = out(&mut host);
        assert!(!text.contains("prompt=consent"), "consent was granted before");
        // No Android bridge (a browser pretending to be the app): fall back to a navigation.
        io(&mut app, &mut host, req_of(&text, "native_call"), -1, b"");
        assert!(out(&mut host).starts_with("navigate 3 \"https://accounts.google.com/o/oauth2/v2/auth?client_id="));
        assert_eq!(host.logs.last().map(|l| l.1.as_str()), Some("PKCE sign-in: Android bridge unavailable, falling back to web flow."));
        // Errors of the redirect and of the exchange reach the status line.
        msg(&mut app, &mut host, chan::OAUTH, "error=access_denied");
        assert_eq!(out(&mut host), format!("text {} \"Sign-in failed: access_denied\"\n", Ui::DRIVE_STATUS.0));
        msg(&mut app, &mut host, chan::OAUTH, "code=zz");
        let exchange = req_of(&out(&mut host), "fetch");
        io(&mut app, &mut host, exchange, -2, b"");
        assert_eq!(out(&mut host), format!("text {} \"Sign-in failed. Try again.\"\n", Ui::DRIVE_STATUS.0));
        assert_eq!(host.logs.last().map(|l| l.1.as_str()), Some("Auth code exchange failed: Network request timed out"));
    }

    #[test]
    fn the_apk_refreshes_silently_at_boot_and_only_logs_a_failure() {
        let (mut app, mut host) = boot(mode::APP | fx::BOOT_SIGNED_IN, |h| {
            h.local.insert(REFRESH_TOKEN_KEY.into(), "rt-9".into());
        });
        let text = out(&mut host);
        let refresh = req_of(&text, "fetch");
        assert!(line_of(&text, "fetch").contains("refresh_token=rt-9"));
        io(&mut app, &mut host, refresh, -1, b"Failed to fetch");
        assert!(!reconnecting(&app) && shell::view(&app) == View::Home);
        assert_eq!(host.logs.last().map(|l| l.1.as_str()), Some("Silent token refresh failed: Failed to fetch"));
        assert_eq!(host.local[REFRESH_TOKEN_KEY], "rt-9");
    }

    #[test]
    fn gis_is_retried_until_its_script_has_loaded() {
        let (mut app, mut host) = boot(0, |_| {});
        let text = out(&mut host);
        io(&mut app, &mut host, req_of(&text, "gis_init"), -1, b"");
        let text = out(&mut host);
        assert!(text.starts_with("query 1024 1 \"script[src*=\\\"accounts.google.com/gsi/client\\\"]\"\nlisten 1024 \"load\""), "{text}");
        assert!(text.ends_with(&format!("release 1024\ntimer_set {T_RETRY} 200 0\n")));
        timer(&mut app, &mut host, T_RETRY);
        let text = out(&mut host);
        io(&mut app, &mut host, req_of(&text, "gis_init"), -1, b"");
        assert_eq!(out(&mut host), format!("timer_set {T_RETRY} 200 0\n"));
        // After 30 s the retry slows to every 2 s.
        host.advance(30_000.0);
        timer(&mut app, &mut host, T_RETRY);
        let text = out(&mut host);
        io(&mut app, &mut host, req_of(&text, "gis_init"), -1, b"");
        assert_eq!(out(&mut host), format!("timer_set {T_RETRY} 2000 0\n"));
        // The script's load event retries at once.
        dom(&mut app, &mut host, L_GIS_SCRIPT, crate::Node(1024), crate::abi::ety::LOAD, 0, "", &[]);
        let text = out(&mut host);
        assert!(text.starts_with(&format!("timer_clear {T_RETRY}\n")));
        io(&mut app, &mut host, req_of(&text, "gis_init"), 0, b"");
        assert!(app.st.auth.gis_ready);
        // Signing in before GIS is ready fails politely.
        app.st.auth.gis_ready = false;
        sign_in(&mut app, &mut host);
        assert!(out(&mut host).ends_with(&format!("text {} \"Sign-in failed. Try again.\"\n", Ui::DRIVE_STATUS.0)));
    }
}
