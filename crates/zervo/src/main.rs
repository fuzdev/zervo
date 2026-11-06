//! Zervo: a native window host for Zzz (Servo + winit, Wayland).
//!
//! Opens one winit window, embeds a Servo `WebView` in it, loads the target
//! URL, and forwards the window's input to the page: keyboard, mouse, wheel,
//! and touch, plus the `HiDPI` scale factor, the size, and the theme, where
//! winit reports one. Pointed at a running Zzz daemon
//! (`http://localhost:4460/` as the final argument, or `ZERVO_URL`), it
//! doubles as the Servo compat probe against the real Zzz SPA.
//!
//! This process never embeds the daemon. With no URL it shows a stub page;
//! otherwise it points at a separately-running `zzz_server` over loopback.
//!
//! Every key goes to the page — there are no host shortcuts, so a terminal in
//! the page sees Ctrl+C. The DOM `copy` / `cut` / `paste` events never fire:
//! servoshell produces them by turning Ctrl+X/C/V into
//! `InputEvent::EditingAction` instead of sending the key to the page, which
//! isn't done here (text fields still cut, copy, and paste through Servo's own
//! key handling). The winit → Servo mapping is pure functions in `keyutils`
//! (keyboard) and `input` (pointer, wheel, touch, cursor, theme), mirroring
//! servoshell's headed window.
//!
//! The loop is `ControlFlow::Wait`: it sleeps until a window event arrives or
//! Servo wakes it (`render_loop`). Servo is spun on each wake-up and after each
//! window event, and a frame is painted only on `RedrawRequested`, which the
//! delegate asks for when Servo has a frame ready.
//!
//! Exit status: 0 after a clean close, 2 for a usage error (an argument or
//! `ZERVO_URL` that isn't valid UTF-8, or a URL that doesn't parse —
//! nothing is started), 1 when the window or engine fails to start or the
//! event loop fails. Logs go to stderr.
//!
//! The embedding surface still moves: reconcile against the Servo revision
//! pinned in `Cargo.lock` on each bump.

mod input;
mod keyutils;
mod render_loop;

use std::error::Error;
use std::ffi::OsString;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
// `servo` is the embedding crate. `RenderingContext` is the trait whose
// `make_current`/`present` methods we call on the window context.
use servo::{
    Cursor, DevicePoint, InputEvent, MouseButtonEvent, MouseLeftViewportEvent, MouseMoveEvent,
    RenderingContext, Servo, ServoBuilder, ServoUrl, TouchEvent, WebView, WebViewBuilder,
    WebViewDelegate, WheelEvent, WindowRenderingContext,
};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, Touch, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::render_loop::{AppEvent, LoopStats, Waker};

/// Minimal stub page — the default target, proving the render path without a
/// running daemon. Override with the final argument or `ZERVO_URL` to point
/// at a real `zzz_server` (`http://localhost:4460/`) for the live compat audit.
const STUB_URL: &str = "data:text/html,<!doctype html><meta charset=utf-8><title>Zervo</title><body style='font:48px/1.4 system-ui;display:grid;place-items:center;height:100vh;margin:0;background:%23111;color:%23eee'>zervo · servo · wayland</body>";

/// Env var that sets the target URL (lower precedence than an argument).
const URL_ENV: &str = "ZERVO_URL";

/// Resolves the URL to load: the **final** argument > `ZERVO_URL` > stub.
/// `args` excludes the program name. The final argument is the one zzz's
/// opener contract fills: the URL goes after any arguments the opener was
/// configured with. A blank env value reads as unset, but a blank final
/// argument doesn't: it's returned as is and fails the URL parse, rather than
/// quietly loading something other than what the caller passed.
fn resolve_url(args: impl Iterator<Item = String>, env_url: Option<String>) -> String {
    args.last()
        .or_else(|| env_url.filter(|url| !url.trim().is_empty()))
        .unwrap_or_else(|| STUB_URL.to_string())
}

/// Decodes OS strings (the arguments, the env value) as UTF-8, returning the
/// first one that isn't — a usage error, never loaded lossily.
fn utf8_strings(values: impl Iterator<Item = OsString>) -> Result<Vec<String>, OsString> {
    values.map(OsString::into_string).collect()
}

/// Resolves and parses the target URL from the process's arguments and
/// environment, logging the reason on failure.
fn target_url() -> Option<ServoUrl> {
    let args = match utf8_strings(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(arg) => {
            tracing::error!(?arg, "argument is not valid UTF-8");
            return None;
        }
    };
    let env_url = match utf8_strings(std::env::var_os(URL_ENV).into_iter()) {
        Ok(mut values) => values.pop(),
        Err(value) => {
            tracing::error!(?value, "{URL_ENV} is not valid UTF-8");
            return None;
        }
    };
    let url = resolve_url(args.into_iter(), env_url);
    match ServoUrl::parse(&url) {
        Ok(url) => Some(url),
        Err(err) => {
            tracing::error!(%err, url, "invalid URL");
            None
        }
    }
}

/// Receives Servo's callbacks for the `WebView`. Owned by the `WebView`, so its
/// window reference is released when the `WebView` drops — before Servo's
/// shutdown and before `App` lets go of the window. Never store the `WebView`
/// handles the callbacks receive: a clone outliving `App`'s teardown order
/// would keep Servo alive past the rendering context.
struct ShellDelegate {
    window: Arc<Window>,
}

impl WebViewDelegate for ShellDelegate {
    /// Servo has a frame to show: ask winit for a `RedrawRequested`, where it
    /// is painted. Called from inside `Servo::spin_event_loop`, and repeated
    /// on every spin until the frame is painted — `request_redraw` collapses
    /// the repeats.
    fn notify_new_frame_ready(&self, _webview: WebView) {
        tracing::trace!("new frame ready");
        self.window.request_redraw();
    }

    fn notify_cursor_changed(&self, _webview: WebView, cursor: Cursor) {
        tracing::trace!(?cursor, "cursor changed");
        match input::cursor_icon_from_servo(cursor) {
            Some(icon) => {
                self.window.set_cursor(icon);
                self.window.set_cursor_visible(true);
            }
            None => self.window.set_cursor_visible(false),
        }
    }
}

/// Sets the `WebView`'s `HiDPI` scale factor (device pixels per CSS pixel) from
/// winit's. Servo's scale type is a `euclid::Scale`, which `servo` doesn't
/// re-export, so this rewrites the value of the `WebView`'s current scale
/// rather than naming the type — no direct `euclid` dependency.
fn set_hidpi_scale_factor(webview: &WebView, scale_factor: f64) {
    let mut scale = webview.hidpi_scale_factor();
    scale.0 = input::hidpi_scale_from_winit(scale_factor);
    webview.set_hidpi_scale_factor(scale);
}

// Field order is load-bearing: Rust drops fields top-to-bottom, and `Servo`'s
// Drop pumps its event loop to finish shutting down — which touches the
// compositor's GL context. So the WebView and Servo must drop *before* the
// rendering context and window. Teardown is also done explicitly on close
// (see `CloseRequested`); this ordering is the backstop.
struct App {
    webview: Option<WebView>,
    servo: Option<Servo>,
    rendering_context: Option<Rc<WindowRenderingContext>>,
    window: Option<Arc<Window>>,
    url: ServoUrl,
    /// Modifier state from the last `ModifiersChanged`, attached to key events.
    modifiers: ModifiersState,
    /// Last cursor position in device pixels, relative to the window's top
    /// left. Button and wheel events carry no position, so they use this one.
    cursor_position: DevicePoint,
    /// Set when `init` fails, so `main` can exit non-zero after the loop ends.
    init_failed: bool,
    /// Sends `AppEvent::Wake` to this loop. Servo gets a `Waker` built from a
    /// clone; it is a channel sender, holding nothing of the window or engine.
    proxy: EventLoopProxy<AppEvent>,
    /// Wake-up, spin, and paint counts for the debug log.
    stats: LoopStats,
    /// Whether a frame has been painted yet, to log the first one.
    painted: bool,
}

impl App {
    fn new(url: ServoUrl, proxy: EventLoopProxy<AppEvent>) -> Self {
        Self {
            webview: None,
            servo: None,
            rendering_context: None,
            window: None,
            url,
            modifiers: ModifiersState::empty(),
            cursor_position: DevicePoint::new(0.0, 0.0),
            init_failed: false,
            proxy,
            stats: LoopStats::new(Instant::now()),
            painted: false,
        }
    }

    /// Create the window, surfman-backed rendering context, engine, and `WebView`
    /// and store them on `self`. Returns the first failure for the caller to log.
    /// On a failure nothing is stored. Every `?` fires before the engine is
    /// built, so only the window exists then; were a later step to fail, the
    /// locals would drop in reverse declaration order — `WebView`, Servo,
    /// rendering context, window — the same order the fields give.
    /// The raw handles are the version-stable seam to Servo (raw-window-handle
    /// 0.6), so winit's version is independent; the rendering context is supplied
    /// to the `WebView`, not the engine.
    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        let window =
            Arc::new(event_loop.create_window(Window::default_attributes().with_title("Zervo"))?);
        // surfman's Error doesn't impl std::error::Error, so map it to a string
        // (Box<dyn Error>: From<String>); the other `?`s convert directly.
        let rendering_context = Rc::new(
            WindowRenderingContext::new(
                window.display_handle()?,
                window.window_handle()?,
                window.inner_size(),
            )
            .map_err(|err| format!("rendering context: {err:?}"))?,
        );
        // Without a waker Servo can't wake a waiting loop: its threads call it
        // for every message they queue for this thread, and the refresh driver
        // calls it at each frame start while the page animates.
        let waker = Waker::new(self.proxy.clone());
        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(waker))
            .build();
        let webview = WebViewBuilder::new(&servo, rendering_context.clone())
            .url(self.url.clone().into_url())
            .delegate(Rc::new(ShellDelegate {
                window: Arc::clone(&window),
            }))
            .build();
        // set right after the build rather than on the builder (see
        // `set_hidpi_scale_factor`): the page is told the real scale before it
        // has loaded anything
        set_hidpi_scale_factor(&webview, window.scale_factor());
        if let Some(theme) = window.theme() {
            webview.notify_theme_change(input::theme_from_winit(theme));
        }
        webview.focus();
        // The first paint: it has no page content yet, but its buffer is what
        // maps the window. The page's own first frame arrives through
        // `notify_new_frame_ready`.
        window.request_redraw();

        self.webview = Some(webview);
        self.servo = Some(servo);
        self.rendering_context = Some(rendering_context);
        self.window = Some(window);
        Ok(())
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Runs Servo's event loop once on this thread: it drains what Servo's
    /// threads have queued, runs delegate callbacks (a ready frame asks for a
    /// redraw there), and applies queued input such as scrolling. A no-op once
    /// the engine is gone, so a wake-up or window event arriving after
    /// teardown is harmless.
    fn spin(&mut self) {
        let Some(servo) = &self.servo else { return };
        servo.spin_event_loop();
        self.stats.record_spin();
    }

    /// Paints the `WebView` into the window's surface and presents it:
    /// make the GL context current → paint → present. Painting is also what
    /// re-arms Servo's refresh driver while the page animates, so the next
    /// frame's wake-up follows from this call.
    fn paint(&mut self) {
        let (Some(ctx), Some(webview), Some(window)) =
            (&self.rendering_context, &self.webview, &self.window)
        else {
            return;
        };
        if let Err(err) = ctx.make_current() {
            tracing::error!(?err, "make_current failed; skipping frame");
            return;
        }
        webview.paint();
        // On Wayland this asks the compositor for a frame callback (sent with
        // the commit `present` makes), and winit holds the next
        // `RedrawRequested` until it arrives. A hidden window gets no
        // callback, so it stops painting — and never reaches a second buffer
        // swap, which would block this thread until the window is shown again,
        // leaving the engine unspun and a close request unanswered. servoshell
        // doesn't make this call.
        // The cost: if `present` fails to commit (it only logs), the callback
        // never comes and winit withholds every later `RedrawRequested` — a
        // failed swap freezes the window rather than being retried.
        window.pre_present_notify();
        ctx.present();
        self.stats.record_paint();
        if !self.painted {
            self.painted = true;
            tracing::debug!("first frame painted");
        }
    }

    /// Sends an input event to the page, if the `WebView` is still alive.
    fn forward(&self, event: InputEvent) {
        if let Some(webview) = &self.webview {
            webview.notify_input_event(event);
        }
    }

    /// Whether `point` is inside the `WebView`, which fills the window. False
    /// once the `WebView` is gone.
    fn webview_contains(&self, point: DevicePoint) -> bool {
        self.webview.as_ref().is_some_and(|webview| {
            let size = webview.size();
            input::viewport_contains(size.width, size.height, point)
        })
    }

    fn forward_key(&self, key_event: &KeyEvent) {
        let event = keyutils::keyboard_event_from_winit(key_event, self.modifiers);
        // trace output includes typed characters (passwords too)
        tracing::trace!(?event, "key");
        self.forward(InputEvent::Keyboard(event));
    }

    /// Records the cursor position and forwards the move. A pointer grab (a
    /// drag) keeps reporting positions outside the window: the first such move
    /// tells the page the mouse left, and later ones are dropped — so there is
    /// no drag-autoscroll.
    fn forward_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        let point = input::device_point_from_winit(position);
        let previous = std::mem::replace(&mut self.cursor_position, point);
        if self.webview_contains(point) {
            tracing::trace!(?point, "mouse move");
            self.forward(InputEvent::MouseMove(MouseMoveEvent::new(point.into())));
        } else if self.webview_contains(previous) {
            tracing::trace!(?point, "mouse left viewport (moved outside)");
            self.forward(InputEvent::MouseLeftViewport(
                MouseLeftViewportEvent::default(),
            ));
        }
    }

    fn forward_cursor_left(&self) {
        if self.webview_contains(self.cursor_position) {
            tracing::trace!("mouse left viewport");
            self.forward(InputEvent::MouseLeftViewport(
                MouseLeftViewportEvent::default(),
            ));
        }
    }

    /// Forwards a button press or release at the last cursor position. Back
    /// and forward go to the page like any other button, not to history
    /// navigation.
    ///
    /// A press outside the window is dropped. A release outside it — the end
    /// of a drag that left the window, which the pointer grab still delivers —
    /// is clamped to the nearest point inside instead (a press within a few
    /// pixels of the edge can therefore still complete a click): Servo drops an
    /// event whose point hits nothing, and a lost release would leave the page
    /// mid-drag with the button recorded as held.
    fn forward_mouse_button(&self, state: ElementState, button: MouseButton) {
        let Some(webview) = &self.webview else { return };
        let size = webview.size();
        let Some(point) =
            input::mouse_button_point(state, size.width, size.height, self.cursor_position)
        else {
            let point = self.cursor_position;
            tracing::trace!(?button, ?point, "mouse press outside the viewport; dropped");
            return;
        };
        tracing::trace!(?button, ?state, ?point, "mouse button");
        self.forward(InputEvent::MouseButton(MouseButtonEvent::new(
            input::mouse_button_action_from_winit(state),
            input::mouse_button_from_winit(button),
            point.into(),
        )));
    }

    /// Forwards a wheel event at the last cursor position. Servo scrolls the
    /// page itself once the page has handled the event without preventing it.
    fn forward_wheel(&self, delta: MouseScrollDelta) {
        let delta = input::wheel_delta_from_winit(delta);
        tracing::trace!(?delta, "wheel");
        self.forward(InputEvent::Wheel(WheelEvent::new(
            delta,
            self.cursor_position.into(),
        )));
    }

    fn forward_touch(&self, touch: &Touch) {
        tracing::trace!(?touch, "touch");
        self.forward(InputEvent::Touch(TouchEvent::new(
            input::touch_event_type_from_winit(touch.phase),
            input::touch_id_from_winit(touch.id),
            input::device_point_from_winit(touch.location).into(),
        )));
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return; // already initialized
        }

        if let Err(err) = self.init(event_loop) {
            tracing::error!(%err, "failed to initialize zervo");
            self.init_failed = true;
            event_loop.exit();
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Wake => {
                self.stats.record_wake();
                self.spin();
            }
        }
    }

    // Every `WindowEvent` variant is named: forwarded to the page, handled
    // here, or given an empty arm with the reason it's ignored. There is
    // no catch-all, so a winit bump that adds a variant fails to compile
    // instead of dropping the event silently.
    //
    // Servo is spun after every event, as servoshell does: forwarded input,
    // a resize, or a new scale only takes effect in Paint on the next spin,
    // and with nothing else pending no wake-up would come to trigger one.
    #[expect(
        clippy::match_same_arms,
        reason = "each ignored group keeps its own arm and reason; as one or-pattern the comments would sit between `|` alternatives, which rustfmt can't format"
    )]
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                // Tear the engine down cleanly while the GL context + window are
                // still alive: Servo's Drop pumps its event loop to finish
                // shutting down, and that teardown touches the compositor's GL
                // context. Drop the WebView then Servo first, then exit.
                // Servo's threads may still wake the loop while it shuts down
                // and after: `spin` finds no engine, and once the loop is gone
                // the waker's send fails quietly.
                self.webview = None;
                self.servo = None;
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                // `WebView::resize` resizes the rendering context *and* tells
                // Paint and the page the new viewport. Resizing the context
                // directly would leave the page laid out at the old size — and
                // make a later `WebView::resize` a no-op, since it returns
                // early when the context already has the new size.
                tracing::debug!(?size, "resized");
                if let Some(webview) = &self.webview {
                    webview.resize(size);
                }
                // repaint at once rather than waiting for the page's relayout
                // to produce a frame: the surface has a new size to fill
                self.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // the new physical size arrives as its own `Resized`
                tracing::debug!(scale_factor, "scale factor changed");
                if let Some(webview) = &self.webview {
                    set_hidpi_scale_factor(webview, scale_factor);
                }
                self.request_redraw();
            }
            // asked for by the delegate when Servo has a frame ready, by the
            // arms above, and by the compositor (a newly exposed window)
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::ThemeChanged(theme) => {
                tracing::debug!(?theme, "theme changed");
                if let Some(webview) = &self.webview {
                    webview.notify_theme_change(input::theme_from_winit(theme));
                }
            }
            // never forwarded: the state rides on each key event
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            // synthetic events (keys already held when the window gains focus)
            // are forwarded like any other, as servoshell does
            WindowEvent::KeyboardInput { event, .. } => self.forward_key(&event),
            WindowEvent::CursorMoved { position, .. } => self.forward_cursor_moved(position),
            WindowEvent::CursorLeft { .. } => self.forward_cursor_left(),
            WindowEvent::MouseInput { state, button, .. } => {
                self.forward_mouse_button(state, button);
            }
            WindowEvent::MouseWheel { delta, .. } => self.forward_wheel(delta),
            WindowEvent::Touch(touch) => self.forward_touch(&touch),

            // Deliberately ignored, each with its reason.
            //
            // never requested (it's the answer to a startup-notify token request)
            WindowEvent::ActivationTokenDone { .. } => {}
            // the window's screen position isn't reported to the page
            WindowEvent::Moved(_) => {}
            // teardown runs on `CloseRequested`; nothing is left to release here
            WindowEvent::Destroyed => {}
            // Servo has no file drag-and-drop entry point on `WebView`
            WindowEvent::DroppedFile(_)
            | WindowEvent::HoveredFile(_)
            | WindowEvent::HoveredFileCancelled => {}
            // the WebView is focused once at creation and stays focused:
            // servoshell doesn't forward window focus either
            WindowEvent::Focused(_) => {}
            // winit sends no IME events until `set_ime_allowed(true)`, which
            // needs Servo's input-method requests wired through the delegate;
            // plain typing arrives as `KeyboardInput`
            WindowEvent::Ime(_) => {}
            // the next `CursorMoved` carries the position
            WindowEvent::CursorEntered { .. } => {}
            // winit emits these on Apple platforms only
            WindowEvent::PinchGesture { .. }
            | WindowEvent::PanGesture { .. }
            | WindowEvent::DoubleTapGesture { .. }
            | WindowEvent::RotationGesture { .. }
            | WindowEvent::TouchpadPressure { .. } => {}
            // raw joystick / tablet axes: no Servo input event for them
            WindowEvent::AxisMotion { .. } => {}
            // winit doesn't report occlusion on Wayland; a hidden window stops
            // painting because its frame callbacks stop (see `paint`)
            WindowEvent::Occluded(_) => {}
        }
        self.spin();
    }

    // Nothing drives the engine or a repaint from here: the loop is about to
    // sleep until the next window event or wake-up. Only the debug log's
    // activity line is flushed.
    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some((counts, elapsed)) = self.stats.take_report(Instant::now()) {
            tracing::debug!(
                wakes = counts.wakes,
                spins = counts.spins,
                paints = counts.paints,
                elapsed_ms = elapsed.as_millis(),
                "loop activity"
            );
        }
    }
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    // resolve and parse before anything else exists, so a bad argument or URL
    // never boots the engine
    let Some(url) = target_url() else {
        return ExitCode::from(2);
    };
    tracing::info!(%url, "zervo loading");

    let event_loop = match EventLoop::<AppEvent>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(err) => {
            tracing::error!(%err, "failed to create event loop");
            return ExitCode::FAILURE;
        }
    };
    // sleep until a window event or a wake-up from Servo
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::new(url, event_loop.create_proxy());
    if let Err(err) = event_loop.run_app(&mut app) {
        tracing::error!(?err, "event loop exited with error");
        return ExitCode::FAILURE;
    }
    if app.init_failed {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(args: &[&str]) -> impl Iterator<Item = String> {
        args.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn url_is_the_final_argument() {
        let url = resolve_url(args(&["http://localhost:4460/"]), None);
        assert_eq!(url, "http://localhost:4460/");
        let url = resolve_url(
            args(&["--flag", "value", "http://localhost:4460/?workspace=a"]),
            None,
        );
        assert_eq!(url, "http://localhost:4460/?workspace=a");
    }

    #[test]
    fn argument_wins_over_env() {
        let url = resolve_url(args(&["http://a/"]), Some("http://b/".to_string()));
        assert_eq!(url, "http://a/");
    }

    #[test]
    fn env_is_used_without_arguments() {
        assert_eq!(
            resolve_url(args(&[]), Some("http://b/".to_string())),
            "http://b/"
        );
    }

    #[test]
    fn blank_env_and_no_arguments_fall_back_to_the_stub() {
        assert_eq!(resolve_url(args(&[]), None), STUB_URL);
        assert_eq!(resolve_url(args(&[]), Some(String::new())), STUB_URL);
        assert_eq!(resolve_url(args(&[]), Some("  ".to_string())), STUB_URL);
    }

    #[test]
    fn blank_final_argument_is_not_a_fallback() {
        assert_eq!(resolve_url(args(&[""]), Some("http://b/".to_string())), "");
        assert_eq!(resolve_url(args(&["http://a/", ""]), None), "");
        assert!(ServoUrl::parse("").is_err());
    }

    #[test]
    fn utf8_strings_decode_in_order() {
        let values = [OsString::from("--flag"), OsString::from("http://a/")];
        assert_eq!(
            utf8_strings(values.into_iter()),
            Ok(vec!["--flag".to_string(), "http://a/".to_string()])
        );
        assert_eq!(utf8_strings(std::iter::empty()), Ok(vec![]));
    }

    #[test]
    fn utf8_strings_return_the_first_invalid_value() {
        use std::os::unix::ffi::OsStringExt;

        let invalid = OsString::from_vec(vec![b'a', 0xff]);
        let values = [
            OsString::from("ok"),
            invalid.clone(),
            OsString::from("later"),
        ];
        assert_eq!(utf8_strings(values.into_iter()), Err(invalid));
    }

    #[test]
    fn stub_url_parses() {
        assert!(ServoUrl::parse(STUB_URL).is_ok());
    }
}
