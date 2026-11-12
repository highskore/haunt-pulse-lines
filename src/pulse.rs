//! Tool calls per minute for one tab, and whether it looks busy, quiet or stuck in a loop. Free of
//! host calls so it tests natively.

use std::collections::VecDeque;

pub const MIN_MS: u64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Minutes in the sparkline, one point each.
    pub minutes: usize,
    /// No tool call for this long while running reads as quiet.
    pub quiet_after_min: u64,
    /// The same call this many times in a row reads as a loop.
    pub loop_after: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { minutes: 30, quiet_after_min: 5, loop_after: 8 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mood {
    Busy,
    /// Minutes since the last tool call.
    Quiet(u64),
    /// The repeated tool and how many times in a row.
    Looping { tool: String, times: u32 },
}

#[derive(Debug, Clone, Default)]
pub struct Pulse {
    calls: VecDeque<u64>,
    last_call: Option<(String, String)>,
    repeats: u32,
    last_at: Option<u64>,
    pub running: bool,
}

impl Pulse {
    /// A tool call: its name and its input (command or path; empty if unknown).
    pub fn call(&mut self, now: u64, tool: &str, input: &str, s: &Settings) {
        let key = (tool.to_owned(), input.to_owned());
        if !input.is_empty() && self.last_call.as_ref() == Some(&key) {
            self.repeats = self.repeats.saturating_add(1);
        } else {
            self.repeats = 1;
        }
        self.last_call = Some(key);
        self.last_at = Some(now);
        self.calls.push_back(now);
        let window = s.minutes as u64 * MIN_MS;
        while self.calls.front().is_some_and(|t| now.saturating_sub(*t) >= window) {
            self.calls.pop_front();
        }
    }

    /// Calls in each of the last `minutes` minutes, oldest first, ending with the current one.
    pub fn series(&self, now: u64, s: &Settings) -> Vec<u32> {
        let mut out = vec![0u32; s.minutes];
        for t in &self.calls {
            let ago = (now.saturating_sub(*t) / MIN_MS) as usize;
            if ago < s.minutes {
                out[s.minutes - 1 - ago] += 1;
            }
        }
        out
    }

    pub fn mood(&self, now: u64, s: &Settings) -> Mood {
        if let Some((tool, _)) = &self.last_call
            && self.repeats >= s.loop_after
        {
            return Mood::Looping { tool: tool.clone(), times: self.repeats };
        }
        match self.last_at {
            Some(at) if self.running && now.saturating_sub(at) >= s.quiet_after_min * MIN_MS => Mood::Quiet(now.saturating_sub(at) / MIN_MS),
            _ => Mood::Busy,
        }
    }

    /// A new turn starts the loop count over.
    pub fn new_turn(&mut self) {
        self.repeats = 0;
        self.last_call = None;
    }
}

impl Mood {
    /// The meta-line suffix: `quiet 6m`, `same Bash call ×14`, nothing while busy.
    pub fn suffix(&self) -> Option<String> {
        match self {
            Mood::Busy => None,
            Mood::Quiet(m) => Some(format!("quiet {m}m")),
            Mood::Looping { tool, times } => Some(format!("same {tool} call ×{times}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Settings = Settings { minutes: 8, quiet_after_min: 5, loop_after: 8 };

    #[test]
    fn the_window_is_thirty_minutes() {
        assert_eq!(Settings::default().minutes, 30);
    }

    #[test]
    fn calls_land_in_their_minute() {
        let mut p = Pulse::default();
        let now = 100 * MIN_MS;
        for t in [now - 8 * MIN_MS, now - 7 * MIN_MS - 1, now - MIN_MS, now - 1, now] {
            p.call(t, "Read", "", &S);
        }
        assert_eq!(p.series(now, &S), [1, 0, 0, 0, 0, 0, 1, 2], "the call 8 minutes back has left the window");
        assert_eq!(p.series(now + 8 * MIN_MS, &S), [0; 8]);
    }

    #[test]
    fn a_running_tab_without_calls_goes_quiet() {
        let mut p = Pulse { running: true, ..Pulse::default() };
        p.call(0, "Bash", "terraform plan", &S);
        assert_eq!(p.mood(4 * MIN_MS, &S), Mood::Busy);
        assert_eq!(p.mood(6 * MIN_MS + 5, &S), Mood::Quiet(6));
        assert_eq!(p.mood(6 * MIN_MS, &S).suffix().as_deref(), Some("quiet 6m"));
        p.running = false;
        assert_eq!(p.mood(60 * MIN_MS, &S), Mood::Busy, "an idle tab isn't stalled");
    }

    #[test]
    fn the_same_call_over_and_over_is_a_loop() {
        let mut p = Pulse::default();
        for i in 0..14 {
            p.call(i, "Bash", "pytest -x", &S);
        }
        assert_eq!(p.mood(14, &S).suffix().as_deref(), Some("same Bash call ×14"));
        p.call(15, "Bash", "pytest", &S);
        assert_eq!(p.mood(15, &S), Mood::Busy);
    }

    #[test]
    fn calls_without_input_never_count_as_a_loop() {
        let mut p = Pulse::default();
        for i in 0..20 {
            p.call(i, "TodoWrite", "", &S);
        }
        assert_eq!(p.mood(20, &S), Mood::Busy);
    }

    #[test]
    fn a_new_turn_resets_the_loop() {
        let mut p = Pulse::default();
        for i in 0..9 {
            p.call(i, "Read", "a.rs", &S);
        }
        p.new_turn();
        p.call(10, "Read", "a.rs", &S);
        assert_eq!(p.mood(10, &S), Mood::Busy);
    }
}
