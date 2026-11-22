//! Import von QLC+-Fixture-Definitionen (.qxf).
//!
//! Unterstützt das neue Format (Preset-Attribute, QLC+ 4.12+) und das alte
//! (Group/Colour-Elemente). Modi, Kanalnamen, Default-Werte und
//! Capabilities inkl. Strobe-Hz und Farben werden übernommen.
//! Eigener Mini-XML-Parser, damit keine zusätzliche Abhängigkeit nötig ist.

use crate::config::{
    finalize_channel, parse_color, Capability, ChanKind, Effect, FixtureChannel, FixtureMode,
    FixtureType, DEFAULT_STROBE,
};

// ---------------------------------------------------------------------------
// Mini-XML
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct Elem {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Elem>,
    pub text: String,
}

impl Elem {
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    }

    pub fn child(&self, n: &str) -> Option<&Elem> {
        self.children.iter().find(|c| c.name == n)
    }

    pub fn all<'a>(&'a self, n: &'a str) -> impl Iterator<Item = &'a Elem> + 'a {
        self.children.iter().filter(move |c| c.name == n)
    }

    pub fn text(&self) -> &str {
        self.text.trim()
    }
}

pub fn parse_xml(s: &str) -> Result<Elem, String> {
    let mut stack: Vec<Elem> = vec![Elem {
        name: "#doc".into(),
        ..Default::default()
    }];
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").ok_or("XML-Kommentar nicht geschlossen")? + 3;
        } else if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").ok_or("CDATA nicht geschlossen")?;
            top(&mut stack).text.push_str(&rest[9..end]);
            i += end + 3;
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            i += rest.find('>').ok_or("XML-Deklaration nicht geschlossen")? + 1;
        } else if rest.starts_with("</") {
            let end = rest.find('>').ok_or("End-Tag nicht geschlossen")?;
            let name = rest[2..end].trim();
            if stack.len() < 2 {
                return Err(format!("unerwartetes </{name}>"));
            }
            let el = stack.pop().unwrap_or_default();
            if el.name != name {
                return Err(format!("</{name}> passt nicht zu <{}>", el.name));
            }
            top(&mut stack).children.push(el);
            i += end + 1;
        } else if rest.starts_with('<') {
            let end = find_tag_end(rest).ok_or("Tag nicht geschlossen")?;
            let mut inner = &rest[1..end];
            let self_close = inner.ends_with('/');
            if self_close {
                inner = &inner[..inner.len() - 1];
            }
            let el = parse_tag(inner)?;
            if self_close {
                top(&mut stack).children.push(el);
            } else {
                stack.push(el);
            }
            i += end + 1;
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            top(&mut stack).text.push_str(&decode(&rest[..end]));
            i += end;
        }
    }
    if stack.len() != 1 {
        return Err(format!("<{}> nicht geschlossen", top(&mut stack).name));
    }
    Ok(stack.pop().unwrap_or_default())
}

fn top(stack: &mut [Elem]) -> &mut Elem {
    stack.last_mut().expect("XML-Stack nie leer")
}

fn find_tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, &c) in s.as_bytes().iter().enumerate() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == b'"' || c == b'\'' => quote = Some(c),
            None if c == b'>' => return Some(i),
            None => {}
        }
    }
    None
}

fn parse_tag(inner: &str) -> Result<Elem, String> {
    let inner = inner.trim();
    let name_end = inner.find(char::is_whitespace).unwrap_or(inner.len());
    let mut el = Elem {
        name: inner[..name_end].to_string(),
        ..Default::default()
    };
    let mut s = inner[name_end..].trim_start();
    while !s.is_empty() {
        let eq = s
            .find('=')
            .ok_or_else(|| format!("Attribut ohne Wert in <{}>", el.name))?;
        let key = s[..eq].trim().to_string();
        s = s[eq + 1..].trim_start();
        let q = s.chars().next().ok_or("leerer Attributwert")?;
        if q != '"' && q != '\'' {
            return Err(format!("Attribut '{key}' ohne Anführungszeichen"));
        }
        let close = s[1..].find(q).ok_or("Attributwert nicht geschlossen")? + 1;
        el.attrs.push((key, decode(&s[1..close])));
        s = s[close + 1..].trim_start();
    }
    Ok(el)
}

fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let r = &rest[p..];
        if let Some(semi) = r.find(';') {
            let ent = &r[1..semi];
            let ch = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                    u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
                }
                _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            if let Some(c) = ch {
                out.push(c);
                rest = &r[semi + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &r[1..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// QLC+ → eigene Typen
// ---------------------------------------------------------------------------

pub fn parse_qxf(id: &str, text: &str) -> Result<(FixtureType, Vec<String>), String> {
    let doc = parse_xml(text).map_err(|e| crate::tf!("XML-Fehler: {e}", "XML error: {e}"))?;
    let root = doc
        .child("FixtureDefinition")
        .ok_or(crate::i18n::t(
            "kein <FixtureDefinition> – ist das eine QLC+-Fixture?",
            "no <FixtureDefinition> – is this a QLC+ fixture?",
        ))?;
    let manu = root.child("Manufacturer").map(Elem::text).unwrap_or("");
    let model = root.child("Model").map(Elem::text).unwrap_or(id);
    let name = format!("{manu} {model}").trim().to_string();
    let mut warnings = Vec::new();

    let mut chans: Vec<(String, FixtureChannel)> = Vec::new();
    for ch in root.all("Channel") {
        let cname = ch.attr("Name").unwrap_or("?").to_string();
        let kind = kind_from_channel(ch);
        let rest = ch
            .attr("Default")
            .and_then(|v| v.trim().parse::<u8>().ok())
            .unwrap_or(0);
        let caps = ch.all("Capability").filter_map(|c| capability(c, kind)).collect();
        let mut fc = FixtureChannel {
            kind,
            label: cname.clone(),
            rest,
            caps,
        };
        finalize_channel(&mut fc);
        chans.push((cname, fc));
    }

    let mut modes = Vec::new();
    for m in root.all("Mode") {
        let mname = m.attr("Name").unwrap_or("Modus").to_string();
        let mut list: Vec<(usize, FixtureChannel)> = Vec::new();
        for mc in m.all("Channel") {
            let num = mc
                .attr("Number")
                .and_then(|n| n.trim().parse::<usize>().ok())
                .unwrap_or(list.len());
            let cn = mc.text();
            let fc = match chans.iter().find(|(n, _)| n == cn) {
                Some((_, c)) => c.clone(),
                None => {
                    warnings.push(crate::tf!(
                        "Modus '{mname}': Kanal '{cn}' nicht definiert",
                        "mode '{mname}': channel '{cn}' not defined"
                    ));
                    FixtureChannel {
                        kind: ChanKind::Other,
                        label: cn.to_string(),
                        rest: 0,
                        caps: Vec::new(),
                    }
                }
            };
            list.push((num, fc));
        }
        list.sort_by_key(|(n, _)| *n);
        modes.push(FixtureMode {
            name: mname,
            channels: list.into_iter().map(|(_, c)| c).collect(),
        });
    }
    if modes.is_empty() {
        return Err(crate::i18n::t("keine Modi (<Mode>) definiert", "no modes (<Mode>) defined").into());
    }

    Ok((
        FixtureType {
            id: id.to_string(),
            name,
            modes,
        },
        warnings,
    ))
}

fn kind_from_channel(ch: &Elem) -> ChanKind {
    use ChanKind as K;
    if let Some(k) = ch.attr("Preset").and_then(kind_from_preset) {
        return k;
    }
    let group = ch.child("Group");
    let gname = group.map(Elem::text).unwrap_or("");
    let fine = group
        .and_then(|g| g.attr("Byte"))
        .map_or(false, |b| b.trim() == "1");
    let colour = ch
        .child("Colour")
        .map(|c| c.text().to_lowercase())
        .unwrap_or_default();
    match gname {
        "Intensity" => {
            let k = match colour.as_str() {
                "red" => K::Red,
                "green" => K::Green,
                "blue" => K::Blue,
                "cyan" => K::Cyan,
                "magenta" => K::Magenta,
                "yellow" => K::Yellow,
                "white" => K::White,
                "amber" => K::Amber,
                "uv" => K::Uv,
                "lime" => K::Lime,
                "indigo" => K::Indigo,
                _ => K::Dimmer,
            };
            match (fine, k) {
                (false, k) => k,
                (true, K::Dimmer) => K::DimmerFine,
                (true, _) => K::Other,
            }
        }
        "Colour" => K::ColorWheel,
        "Gobo" => K::Gobo,
        "Prism" => K::Prism,
        "Shutter" => K::Shutter,
        "Speed" => K::EffectSpeed,
        "Effect" => K::Effect,
        "Pan" if fine => K::PanFine,
        "Pan" => K::Pan,
        "Tilt" if fine => K::TiltFine,
        "Tilt" => K::Tilt,
        "Maintenance" => K::Maintenance,
        _ => K::Other,
    }
}

fn kind_from_preset(p: &str) -> Option<ChanKind> {
    use ChanKind as K;
    let fine = p.ends_with("Fine");
    let base = p.trim_end_matches("Fine");
    Some(match base {
        "IntensityMasterDimmer" | "IntensityDimmer" if fine => K::DimmerFine,
        "IntensityMasterDimmer" | "IntensityDimmer" => K::Dimmer,
        "PositionPan" if fine => K::PanFine,
        "PositionPan" => K::Pan,
        "PositionTilt" if fine => K::TiltFine,
        "PositionTilt" => K::Tilt,
        // feine Farbkanäle o. Ä. nicht in die Farbvorschau mischen
        _ if fine => K::Other,
        "IntensityRed" => K::Red,
        "IntensityGreen" => K::Green,
        "IntensityBlue" => K::Blue,
        "IntensityCyan" => K::Cyan,
        "IntensityMagenta" => K::Magenta,
        "IntensityYellow" => K::Yellow,
        "IntensityAmber" => K::Amber,
        "IntensityWhite" => K::White,
        "IntensityUV" => K::Uv,
        "IntensityIndigo" => K::Indigo,
        "IntensityLime" => K::Lime,
        "ColorMacro" => K::ColorMacro,
        "ColorWheel" => K::ColorWheel,
        "ColorCTOMixer" | "ColorCTCMixer" | "ColorCTBMixer" => K::Cto,
        "GoboWheel" | "GoboIndex" => K::Gobo,
        "ShutterStrobeSlowFast" | "ShutterStrobeFastSlow" => K::Shutter,
        "ShutterIrisMinToMax" | "ShutterIrisMaxToMin" => K::Iris,
        "BeamFocusNearFar" | "BeamFocusFarNear" => K::Focus,
        "BeamZoomSmallBig" | "BeamZoomBigSmall" => K::Zoom,
        "PrismRotationSlowFast" | "PrismRotationFastSlow" => K::PrismRotation,
        _ if base.starts_with("Speed") => K::PanTiltSpeed,
        "NoFunction" => K::Other,
        _ => return None,
    })
}

fn is_strobe_preset(p: &str) -> bool {
    ["Strobe", "Pulse", "Ramp", "Flicker", "Lightning"]
        .iter()
        .any(|k| p.contains(k))
}

fn capability(c: &Elem, kind: ChanKind) -> Option<Capability> {
    let mut min: u8 = c.attr("Min")?.trim().parse().ok()?;
    let mut max: u8 = c.attr("Max")?.trim().parse().ok()?;
    if min > max {
        std::mem::swap(&mut min, &mut max);
    }
    let label_raw = c.text().to_string();
    let preset = c.attr("Preset").unwrap_or("");
    let res1 = c.attr("Res1").or_else(|| c.attr("Res"));
    let res2 = c.attr("Res2");
    let num = |s: Option<&str>| s.and_then(|v| v.trim().parse::<f32>().ok());
    let default_strobe = |p: &str| {
        if p.contains("FastToSlow") || p.contains("FastSlow") {
            (DEFAULT_STROBE.1, DEFAULT_STROBE.0)
        } else {
            DEFAULT_STROBE
        }
    };

    let effect = if preset == "ShutterOpen" {
        Effect::Open
    } else if preset == "ShutterClose" {
        Effect::Closed
    } else if is_strobe_preset(preset) {
        let (from, to) = match (num(res1), num(res2)) {
            (Some(a), Some(b)) => (a, b),
            (Some(a), None) => (a, a),
            _ => default_strobe(preset),
        };
        Effect::Strobe {
            from,
            to,
            random: preset.contains("Random"),
        }
    } else if let Some(col) = res1
        .and_then(parse_color)
        .or_else(|| c.attr("Color").and_then(parse_color))
    {
        let col2 = res2
            .and_then(parse_color)
            .or_else(|| c.attr("Color2").and_then(parse_color));
        Effect::Color(col, col2)
    } else if kind == ChanKind::Shutter {
        // alte Fixtures ohne Presets: am Text erkennen
        let l = label_raw.to_lowercase();
        if l.contains("strobe") || l.contains("flash") || l.contains("blitz") || l.contains("puls") {
            let (from, to) = default_strobe(&label_raw.replace(' ', ""));
            Effect::Strobe {
                from,
                to,
                random: l.contains("random") || l.contains("zufall"),
            }
        } else if l.contains("close") || l.contains("blackout") || l.contains("zu") {
            Effect::Closed
        } else if l.contains("open") || l.contains("offen") || l.contains("on") {
            Effect::Open
        } else {
            Effect::Nothing
        }
    } else if label_raw.is_empty() {
        Effect::Nothing
    } else {
        Effect::Text
    };

    let label = if label_raw.is_empty() {
        preset.to_string()
    } else {
        label_raw
    };
    Some(Capability {
        min,
        max,
        effect,
        label,
    })
}
