//! Pulse lines: a sparkline of each agent's tool calls per minute on its row, so a busy, a stalled
//! and a looping agent each look different at a glance.
// wit-bindgen's flattened `notify` import has 14 params.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({ path: "wit/haunt-plugin.wit", world: "plugin" });

mod pulse;

use std::collections::BTreeMap;
use std::sync::Mutex;

use haunt::plugin::log::log;
use haunt::plugin::rows::{self, Decoration, Spark, Tone};
use haunt::plugin::timer;
use haunt::plugin::types::LogLevel;
use pulse::{Mood, Pulse};
use serde::Deserialize;

const TICK_SECS: u32 = 30;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub minutes: i64,
    pub quiet_after_min: i64,
    pub loop_after: i64,
    /// The `quiet 6m` and `same Bash call ×14` notes after the meta line.
    pub notes: bool,
    pub subagents: bool,
    pub unverified: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { minutes: 30, quiet_after_min: 5, loop_after: 8, notes: true, subagents: true, unverified: false }
    }
}

impl Settings {
    fn pulse(&self) -> pulse::Settings {
        pulse::Settings {
            minutes: self.minutes.clamp(5, 32) as usize,
            quiet_after_min: self.quiet_after_min.clamp(1, 120) as u64,
            loop_after: self.loop_after.clamp(3, 1000) as u32,
        }
    }
}

#[derive(Default)]
struct Live {
    settings: Settings,
    tabs: BTreeMap<String, Pulse>,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Live) -> R) -> R {
    f(LIVE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Live::default))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Counts scaled to the busiest minute in view: 0 is the bottom, 255 the top.
pub fn points(series: &[u32]) -> Vec<u8> {
    let max = series.iter().copied().max().unwrap_or(0);
    series.iter().map(|v| if max == 0 { 0 } else { (u64::from(*v) * 255 / u64::from(max)) as u8 }).collect()
}

/// Busy draws in the text colour, quiet dim and flat, a loop in orange with its note.
pub fn decoration(p: &Pulse, now: u64, s: &Settings) -> Decoration {
    let ps = s.pulse();
    let mood = p.mood(now, &ps);
    let (tone, note_tone) = match mood {
        Mood::Busy => (Tone::Text, Tone::Dim),
        Mood::Quiet(_) => (Tone::Dim, Tone::Dim),
        Mood::Looping { .. } => (Tone::Warn, Tone::Warn),
    };
    Decoration {
        label: None,
        label_tone: Tone::Dim,
        suffix: if s.notes { mood.suffix() } else { None },
        suffix_tone: note_tone,
        scroll: false,
        bar: None,
        spark: Some(Spark { points: points(&p.series(now, &ps)), tone }),
    }
}

fn redraw(surface: &str, l: &Live, now: u64) {
    let Some(p) = l.tabs.get(surface) else { return };
    if let Err(e) = rows::decorate(surface, Some(&decoration(p, now, &l.settings))) {
        log(LogLevel::Debug, &format!("pulse-lines: {surface}: {e}"));
    }
}

/// What an event does to a tab's pulse.
#[derive(Debug, PartialEq, Eq)]
pub enum Beat<'a> {
    Call { tool: &'a str, input: &'a str },
    Turn,
    Running(bool),
}

pub fn beat<'a>(e: &'a AgentEvent, s: &Settings) -> Option<Beat<'a>> {
    let input = e.command.as_deref().or(e.file_path.as_deref()).unwrap_or("");
    match e.kind.as_str() {
        "tool_start" => Some(Beat::Call { tool: e.tool.as_deref().unwrap_or("tool"), input }),
        "subagent_tool" if s.subagents => Some(Beat::Call { tool: e.tool.as_deref().unwrap_or("tool"), input: "" }),
        "prompt_submit" => Some(Beat::Turn),
        "stop" | "stop_failure" | "permission_seen" => Some(Beat::Running(false)),
        _ => None,
    }
}

struct Plugin;

impl Guest for Plugin {
    fn init(json: String) {
        with(|l| l.settings = serde_json::from_str(&json).unwrap_or_default());
        if let Err(e) = timer::every(TICK_SECS) {
            log(LogLevel::Warn, &format!("pulse-lines: no timer: {e}"));
        }
    }

    fn on_settings_changed(json: String) {
        with(|l| {
            l.settings = serde_json::from_str(&json).unwrap_or_default();
            let now = now_ms();
            for s in l.tabs.keys() {
                redraw(s, l, now);
            }
        });
    }

    fn on_event(event: AgentEvent) {
        let Some(surface) = event.surface.clone() else { return };
        with(|l| {
            if !event.verified && !l.settings.unverified {
                return;
            }
            let Some(b) = beat(&event, &l.settings) else { return };
            let now = now_ms();
            let ps = l.settings.pulse();
            let p = l.tabs.entry(surface.clone()).or_default();
            match b {
                Beat::Call { tool, input } => {
                    p.running = true;
                    p.call(now, tool, input, &ps);
                }
                Beat::Turn => {
                    p.running = true;
                    p.new_turn();
                }
                Beat::Running(r) => p.running = r,
            }
            redraw(&surface, l, now);
        });
    }

    fn palette_items(_mode: String, _query: String) -> Vec<PaletteItem> {
        Vec::new()
    }

    fn on_action(_action_id: String, _payload: String) {}

    fn row_menu(_surface: String) -> Vec<MenuItem> {
        Vec::new()
    }

    fn on_row_moved(_surface: String, _group: String, _before: Option<String>) {}

    fn on_theme_changed() {}

    fn on_tick(now_ms: u64) {
        with(|l| {
            let gone: Vec<String> = l.tabs.keys().filter(|s| rows::decorate(s, Some(&decoration(&l.tabs[*s], now_ms, &l.settings))).is_err()).cloned().collect();
            for s in gone {
                l.tabs.remove(&s);
            }
        });
    }

    fn on_exec(_job: u64, _outcome: Result<haunt::plugin::exec::Output, String>) {}

    fn on_away_changed(_away: bool, _away_ms: u64) {}

    fn on_hover_hold(_held: bool, _surface: String) {}

    fn on_fs_tree(_job: u64, _outcome: Result<haunt::plugin::fs_list::TreeResult, String>) {}
}

export!(Plugin);

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    fn ev(kind: &str, tool: Option<&str>, command: Option<&str>) -> AgentEvent {
        AgentEvent {
            kind: kind.into(),
            session_id: None,
            agent_id: None,
            cwd: None,
            summary: None,
            verified: true,
            surface: Some("s".into()),
            tool: tool.map(Into::into),
            command: command.map(Into::into),
            file_path: None,
            task_id: None,
            request_key: None,
        }
    }

    #[test]
    fn points_scale_to_the_busiest_minute() {
        assert_eq!(points(&[0, 2, 4, 1]), [0, 127, 255, 63]);
        assert_eq!(points(&[0, 0, 0]), [0, 0, 0], "a flat line at the bottom");
    }

    #[test]
    fn three_moods_like_the_mockup() {
        let s = Settings::default();
        let mut busy = Pulse::default();
        busy.running = true;
        for i in 0..6 {
            busy.call(100 * MIN + i * 7_000, "Read", &format!("f{i}"), &s.pulse());
        }
        let d = decoration(&busy, 100 * MIN + 50_000, &s);
        let spark = d.spark.unwrap();
        assert_eq!((spark.points.len(), spark.points[29], d.suffix), (30, 255, None));
        assert!(matches!(spark.tone, Tone::Text));

        let quiet = decoration(&busy, 106 * MIN + 50_000, &s);
        assert_eq!(quiet.suffix.as_deref(), Some("quiet 6m"));
        assert!(matches!(quiet.spark.unwrap().tone, Tone::Dim));

        let mut looping = Pulse::default();
        looping.running = true;
        for i in 0..14 {
            looping.call(i * 1000, "Bash", "pytest -x", &s.pulse());
        }
        let d = decoration(&looping, 14_000, &s);
        assert_eq!(d.suffix.as_deref(), Some("same Bash call ×14"));
        assert!(matches!((d.spark.unwrap().tone, d.suffix_tone), (Tone::Warn, Tone::Warn)));
        assert_eq!(decoration(&looping, 14_000, &Settings { notes: false, ..s }).suffix, None);
    }

    #[test]
    fn events_become_beats() {
        let s = Settings::default();
        let start = ev("tool_start", Some("Bash"), Some("npm test"));
        assert_eq!(beat(&start, &s), Some(Beat::Call { tool: "Bash", input: "npm test" }));
        assert_eq!(beat(&ev("tool_ran", Some("Bash"), None), &s), None, "each call counts once, at its start");
        assert_eq!(beat(&ev("subagent_tool", Some("Read"), None), &s), Some(Beat::Call { tool: "Read", input: "" }));
        assert_eq!(beat(&ev("subagent_tool", Some("Read"), None), &Settings { subagents: false, ..s.clone() }), None);
        assert_eq!(beat(&ev("prompt_submit", None, None), &s), Some(Beat::Turn));
        assert_eq!(beat(&ev("permission_seen", None, None), &s), Some(Beat::Running(false)), "waiting on you isn't quiet");
    }

    #[test]
    fn manifest_asks_for_what_it_uses_and_defaults_match() {
        let m = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/haunt-plugin.toml")).unwrap();
        let t: toml::Table = toml::from_str(&m).unwrap();
        let caps = t["capabilities"].as_table().unwrap();
        assert_eq!(caps.keys().collect::<Vec<_>>(), ["decorate", "event_details", "events", "timer"]);
        let defaults: serde_json::Map<String, serde_json::Value> =
            t["settings"].as_table().unwrap().iter().map(|(k, v)| (k.clone(), serde_json::to_value(&v["default"]).unwrap())).collect();
        assert_eq!(serde_json::from_value::<Settings>(serde_json::Value::Object(defaults)).unwrap(), Settings::default());
    }
}
