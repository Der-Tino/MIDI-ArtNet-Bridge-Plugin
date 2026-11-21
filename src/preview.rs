//! Live-Vorschau: aus den aktuellen DMX-Werten eines Geräts eine Farbe
//! und einen Strobe-Zustand berechnen.

use crate::config::{ChanKind, Effect, FixtureChannel, PatchedFixture};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shutter {
    Open,
    Closed,
    Blink { hz: f32, random: bool },
}

#[derive(Clone, Copy, Debug)]
pub struct Preview {
    /// Farbe bei offenem Shutter
    pub rgb: [u8; 3],
    /// erzeugt das Gerät überhaupt Licht (Dimmer/Farbe/Farbrad)?
    pub emits: bool,
    pub shutter: Shutter,
    /// tatsächlich sichtbar zum Zeitpunkt t (inkl. Blinken)
    pub lit: bool,
}

pub fn shutter_state(ch: &FixtureChannel, v: u8) -> Shutter {
    match ch.cap_at(v) {
        Some(cap) => match cap.effect {
            Effect::Closed => Shutter::Closed,
            Effect::Strobe { random, .. } => Shutter::Blink {
                hz: cap.strobe_hz(v).unwrap_or(0.0),
                random,
            },
            _ => Shutter::Open,
        },
        None => Shutter::Open,
    }
}

/// Blinkt bei `hz` mit 50 % Tastverhältnis (sonst sind schnelle Blitze
/// bei 60 fps GUI-Bildrate unsichtbar).
pub fn blink_on(hz: f32, random: bool, t: f64) -> bool {
    if hz <= 0.0 {
        return true;
    }
    let x = t * hz as f64;
    let idx = x.floor() as u64;
    let phase = x - x.floor();
    if random {
        let h = idx
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (h >> 33) % 3 == 0 && phase < 0.5
    } else {
        phase < 0.5
    }
}

pub fn compute(f: &PatchedFixture, vals: &[u8], t: f64) -> Preview {
    use ChanKind as K;
    let mut add = [0f32; 3];
    let mut has_add = false;
    let mut cmy = [0f32; 3];
    let mut has_cmy = false;
    let mut wheel: Option<[f32; 3]> = None;
    let mut dim = 1f32;
    let mut has_dim = false;
    let mut shutter = Shutter::Open;

    for (ch, &v) in f.channels.iter().zip(vals) {
        let x = v as f32;
        let mut a = |r: f32, g: f32, b: f32| {
            add[0] += r * x;
            add[1] += g * x;
            add[2] += b * x;
            has_add = true;
        };
        match ch.kind {
            K::Red => a(1.0, 0.0, 0.0),
            K::Green => a(0.0, 1.0, 0.0),
            K::Blue => a(0.0, 0.0, 1.0),
            K::White => a(1.0, 1.0, 1.0),
            K::Amber => a(1.0, 0.75, 0.0),
            K::Uv => a(0.35, 0.0, 0.9),
            K::Lime => a(0.6, 1.0, 0.0),
            K::Indigo => a(0.3, 0.0, 0.6),
            K::Cyan => {
                cmy[0] = x;
                has_cmy = true;
            }
            K::Magenta => {
                cmy[1] = x;
                has_cmy = true;
            }
            K::Yellow => {
                cmy[2] = x;
                has_cmy = true;
            }
            K::ColorWheel | K::ColorMacro => {
                if let Some(Effect::Color(c1, c2)) = ch.cap_at(v).map(|c| &c.effect) {
                    let mut c = [c1[0] as f32, c1[1] as f32, c1[2] as f32];
                    if let Some(c2) = c2 {
                        for i in 0..3 {
                            c[i] = (c[i] + c2[i] as f32) / 2.0;
                        }
                    }
                    wheel = Some(c);
                }
            }
            K::Dimmer => {
                dim *= x / 255.0;
                has_dim = true;
            }
            K::Shutter => {
                shutter = match (shutter, shutter_state(ch, v)) {
                    (Shutter::Closed, _) | (_, Shutter::Closed) => Shutter::Closed,
                    (Shutter::Blink { hz: h1, random: r1 }, Shutter::Blink { hz: h2, random: r2 }) => {
                        if h1 >= h2 {
                            Shutter::Blink { hz: h1, random: r1 }
                        } else {
                            Shutter::Blink { hz: h2, random: r2 }
                        }
                    }
                    (Shutter::Blink { hz, random }, _) | (_, Shutter::Blink { hz, random }) => {
                        Shutter::Blink { hz, random }
                    }
                    _ => Shutter::Open,
                };
            }
            _ => {}
        }
    }

    let emits = has_add || has_cmy || wheel.is_some() || has_dim;
    let add_dark = add.iter().all(|&c| c <= 0.0);
    let mut base = if has_add && !(add_dark && wheel.is_some()) {
        add
    } else if has_cmy {
        [255.0 - cmy[0], 255.0 - cmy[1], 255.0 - cmy[2]]
    } else {
        [255.0; 3]
    };
    if let Some(w) = wheel {
        for i in 0..3 {
            base[i] = base[i] * w[i] / 255.0;
        }
    }
    let rgb = [
        (base[0] * dim).clamp(0.0, 255.0) as u8,
        (base[1] * dim).clamp(0.0, 255.0) as u8,
        (base[2] * dim).clamp(0.0, 255.0) as u8,
    ];

    let open_now = match shutter {
        Shutter::Open => true,
        Shutter::Closed => false,
        Shutter::Blink { hz, random } => blink_on(hz, random, t),
    };
    let lit = emits && open_now && rgb.iter().any(|&c| c > 0);

    Preview {
        rgb,
        emits,
        shutter,
        lit,
    }
}
