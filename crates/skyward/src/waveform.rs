//! What the vibration motor is given to play: a click's and the ticks'
//! samples, made for the frequency the motor swings at, and the share of
//! the full strength the slider gives.

use skyward_config::Strength;
use std::f32::consts::{FRAC_PI_2, TAU};

/// The rate a made click's samples are played at. It has to hold many samples
/// to each swing of the motor, or the wave given to it is no wave at all: at
/// 8000 a motor swinging at 1780 Hz got four and a half samples a swing, and
/// what came out was a rasping buzz at a quarter of the strength it should
/// have been (2026-11-17).
pub(crate) const RATE: u32 = 24000;

/// How long a made click rings for: long enough to be felt, short enough to
/// be a tap. It is counted in time rather than in swings, so that a motor
/// swinging slowly gets the same tap as a fast one -- two swings of a motor
/// at 1780 Hz came to two milliseconds, which is below what a finger notices.
///
/// 45 ms was felt but rang on into a rasping buzz: the motor is driven at its
/// resonant frequency, and driven on past a certain point its swing loses the
/// wave and comes out as noise (2026-11-17). 22 ms is felt, and is a tap.
const CLICK_MS: f32 = 22.0;
/// The share of the click the motor is driven through, the rest being the
/// swings turned over that stop it.
const DRIVE_SHARE: f32 = 0.6;
/// The ticks of the screen going off by the power key, as the phone's own
/// system plays them on the 17 Ultra, whose motor swings at [`TICKS_HZ`]:
/// two crisp ticks, the second lighter, each six half-swings at about one
/// and a half times the motor's frequency. Each half-swing of a tick is its
/// start, peak and end in milliseconds, its signed share of the full, and
/// how it rises to its peak and falls from it: as a quarter of a sine
/// raised to that power, 1 crossing zero at its steepest, higher powers
/// setting off and dying away more gently. Each of the ticks is when it
/// starts, in milliseconds, and its share of the tick in full. For another
/// motor the times go in proportion. They are made at [`TICKS_RATE`].
const TICKS_HZ: f32 = 172.7;
const TICK: [(f32, f32, f32, f32, f32, f32); 6] = [
    (0.125, 1.717, 2.375, 0.531, 2.6, 1.0),
    (2.375, 3.45, 4.5, -1.0, 1.1, 1.0),
    (4.5, 5.448, 6.375, 0.913, 1.0, 1.0),
    (6.375, 7.227, 8.063, -0.801, 1.0, 1.0),
    (8.063, 9.029, 10.025, 0.935, 1.0, 1.1),
    (10.025, 10.671, 12.167, -0.508, 1.0, 2.3),
];
const TICKS: [(f32, f32); 2] = [(0.0, 1.0), (55.125, 0.79)];
pub(crate) const TICKS_RATE: u32 = 24000;

/// The share of the click's full strength at the slider's `position`.
pub(crate) fn share(strength: &Strength, position: f32) -> f32 {
    strength.lightest * (strength.strongest / strength.lightest).powf(position.clamp(0.0, 1.0))
}

/// A click's samples at [`RATE`] for a motor that swings at `hz`: whole
/// swings that drive it, then swings turned over that stop it.
pub(crate) fn made(hz: f32) -> Vec<i8> {
    let all = (CLICK_MS / 1000.0 * RATE as f32).round() as usize;
    let drive = (all as f32 * DRIVE_SHARE).round() as usize;
    (0..all)
        .map(|i| {
            let wave = (TAU * hz * i as f32 / RATE as f32).sin() * f32::from(i8::MAX);
            (if i < drive { wave } else { -wave }).round() as i8
        })
        .collect()
}

/// The ticks at [`TICKS_RATE`] for a motor that swings at `hz`, still
/// between them.
pub(crate) fn ticks(hz: f32) -> Vec<i8> {
    let scale = TICKS_HZ / hz;
    let at = |ms: f32| (ms * scale * TICKS_RATE as f32 / 1000.0).round() as usize;
    let (last, _) = TICKS[TICKS.len() - 1];
    let mut samples = vec![0i8; at(last + TICK[TICK.len() - 1].2)];
    for (offset, strength) in TICKS {
        for (start, peak, end, share, rise, fall) in TICK {
            let (start, peak, end) = (start + offset, peak + offset, end + offset);
            for (i, sample) in samples.iter_mut().enumerate().take(at(end)).skip(at(start)) {
                let t = i as f32 * 1000.0 / TICKS_RATE as f32 / scale;
                let (way, power) = if t < peak { ((t - start) / (peak - start), rise) } else { ((end - t) / (end - peak), fall) };
                let k = (FRAC_PI_2 * way.clamp(0.0, 1.0)).sin().powf(power);
                *sample = (strength * share * k * f32::from(i8::MAX)).round() as i8;
            }
        }
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_made_click_drives_whole_swings_then_turns_over() {
        let samples = made(172.4);
        let drive = (DRIVE_SWINGS * RATE as f32 / 172.4).round() as usize;
        assert_eq!(samples.len(), (3.5 * RATE as f32 / 172.4).round() as usize);
        assert_eq!(samples[0], 0);
        assert_eq!(*samples.iter().max().unwrap(), i8::MAX);
        let quarter = (RATE as f32 / 172.4 / 4.0).round() as usize;
        assert!(samples[quarter] > 120, "it rises first");
        assert!(samples[drive + quarter] < -120, "it falls first once turned over");
    }

    #[test]
    fn the_screen_going_off_ticks_twice_crisply_the_second_lighter() {
        let at = |ms: f32| (ms * TICKS_RATE as f32 / 1000.0).round() as usize;
        let samples = ticks(TICKS_HZ);
        let (first, second) = (&samples[..at(12.167)], &samples[at(55.125)..]);
        assert_eq!(samples.len(), at(55.125 + 12.167));
        assert!(samples[at(12.167)..at(55.125)].iter().all(|&s| s == 0), "still between");
        let halves = |s: &[i8]| 1 + s.iter().filter(|&&s| s != 0).map(|&s| s > 0).collect::<Vec<_>>().windows(2).filter(|w| w[0] != w[1]).count();
        assert_eq!((halves(first), halves(second)), (6, 6), "six half-swings each");
        let motor = TICKS_RATE as f32 / TICKS_HZ / 2.0;
        assert!((at(12.167 - 0.125) as f32 / 6.0) < motor, "each quicker than the motor's own");
        assert_eq!(first.iter().min(), Some(&-i8::MAX), "the first swings to the full");
        for (i, (&a, &b)) in first.iter().zip(second).enumerate() {
            assert!((f32::from(a) * 0.79 - f32::from(b)).abs() <= 1.0, "the second is the first at 0.79: {a} and {b} at {i}");
        }
        assert_eq!(ticks(TICKS_HZ * 2.0).len(), samples.len().div_ceil(2), "a motor twice as quick ticks in half the time");
    }

    #[test]
    fn the_slider_spans_the_strengths_each_step_the_same_factor() {
        let s = Strength { lightest: 0.25, strongest: 1.0 };
        assert_eq!((share(&s, 0.0), share(&s, 1.0)), (0.25, 1.0));
        assert!((share(&s, 0.5) - 0.5).abs() < 1e-6);
        assert!((share(&s, 0.25) / share(&s, 0.0) - share(&s, 1.0) / share(&s, 0.75)).abs() < 1e-6, "each step the same factor");
        assert_eq!((share(&s, -1.0), share(&s, 2.0)), (0.25, 1.0), "no further than its ends");
    }
}
