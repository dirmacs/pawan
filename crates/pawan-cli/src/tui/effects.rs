//! Motion + value animation that give the TUI its own visual identity.
//!
//! Two complementary systems live here:
//!
//! * **tachyonfx** animates already-rendered *cells* (fade/sweep/pulse). Each
//!   helper is applied in `ui()` *after* the relevant widget has been drawn.
//! * **animate-core** interpolates the underlying *values* a frame is built from
//!   (token counter roll, context-bar glide, accent-colour fade).
//!
//! Both are driven by the same clamped per-frame delta from [`frame_tick`], so a
//! long blocking gap (e.g. a synchronous agent call) can't fast-forward an
//! animation in a single jarring jump, and the two systems never drift apart.

use std::time::Instant;

use ratatui::style::Color;
use tachyonfx::{fx, Duration as FxDuration, Effect, Interpolation, Motion};

use animate_core::{easing, Once, Tween, TweenAnim};

/// Largest per-frame delta handed to the effect system. Keeps animations smooth
/// even when the render loop stalls between draws.
const MAX_TICK_MS: u32 = 100;

/// Advance the frame clock and return the elapsed time as a tachyonfx duration.
///
/// The returned delta is clamped to [`MAX_TICK_MS`] so animations degrade
/// gracefully (slow but never skipping) after an idle stretch.
pub(crate) fn frame_tick(last: &mut Instant) -> FxDuration {
    let now = Instant::now();
    let elapsed = now.duration_since(*last);
    *last = now;
    let ms = (elapsed.as_millis() as u32).min(MAX_TICK_MS);
    FxDuration::from_millis(ms)
}

/// Soft reveal for freshly finalized assistant content: the foreground fades up
/// from the surface colour so a new turn "develops" into view instead of
/// snapping in. Subtle and cheap — backgrounds are untouched.
pub(crate) fn content_reveal(surface: Color) -> Effect {
    fx::fade_from_fg(surface, (320, Interpolation::QuadOut))
}

/// Sweep a modal popup in from the top with a short gradient, giving overlays a
/// deliberate "slide + settle" entrance rather than a hard pop.
pub(crate) fn popup_open(faded: Color) -> Effect {
    fx::sweep_in(Motion::UpToDown, 8, 0, faded, (240, Interpolation::QuadOut))
}

/// Brief accent glow for the status strip when token usage / context updates.
/// Ping-pong returns the foreground to its resolved colour automatically.
pub(crate) fn status_pulse(accent: Color) -> Effect {
    fx::ping_pong(fx::fade_to_fg(accent, (180, Interpolation::SineInOut)))
}

// --- Value tweens (animate-core) ----------------------------------------------
//
// The token counter rolls toward its new total, the context bar glides to its
// new fill, and the accent colour fades across theme switches. All three share a
// single global clock advanced once per frame via [`advance_value_clock`].

/// Roll duration (ms) for the cumulative token counter.
const TOKEN_ROLL_MS: f64 = 600.0;
/// Glide duration (ms) for the context-usage bar.
const CTX_GLIDE_MS: f64 = 450.0;
/// Fade duration (ms) for accent-colour transitions on `/theme` switches.
const ACCENT_FADE_MS: f64 = 400.0;

/// Concrete [`animate_core::Tween`] handle stored in
/// [`App`](crate::tui::app::App) state. Function-pointer easing/interpolation
/// keep the type nameable (no closures captured in the struct).
pub(crate) type ValueTween<T> = Tween<T, fn(f64) -> f64, fn(&T, &T, f64) -> T, Once>;

/// Advance `animate-core`'s global frame clock by the same clamped delta
/// tachyonfx uses, so value tweens and cell effects stay in lockstep. Call
/// exactly once per rendered frame.
pub(crate) fn advance_value_clock(tick: FxDuration) {
    animate_core::tick(tick.milliseconds as usize);
}

/// Tween that rolls the displayed token total from its old value to the new one.
pub(crate) fn token_roll_tween() -> ValueTween<f64> {
    Tween::new(
        0.0,
        TOKEN_ROLL_MS,
        easing::cubic_out as fn(f64) -> f64,
        f64::tween as fn(&f64, &f64, f64) -> f64,
    )
}

/// Tween that eases the context-usage fraction (0.0..=1.0) toward its new value.
pub(crate) fn ctx_glide_tween() -> ValueTween<f32> {
    Tween::new(
        0.0,
        CTX_GLIDE_MS,
        easing::cubic_out as fn(f64) -> f64,
        f32::tween as fn(&f32, &f32, f64) -> f32,
    )
}

/// Tween that fades the accent colour across palette switches. Replaces the
/// hand-rolled `theme::ColorTransition`.
pub(crate) fn accent_fade_tween(initial: Color) -> ValueTween<Color> {
    Tween::new(
        initial,
        ACCENT_FADE_MS,
        easing::cubic_out as fn(f64) -> f64,
        Color::tween as fn(&Color, &Color, f64) -> Color,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use animate_core::Animate;
    use std::time::{Duration, Instant};

    #[test]
    fn frame_tick_advances_and_clamps() {
        // A long gap is clamped to MAX_TICK_MS so animations never fast-forward.
        let mut last = Instant::now() - Duration::from_secs(5);
        let tick = frame_tick(&mut last);
        assert_eq!(tick.milliseconds, MAX_TICK_MS);
        // `last` is advanced to ~now, so the next tick is tiny.
        let next = frame_tick(&mut last);
        assert!(next.milliseconds <= MAX_TICK_MS);
    }

    #[test]
    fn constructors_yield_running_effects() {
        assert!(content_reveal(Color::Black).running());
        assert!(popup_open(Color::Black).running());
        assert!(status_pulse(Color::Cyan).running());
    }

    #[test]
    fn effects_complete_after_their_duration() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        let mut fx = content_reveal(Color::Black);
        // Process well past the 320ms duration; the effect should report done.
        fx.process(FxDuration::from_millis(1_000), &mut buf, area);
        assert!(fx.done());
    }

    #[test]
    fn value_tweens_start_at_initial() {
        assert_eq!(*token_roll_tween().get(), 0.0);
        assert_eq!(*ctx_glide_tween().get(), 0.0);
        let accent = Color::Rgb(0, 204, 204);
        assert_eq!(*accent_fade_tween(accent).get(), accent);
    }

    #[test]
    fn token_tween_rolls_toward_target() {
        let mut tok = token_roll_tween();
        tok.set(1000.0);
        tok.update();
        let early = *tok.get();
        // Advancing the shared clock well past the roll duration settles it.
        advance_value_clock(FxDuration::from_millis(2_000));
        tok.update();
        let settled = *tok.get();
        assert!(settled >= early);
        assert!((settled - 1000.0).abs() < 1.0);
    }

    #[test]
    fn accent_tween_interpolates_rgb() {
        let mut accent = accent_fade_tween(Color::Rgb(0, 0, 0));
        accent.set(Color::Rgb(200, 100, 50));
        accent.update();
        advance_value_clock(FxDuration::from_millis(2_000));
        accent.update();
        assert_eq!(*accent.get(), Color::Rgb(200, 100, 50));
    }
}
