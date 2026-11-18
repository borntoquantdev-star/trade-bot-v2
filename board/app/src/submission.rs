//! The submission card: the picture a student hands in.
//!
//! A teacher gets a folder of screenshots that look the same, carry no name
//! and come in every size. The board can save one fixed-size picture instead:
//! the student's name, whether they did the steps, and a code that ties the
//! picture to one run.
//!
//! **Everything here is plain data and arithmetic.** What goes on the card
//! (`Card`), where each thing goes (`layout`, a list of drawing instructions
//! called `Op`s) and the arithmetic in between are all written and tested
//! without a browser. Only the last step - turning the list of `Op`s into
//! pixels - needs one, and that step is a thin layer in `submission_web.rs`
//! that does nothing but replay the list on a canvas. The same list is also
//! replayed by egui for the small preview in the control card, so the preview
//! cannot drift from the picture: they are one description drawn twice.
//!
//! **Why the browser draws the saved picture and egui does not.** Thai needs
//! shaping: a tone mark or a vowel above a consonant has to be placed over it,
//! and the result depends on the font. The browser's canvas hands the text to
//! the platform's own shaper and its own Thai fonts (Leelawadee UI, Tahoma,
//! Noto Sans Thai, Thonburi), with bold and a fallback for every glyph, and
//! can hand the pixels back as a PNG (`toBlob`) with no read-back of a WebGL
//! surface. egui can shape Thai too (0.37 does, through the same engine the
//! browsers use), but only with the one regular-weight font the board carries,
//! so the preview is allowed to differ a little in weight and spacing from the
//! file. The file is what a teacher receives.
//!
//! # What the card says, and what it does not
//!
//! The biggest things on it are three badges - *wrote it myself*, *passed the
//! check*, *ran and traded N times* - because what a class wants to know is
//! whether the student did the steps. Profit and drawdown are on the card as
//! small grey text and nowhere else: nobody is being asked to win. The two
//! case-study strategies may make a card too, but it says "กรณีศึกษา" in plain
//! words and its first badge is not ticked.
//!
//! * "Wrote it myself" is decided **by the strategy's name**: anything that is
//!   not one of the two case studies counts. A student who copies
//!   `ping_pong.py` to another name gets the tick, and nothing on this
//!   machine can tell otherwise. The strategy's name is on the card so that a
//!   teacher can ask to see the file.
//! * Nothing on the card is read from a key, a private key or an account id.
//!   The card is built from the run's own row (strategy, mode, counts, equity
//!   change), the check state of its strategy, the student's name and price
//!   ticks. There is no field for a secret to travel in.
//!
//! # The code
//!
//! `XXXX-XXXX`, from [`submission_code`]. It is **not cryptographic** and does
//! not pretend to be. What it is for: two pictures with the same code came
//! from the same run (so one was copied), and a picture whose numbers were
//! changed afterwards in an image editor no longer matches a recomputation.
//! Recomputing needs the run's id, which is in the student's own
//! `logs/runs.jsonl` and deliberately not printed on the card - so a teacher
//! can only check a code with the student's folder in front of them. Anybody
//! who knows the recipe can compute a code for numbers they made up; that is
//! why it is a copy detector and not a proof.

use egui::Color32;
use serde::Serialize;

use crate::bot::Row;
use crate::control::{is_case_study, CheckState, HIDDEN_STRATEGY};
use crate::history::Run;
use crate::{theme, time};

/// The picture is always this size, whatever the screen it was made on.
pub const WIDTH: f32 = 1200.0;
pub const HEIGHT: f32 = 675.0;

/// The longest name the card takes. The server holds the same limit.
pub const NAME_MAX_CHARS: usize = 60;

/// How many of the latest runs the student can choose between.
pub const PICK_COUNT: usize = 5;

/// What the top right corner says. "v2" is the second generation of the
/// student kit, not a version number of anything in this crate.
pub const APP_TAG: &str = "TradeBot · v2";

/// The font stacks the browser draws the card with. The board does not load a
/// web font (egui's Thai font lives inside the binary where a canvas cannot
/// reach it), so the picture uses what the machine has: Leelawadee UI and
/// Tahoma ship with Windows, Noto Sans Thai with Android and most Linux, and
/// Thonburi with macOS. Each stack ends in a generic family so the browser
/// always finds something.
pub const TEXT_STACK: &str =
    "\"Leelawadee UI\", \"Noto Sans Thai\", Tahoma, Thonburi, \"Segoe UI\", system-ui, sans-serif";
pub const MONO_STACK: &str = "Consolas, \"Cascadia Mono\", \"Courier New\", monospace";

// ------------------------------------------------------------------ the name

/// Why a typed name cannot go on a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameProblem {
    TooLong,
    /// A newline, a tab or another control character. Refused instead of
    /// stripped: it is a paste gone wrong, and a name that is silently
    /// different from what was typed is worse than a message.
    ControlCharacter,
}

impl NameProblem {
    pub fn say(self) -> &'static str {
        match self {
            NameProblem::TooLong => "ชื่อยาวเกิน 60 ตัวอักษร",
            NameProblem::ControlCharacter => "ชื่อมีอักขระที่ไม่ใช่ตัวหนังสือ (ขึ้นบรรทัดใหม่หรือแท็บ)",
        }
    }
}

/// The characters that turn the text after them around. Not control
/// characters to Unicode, but a name holding one can be made to read as a
/// different name, so they are refused with the real ones. The server's rule
/// is the same.
fn is_bidi_override(ch: char) -> bool {
    matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// What a typed name becomes, or why it cannot. Trimmed; empty is allowed and
/// means "no name yet". The server applies the same rules.
pub fn clean_name(raw: &str) -> Result<String, NameProblem> {
    let name = raw.trim();
    if name
        .chars()
        .any(|ch| ch.is_control() || is_bidi_override(ch))
    {
        return Err(NameProblem::ControlCharacter);
    }
    if name.chars().count() > NAME_MAX_CHARS {
        return Err(NameProblem::TooLong);
    }
    Ok(name.to_owned())
}

// ------------------------------------------------------------------ the run

/// What the bot ran on. The card has only these two words for it, and only
/// these two can make a card: a run against a real market is not what a
/// classroom exercise is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardMode {
    Paper,
    Testnet,
}

impl CardMode {
    /// The word on the card.
    pub fn word(self) -> &'static str {
        match self {
            CardMode::Paper => "กระดาษ",
            CardMode::Testnet => "testnet",
        }
    }

    /// The word in the code's recipe. Plain ASCII and never reworded: changing
    /// it would change every code.
    fn wire(self) -> &'static str {
        match self {
            CardMode::Paper => "paper",
            CardMode::Testnet => "testnet",
        }
    }
}

/// The card's word for the `mode` the bot wrote into `runs.jsonl`
/// (`paper`, `grvt testnet`, ...), or `None` for a run that is neither.
pub fn card_mode(run_mode: &str) -> Option<CardMode> {
    let mode = run_mode.trim().to_lowercase();
    if mode == "paper" {
        Some(CardMode::Paper)
    } else if mode.contains("testnet") && !mode.contains("paper") {
        Some(CardMode::Testnet)
    } else {
        None
    }
}

/// The runs a card can be made of, newest first, at most [`PICK_COUNT`].
///
/// Left out, and why:
/// * a run **inferred** from silences in the audit trail: it has no
///   strategy to put on the card, and no id the bot wrote;
/// * a run with no strategy name, and `random`, the plumbing test that is
///   hidden everywhere else on the board;
/// * a run that was neither paper nor testnet.
pub fn eligible(runs: &[Run]) -> Vec<&Run> {
    let mut found: Vec<(i64, &Run)> = runs
        .iter()
        .filter(|run| {
            !run.inferred
                && !run.id.is_empty()
                && !run.strategy.is_empty()
                && run.strategy != HIDDEN_STRATEGY
                && card_mode(&run.mode).is_some()
        })
        .filter_map(|run| Some((time::utc_seconds(&run.started_at)?, run)))
        .collect();
    // The server already sends newest first; sorting again costs nothing and
    // means the order here does not depend on that.
    found.sort_by_key(|(start, _)| std::cmp::Reverse(*start));
    found
        .into_iter()
        .map(|(_, run)| run)
        .take(PICK_COUNT)
        .collect()
}

/// How many trades a run made.
///
/// A trade is opened by one order and closed by one exit, so the two counts
/// describe the **same** trades from either end and adding them would count
/// every round trip twice. The larger of the two is used because each end can
/// be missing for a reason that is not a missing trade: a run still going has
/// opened one it has not closed yet (`orders` is ahead), and a run that
/// started with a position left over from the last one closes a trade it did
/// not open (`exits` is ahead). Either way it is a trade this run took part in.
pub fn trade_count(orders: u64, exits: u64) -> u64 {
    orders.max(exits)
}

/// How many trades `run` made: from its closing line if it has one, and from
/// what the audit trail shows inside the run if it has not (it is still going,
/// or it was killed). A run with no closing line has no `orders` of its own,
/// and reading that as "no trades" would put a grey badge on a student who is
/// in the middle of trading.
pub fn trades_of(run: &Run) -> u64 {
    if run.finished {
        trade_count(run.orders, run.exits)
    } else {
        trade_count(run.audit_orders, run.audit_exits)
    }
}

/// The change of equity over the run as tenths of a percent of where it
/// started, or `None` when it cannot be worked out (a run with no closing
/// line has no change; a start of zero has no percentage). Rounded the way
/// the card prints it, because the code is made from what is printed.
pub fn pl_tenths(run: &Run) -> Option<i64> {
    let change = run.change?;
    let start: f64 = run.first_equity.trim().parse().ok()?;
    if !(start.is_finite() && start > 0.0 && change.is_finite()) {
        return None;
    }
    Some(tenths(change / start * 100.0))
}

/// The deepest fall in equity during the run, as tenths of a percent, or
/// `None` when the server could not measure one.
pub fn dd_tenths(run: &Run) -> Option<i64> {
    let pct = run.max_drawdown_pct?;
    pct.is_finite().then(|| tenths(pct.abs()))
}

fn tenths(percent: f64) -> i64 {
    (percent * 10.0).round() as i64
}

/// `P/L +0.4%`, `P/L -0.4%`, `P/L 0.0%` or `P/L วัดไม่ได้`.
///
/// "Could not be measured" is said in words and is never `0.0%`: those are
/// different sentences, and one of them is false. The minus is the ASCII
/// hyphen so that it cannot be a missing glyph (see `fonts.rs`).
pub fn format_pl(tenths: Option<i64>) -> String {
    match tenths {
        None => "P/L วัดไม่ได้".to_owned(),
        Some(0) => "P/L 0.0%".to_owned(),
        Some(value) => format!(
            "P/L {}{}.{}%",
            if value < 0 { '-' } else { '+' },
            value.abs() / 10,
            value.abs() % 10
        ),
    }
}

/// `DD 1.1%` or `DD วัดไม่ได้`.
pub fn format_dd(tenths: Option<i64>) -> String {
    match tenths {
        None => "DD วัดไม่ได้".to_owned(),
        Some(value) => format!("DD {}.{}%", value.abs() / 10, value.abs() % 10),
    }
}

/// `45 นาที`, `1 ชม. 5 นาที`, `30 วินาที`.
pub fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{seconds} วินาที");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes} นาที");
    }
    match minutes % 60 {
        0 => format!("{} ชม.", minutes / 60),
        rest => format!("{} ชม. {rest} นาที", minutes / 60),
    }
}

/// How long the run lasted, in seconds. A run that is going lasts until
/// `now`; one that finished lasts until its closing line; one that was killed
/// has no end and no length - guessing one would put a made-up duration on a
/// card.
fn duration_of(run: &Run, status: RunStatus, now_unix: Option<f64>) -> Option<u64> {
    let start = time::utc_seconds(&run.started_at)?;
    let end = match status {
        RunStatus::Finished => time::utc_seconds(&run.ended_at)?,
        RunStatus::Running => now_unix?.floor() as i64,
        RunStatus::Unfinished => return None,
    };
    u64::try_from(end - start).ok()
}

/// Where a run is in its life, as the card says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// "กำลังรัน": the bot is going and this is its run.
    Running,
    Finished,
    /// "ยังไม่จบ": no closing line and nothing running - the bot was killed.
    Unfinished,
}

// ------------------------------------------------------------------ the code

/// What goes into [`submission_code`], and so what a changed picture is
/// compared against.
#[derive(Debug, Clone, Copy)]
pub struct CodeInput<'a> {
    /// The run's id from `logs/runs.jsonl`. Not printed on the card.
    pub run_id: &'a str,
    pub name: &'a str,
    pub strategy: &'a str,
    pub mode: CardMode,
    pub trades: u64,
    pub pl_tenths: Option<i64>,
    pub dd_tenths: Option<i64>,
}

/// The 32 characters a code is made of: capitals and digits with `0 O 1 I`
/// left out, because a code is read aloud and typed from a photograph.
const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Eight characters, `XXXX-XXXX`, worked out from the run and what the card
/// says about it.
///
/// The recipe: join the fields with a separator that cannot occur in any of
/// them, hash the bytes with 64-bit FNV-1a, stir the result with the
/// SplitMix64 finaliser (so that a one-digit change in the input changes every
/// character, not just the last), and read forty of the bits as eight
/// five-bit characters. Written out here rather than borrowed from the
/// standard library's hasher because the standard library promises nothing
/// about its output between versions, and a code that changes when the
/// compiler does is a code nobody can check next term.
///
/// The percentages go in as the **tenths printed on the card**, and an
/// unmeasured one as a dash, so that what is recomputed is what is read off
/// the picture. The drawdown goes in as well as the profit: both are printed
/// on the card, so neither can be edited in an image editor for free.
pub fn submission_code(input: &CodeInput) -> String {
    const SEPARATOR: char = '\u{1f}';
    let figure = |value: Option<i64>| value.map_or_else(|| "-".to_owned(), |v| v.to_string());
    let text = [
        "board-submission-v1",
        input.run_id,
        input.name,
        input.strategy,
        input.mode.wire(),
        &input.trades.to_string(),
        &figure(input.pl_tenths),
        &figure(input.dd_tenths),
    ]
    .join(&SEPARATOR.to_string());

    // FNV-1a, 64 bit.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // SplitMix64 finaliser.
    hash ^= hash >> 30;
    hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash ^= hash >> 27;
    hash = hash.wrapping_mul(0x94d0_49bb_1331_11eb);
    hash ^= hash >> 31;

    let mut code = String::with_capacity(9);
    for index in 0..8 {
        if index == 4 {
            code.push('-');
        }
        let five_bits = (hash >> (59 - 5 * index)) & 0x1f;
        code.push(ALPHABET[five_bits as usize] as char);
    }
    code
}

// ------------------------------------------------------------------ the chart

/// What the small chart marks on the price line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    EntryBuy,
    EntrySell,
    Exit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartMark {
    pub unix: f64,
    pub price: f64,
    pub kind: MarkKind,
}

/// The price ticks of one run, and where it went in and out.
#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    pub symbol: String,
    /// (Unix seconds, price), oldest first.
    pub points: Vec<(f64, f64)>,
    pub marks: Vec<ChartMark>,
}

/// More points than this make no difference on a line a thousand pixels wide.
const MAX_POINTS: usize = 400;

/// The price line of `run`, from the ticks the board already holds
/// (`/api/audit`, the newest 500).
///
/// Nothing new is fetched: if the run is older than those 500 ticks there is
/// nothing to draw and the answer is `None` - the card then says so instead
/// of drawing a different stretch of time under this run's name. A run that is
/// only partly inside the 500 draws the part that is, and its time labels
/// show which part, because they are read off the points and not off the
/// run's own start and end.
///
/// The marks follow `marks.rs`: an entry is an order the exchange answered, an
/// exit is a position the bot let go of (and not one it only asked to).
pub fn chart_of(run: &Run, rows: &[Row]) -> Option<Chart> {
    let inside: Vec<&Row> = rows
        .iter()
        .filter(|row| run.symbol.is_empty() || row.symbol == run.symbol)
        .filter(|row| run.covers(&row.at))
        .collect();

    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut marks: Vec<ChartMark> = Vec::new();
    for row in &inside {
        let (Some(unix), Some(price)) = (time::unix_of(&row.at), row.price_value()) else {
            continue;
        };
        points.push((unix, price));
        if let (Some(order), Some(_)) = (&row.order, &row.receipt) {
            let at = order.limit_price.trim().parse::<f64>().unwrap_or(price);
            marks.push(ChartMark {
                unix,
                price: at,
                kind: if order.side == "buy" {
                    MarkKind::EntryBuy
                } else {
                    MarkKind::EntrySell
                },
            });
        } else if let Some(exit) = &row.exit {
            if !exit.broker_order_id.is_empty() || exit.simulated {
                marks.push(ChartMark {
                    unix,
                    price,
                    kind: MarkKind::Exit,
                });
            }
        }
    }
    if points.len() < 2 || points.last()?.0 <= points.first()?.0 {
        return None;
    }
    if points.len() > MAX_POINTS {
        let last = points.len() - 1;
        points = (0..MAX_POINTS)
            .map(|n| points[n * last / (MAX_POINTS - 1)])
            .collect();
    }
    Some(Chart {
        symbol: inside
            .first()
            .map(|row| row.symbol.clone())
            .unwrap_or_default(),
        points,
        marks,
    })
}

// ------------------------------------------------------------------ the card

/// Everything the card says, worked out once.
#[derive(Debug, Clone)]
pub struct Card {
    pub name: String,
    pub strategy: String,
    pub mode: CardMode,
    pub status: RunStatus,
    /// The student wrote the strategy: it is not one of the two case studies.
    pub own: bool,
    pub case_study: bool,
    /// Did the strategy pass its check. A testnet run since the bot's gate
    /// went in passed by being there ([`check_of`]); otherwise what the
    /// check says now, not what it said that day.
    pub check: CheckState,
    pub trades: u64,
    pub duration_secs: Option<u64>,
    pub pl_tenths: Option<i64>,
    pub dd_tenths: Option<i64>,
    /// When the run started, as the card prints it (`17 พ.ย. 2569 20:14`).
    pub started: String,
    pub code: String,
    pub chart: Option<Chart>,
}

/// What a card is made from.
pub struct Inputs<'a> {
    /// Already through [`clean_name`] and not empty.
    pub name: &'a str,
    pub run: &'a Run,
    /// The check state of `run.strategy`, from the bot's own list.
    pub check: CheckState,
    /// Is the bot running right now, and is this its run?
    pub running: bool,
    pub rows: &'a [Row],
    pub now_unix: Option<f64>,
}

/// Make the card, or `None` for a run that is not paper or testnet.
pub fn build(input: &Inputs) -> Option<Card> {
    let run = input.run;
    let mode = card_mode(&run.mode)?;
    let status = if input.running && !run.finished {
        RunStatus::Running
    } else if run.finished {
        RunStatus::Finished
    } else {
        RunStatus::Unfinished
    };
    let case_study = is_case_study(&run.strategy);
    let trades = trades_of(run);
    let (pl, dd) = (pl_tenths(run), dd_tenths(run));
    let code = submission_code(&CodeInput {
        run_id: &run.id,
        name: input.name,
        strategy: &run.strategy,
        mode,
        trades,
        pl_tenths: pl,
        dd_tenths: dd,
    });
    Some(Card {
        name: input.name.to_owned(),
        strategy: run.strategy.clone(),
        mode,
        status,
        own: !case_study,
        case_study,
        check: check_of(run, mode, input.check),
        trades,
        duration_secs: duration_of(run, status, input.now_unix),
        pl_tenths: pl,
        dd_tenths: dd,
        started: time::thai_stamp(&run.started_at),
        code,
        chart: chart_of(run, input.rows),
    })
}

/// When `--live` started refusing to trade until `--check` had passed on the
/// same file and market (bot commit 4b50a04, 16 Nov 2026 22:30 Thai time).
const LIVE_GATE_SINCE: &str = "2026-11-16T15:30:23+00:00";

/// The check badge of a run.
///
/// A testnet run that started once the gate was in could only start because
/// the check had passed, so it is passed even if the file was edited later
/// and the check now says otherwise. A paper run is not gated, and a testnet
/// run from before the gate proves nothing: both take what the check says now.
pub fn check_of(run: &Run, mode: CardMode, now: CheckState) -> CheckState {
    let gated = match (
        time::utc_seconds(&run.started_at),
        time::utc_seconds(LIVE_GATE_SINCE),
    ) {
        (Some(started), Some(gate)) => started >= gate,
        _ => false,
    };
    if mode == CardMode::Testnet && gated {
        CheckState::Passed
    } else {
        now
    }
}

/// Which badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeKey {
    Own,
    Passed,
    Trades,
}

/// One of the three big badges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    pub key: BadgeKey,
    pub label: String,
    pub done: bool,
    /// Under the label, only on a badge that is not done: why it is not.
    /// A grey badge with no reason invites the wrong guess.
    pub why: Option<&'static str>,
}

impl Card {
    /// The three badges, in the order they are drawn.
    pub fn badges(&self) -> [Badge; 3] {
        let passed = self.check == CheckState::Passed;
        [
            Badge {
                key: BadgeKey::Own,
                label: "เขียนเอง".to_owned(),
                done: self.own,
                why: (!self.own).then_some("กรณีศึกษา"),
            },
            Badge {
                key: BadgeKey::Passed,
                label: "ผ่านด่าน".to_owned(),
                done: passed,
                why: match self.check {
                    CheckState::Passed => None,
                    CheckState::NotPassed => Some("ยังไม่ผ่าน"),
                    CheckState::Unknown => Some("ยังไม่ทราบผล"),
                },
            },
            Badge {
                key: BadgeKey::Trades,
                label: format!("{} ไม้", self.trades),
                done: self.trades > 0,
                why: (self.trades == 0).then_some("ยังไม่มีไม้"),
            },
        ]
    }

    /// `กลยุทธ์: somchai_trend (เขียนเอง)` or `กลยุทธ์: ping_pong (กรณีศึกษา)`.
    pub fn strategy_line(&self) -> String {
        let kind = if self.case_study {
            "กรณีศึกษา"
        } else {
            "เขียนเอง"
        };
        format!("กลยุทธ์: {} ({kind})", self.strategy)
    }

    /// `กระดาษ · 45 นาที · P/L -0.4% · DD 1.1%`. The length is left out when
    /// it is not known.
    pub fn meta_line(&self) -> String {
        let mut parts = vec![self.mode.word().to_owned()];
        if let Some(seconds) = self.duration_secs {
            parts.push(format_duration(seconds));
        }
        parts.push(format_pl(self.pl_tenths));
        parts.push(format_dd(self.dd_tenths));
        parts.join(" · ")
    }

    /// The small tag next to the strategy when the run is not a plain
    /// finished one.
    pub fn status_tag(&self) -> Option<&'static str> {
        match self.status {
            RunStatus::Running => Some("กำลังรัน"),
            RunStatus::Unfinished => Some("ยังไม่จบ"),
            RunStatus::Finished => None,
        }
    }
}

// ------------------------------------------------------------------ the picture

/// A colour with its alpha, as a browser spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub fn of(colour: Color32) -> Rgba {
        let [r, g, b, a] = colour.to_srgba_unmultiplied();
        Rgba(r, g, b, a)
    }

    /// The same colour with a different opacity (0 to 1).
    pub fn alpha(self, opacity: f32) -> Rgba {
        Rgba(
            self.0,
            self.1,
            self.2,
            (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
        )
    }

    /// `rgba(95, 190, 134, 0.5)`
    pub fn css(self) -> String {
        format!(
            "rgba({}, {}, {}, {:.3})",
            self.0,
            self.1,
            self.2,
            f32::from(self.3) / 255.0
        )
    }

    pub fn color32(self) -> Color32 {
        Color32::from_rgba_unmultiplied(self.0, self.1, self.2, self.3)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// One drawing instruction. A card is a list of these, drawn in order.
///
/// Text is positioned by the **middle of its line** (`y`) and by its left,
/// centre or right edge (`x`), so nothing in the layout needs to know how wide
/// a piece of text will turn out: the width depends on the font the machine
/// has, and the layout is not allowed to depend on that.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Op {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        fill: Option<Rgba>,
        stroke: Option<(Rgba, f32)>,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        size: f32,
        bold: bool,
        mono: bool,
        align: Align,
        colour: Rgba,
        /// The widest the text may be. A drawer that finds it wider shrinks
        /// the font (never below 60%), and squeezes what is still too wide.
        fit: Option<f32>,
    },
    /// An open line through the points, with round ends.
    Line {
        points: Vec<(f32, f32)>,
        colour: Rgba,
        width: f32,
    },
    Circle {
        x: f32,
        y: f32,
        r: f32,
        fill: Option<Rgba>,
        stroke: Option<(Rgba, f32)>,
    },
    /// A filled shape through the points.
    Polygon { points: Vec<(f32, f32)>, fill: Rgba },
}

// The palette, as the board's theme has it. Named here once so the layout
// reads as design and not as a table of numbers.
fn canvas() -> Rgba {
    Rgba::of(theme::CANVAS)
}
fn surface() -> Rgba {
    Rgba::of(theme::SURFACE)
}
fn head() -> Rgba {
    Rgba::of(theme::HEAD)
}
fn text() -> Rgba {
    Rgba::of(theme::TEXT)
}
fn muted() -> Rgba {
    Rgba::of(theme::MUTED)
}
fn dim() -> Rgba {
    Rgba::of(theme::DIM)
}
fn faint() -> Rgba {
    Rgba::of(theme::FAINT)
}
fn ok() -> Rgba {
    Rgba::of(theme::OK)
}
fn warn() -> Rgba {
    Rgba::of(theme::WARN)
}
fn bad() -> Rgba {
    Rgba::of(theme::BAD)
}

/// The side margin: everything sits inside it.
pub const MARGIN: f32 = 60.0;
/// The three badges and the gaps between them fill the width inside the margin.
const BADGE_GAP: f32 = 24.0;
pub const BADGE_Y: f32 = 160.0;
pub const BADGE_H: f32 = 124.0;
const BADGE_W: f32 = (WIDTH - 2.0 * MARGIN - 2.0 * BADGE_GAP) / 3.0;
/// The chart panel.
pub const CHART_Y: f32 = 308.0;
pub const CHART_H: f32 = 204.0;
const META_Y: f32 = 544.0;
const RULE_Y: f32 = 578.0;
const FOOT_Y: f32 = 622.0;

/// Where badge `index` (0, 1, 2) sits: `(x, y, w, h)`.
pub fn badge_rect(index: usize) -> (f32, f32, f32, f32) {
    (
        MARGIN + index as f32 * (BADGE_W + BADGE_GAP),
        BADGE_Y,
        BADGE_W,
        BADGE_H,
    )
}

/// The plot area inside the chart panel: `(x0, y0, x1, y1)`.
pub fn plot_area() -> (f32, f32, f32, f32) {
    (
        MARGIN + 28.0,
        CHART_Y + 40.0,
        WIDTH - MARGIN - 28.0,
        CHART_Y + CHART_H - 44.0,
    )
}

fn label(x: f32, y: f32, words: &str, size: f32, align: Align, colour: Rgba) -> Op {
    Op::Text {
        x,
        y,
        text: words.to_owned(),
        size,
        bold: false,
        mono: false,
        align,
        colour,
        fit: None,
    }
}

/// The card as a list of drawing instructions, 1200 by 675.
pub fn layout(card: &Card) -> Vec<Op> {
    let mut ops: Vec<Op> = Vec::new();

    // The page, and the panel on it.
    ops.push(Op::Rect {
        x: 0.0,
        y: 0.0,
        w: WIDTH,
        h: HEIGHT,
        radius: 0.0,
        fill: Some(canvas()),
        stroke: None,
    });
    ops.push(Op::Rect {
        x: 14.0,
        y: 14.0,
        w: WIDTH - 28.0,
        h: HEIGHT - 28.0,
        radius: 28.0,
        fill: Some(surface()),
        stroke: Some((text().alpha(0.10), 2.0)),
    });

    // Who, and what they ran.
    ops.push(Op::Text {
        x: MARGIN,
        y: 76.0,
        text: card.name.clone(),
        size: 40.0,
        bold: true,
        mono: false,
        align: Align::Left,
        colour: text(),
        fit: Some(WIDTH - 2.0 * MARGIN - 300.0),
    });
    ops.push(Op::Text {
        x: WIDTH - MARGIN,
        y: 76.0,
        text: APP_TAG.to_owned(),
        size: 24.0,
        bold: false,
        mono: false,
        align: Align::Right,
        colour: dim(),
        fit: None,
    });
    ops.push(Op::Text {
        x: MARGIN,
        y: 122.0,
        text: card.strategy_line(),
        size: 26.0,
        bold: false,
        mono: false,
        align: Align::Left,
        // A case study is said in amber: it must not read like the student's
        // own work from across a room.
        colour: if card.case_study { warn() } else { muted() },
        fit: Some(WIDTH - 2.0 * MARGIN - 190.0),
    });
    if let Some(tag) = card.status_tag() {
        let colour = if card.status == RunStatus::Running {
            warn()
        } else {
            dim()
        };
        let (w, h) = (160.0, 38.0);
        ops.push(Op::Rect {
            x: WIDTH - MARGIN - w,
            y: 122.0 - h / 2.0,
            w,
            h,
            radius: h / 2.0,
            fill: Some(colour.alpha(0.14)),
            stroke: Some((colour.alpha(0.8), 2.0)),
        });
        ops.push(label(
            WIDTH - MARGIN - w / 2.0,
            122.0,
            tag,
            22.0,
            Align::Center,
            colour,
        ));
    }

    // The three badges: the largest things on the card.
    for (index, badge) in card.badges().iter().enumerate() {
        badge_ops(&mut ops, index, badge);
    }

    chart_ops(&mut ops, card.chart.as_ref());

    // The small print.
    ops.push(label(
        MARGIN,
        META_Y,
        &card.meta_line(),
        26.0,
        Align::Left,
        muted(),
    ));
    ops.push(Op::Line {
        points: vec![(MARGIN, RULE_Y), (WIDTH - MARGIN, RULE_Y)],
        colour: text().alpha(0.10),
        width: 2.0,
    });
    if !card.started.is_empty() {
        ops.push(label(
            MARGIN,
            FOOT_Y,
            &format!("รันเมื่อ {}", card.started),
            22.0,
            Align::Left,
            dim(),
        ));
    }
    ops.push(Op::Text {
        x: WIDTH - MARGIN,
        y: FOOT_Y,
        text: format!("รหัส {}", card.code),
        size: 24.0,
        bold: true,
        mono: true,
        align: Align::Right,
        colour: muted(),
        fit: None,
    });
    ops
}

/// One badge: a rounded box, a round tick (or an empty ring), the word, and
/// when it is not done the reason.
///
/// The tick is **drawn**, not typed. The check-mark character is in some
/// fonts and not in others, and a badge that shows an empty square where its
/// tick should be would say the opposite of what it means.
fn badge_ops(ops: &mut Vec<Op>, index: usize, badge: &Badge) {
    let (x, y, w, h) = badge_rect(index);
    let centre = y + h / 2.0;
    let (accent, fill) = if badge.done {
        (ok(), ok().alpha(0.16))
    } else {
        (faint(), head())
    };
    ops.push(Op::Rect {
        x,
        y,
        w,
        h,
        radius: 26.0,
        fill: Some(fill),
        stroke: Some((accent.alpha(if badge.done { 0.75 } else { 0.6 }), 2.0)),
    });

    let (icon_x, icon_r) = (x + 62.0, 28.0);
    if badge.done {
        ops.push(Op::Circle {
            x: icon_x,
            y: centre,
            r: icon_r,
            fill: Some(accent),
            stroke: None,
        });
        ops.push(Op::Line {
            points: vec![
                (icon_x - 12.0, centre + 1.0),
                (icon_x - 4.0, centre + 10.0),
                (icon_x + 13.0, centre - 9.0),
            ],
            colour: canvas(),
            width: 6.0,
        });
    } else {
        ops.push(Op::Circle {
            x: icon_x,
            y: centre,
            r: icon_r - 1.5,
            fill: None,
            stroke: Some((accent, 3.0)),
        });
        ops.push(Op::Line {
            points: vec![(icon_x - 10.0, centre), (icon_x + 10.0, centre)],
            colour: accent,
            width: 4.0,
        });
    }

    let text_x = x + 112.0;
    let room = w - 112.0 - 22.0;
    let (label_y, label_colour) = match badge.why {
        Some(_) => (centre - 17.0, dim()),
        None => (centre, text()),
    };
    ops.push(Op::Text {
        x: text_x,
        y: label_y,
        text: badge.label.clone(),
        size: 44.0,
        bold: true,
        mono: false,
        align: Align::Left,
        colour: label_colour,
        fit: Some(room),
    });
    if let Some(why) = badge.why {
        ops.push(Op::Text {
            x: text_x,
            y: centre + 27.0,
            text: why.to_owned(),
            size: 22.0,
            bold: false,
            mono: false,
            align: Align::Left,
            colour: dim(),
            fit: Some(room),
        });
    }
}

/// Map a price and a time onto the plot area.
struct Scale {
    t0: f64,
    t1: f64,
    p0: f64,
    p1: f64,
    area: (f32, f32, f32, f32),
}

impl Scale {
    /// The window that holds every point and every mark, with a tenth of room
    /// above and below. A flat line is put in the middle rather than divided
    /// by zero.
    fn of(chart: &Chart) -> Scale {
        let prices = chart
            .points
            .iter()
            .map(|point| point.1)
            .chain(chart.marks.iter().map(|mark| mark.price));
        let (mut low, mut high) = (f64::MAX, f64::MIN);
        for price in prices {
            low = low.min(price);
            high = high.max(price);
        }
        let pad = if high > low {
            (high - low) * 0.1
        } else {
            (high.abs() * 0.001).max(1e-9)
        };
        let times = chart
            .points
            .iter()
            .map(|point| point.0)
            .chain(chart.marks.iter().map(|mark| mark.unix));
        let (mut first, mut last) = (f64::MAX, f64::MIN);
        for at in times {
            first = first.min(at);
            last = last.max(at);
        }
        Scale {
            t0: first,
            t1: last.max(first + 1e-6),
            p0: low - pad,
            p1: high + pad,
            area: plot_area(),
        }
    }

    fn at(&self, unix: f64, price: f64) -> (f32, f32) {
        let (x0, y0, x1, y1) = self.area;
        let across = ((unix - self.t0) / (self.t1 - self.t0)) as f32;
        let up = ((price - self.p0) / (self.p1 - self.p0)) as f32;
        (x0 + across * (x1 - x0), y1 - up * (y1 - y0))
    }
}

fn chart_ops(ops: &mut Vec<Op>, chart: Option<&Chart>) {
    ops.push(Op::Rect {
        x: MARGIN,
        y: CHART_Y,
        w: WIDTH - 2.0 * MARGIN,
        h: CHART_H,
        radius: 20.0,
        fill: Some(canvas().alpha(0.55)),
        stroke: Some((text().alpha(0.08), 2.0)),
    });
    let Some(chart) = chart else {
        ops.push(label(
            WIDTH / 2.0,
            CHART_Y + CHART_H / 2.0,
            "ไม่มีข้อมูลราคาของรอบนี้ในบอร์ด",
            22.0,
            Align::Center,
            faint(),
        ));
        return;
    };

    let scale = Scale::of(chart);
    let (x0, y0, x1, y1) = scale.area;
    // Three hairlines to read the height against.
    for step in 0..3 {
        let y = y0 + (y1 - y0) * step as f32 / 2.0;
        ops.push(Op::Line {
            points: vec![(x0, y), (x1, y)],
            colour: text().alpha(0.07),
            width: 1.0,
        });
    }
    ops.push(Op::Line {
        points: chart
            .points
            .iter()
            .map(|(unix, price)| scale.at(*unix, *price))
            .collect(),
        colour: muted().alpha(0.92),
        width: 3.0,
    });
    for mark in &chart.marks {
        let (x, y) = scale.at(mark.unix, mark.price);
        match mark.kind {
            MarkKind::EntryBuy => ops.push(Op::Polygon {
                points: vec![(x, y - 11.0), (x - 10.0, y + 8.0), (x + 10.0, y + 8.0)],
                fill: ok(),
            }),
            MarkKind::EntrySell => ops.push(Op::Polygon {
                points: vec![(x, y + 11.0), (x - 10.0, y - 8.0), (x + 10.0, y - 8.0)],
                fill: bad(),
            }),
            MarkKind::Exit => ops.push(Op::Circle {
                x,
                y,
                r: 7.0,
                fill: Some(surface()),
                stroke: Some((text(), 3.0)),
            }),
        }
    }

    // Under the line: what it is, and which stretch of time it covers - read
    // off the points, so a run that only partly reached the board says so.
    ops.push(label(
        x0,
        CHART_Y + 24.0,
        &format!("ราคา {}", chart.symbol),
        20.0,
        Align::Left,
        faint(),
    ));
    let clock = |unix: f64| time::hhmm(&time::clock_iso(unix));
    let (first, last) = (chart.points[0].0, chart.points[chart.points.len() - 1].0);
    ops.push(label(
        x0,
        CHART_Y + CHART_H - 22.0,
        &clock(first),
        20.0,
        Align::Left,
        faint(),
    ));
    ops.push(label(
        x1,
        CHART_Y + CHART_H - 22.0,
        &clock(last),
        20.0,
        Align::Right,
        faint(),
    ));
    // The legend, drawn from the same shapes it explains.
    let legend_y = CHART_Y + 24.0;
    // An entry is a triangle: up for a buy, down for a sell.
    ops.push(Op::Polygon {
        points: vec![
            (x1 - 196.0, legend_y - 9.0),
            (x1 - 205.0, legend_y + 7.0),
            (x1 - 187.0, legend_y + 7.0),
        ],
        fill: ok(),
    });
    ops.push(Op::Polygon {
        points: vec![
            (x1 - 170.0, legend_y + 9.0),
            (x1 - 179.0, legend_y - 7.0),
            (x1 - 161.0, legend_y - 7.0),
        ],
        fill: bad(),
    });
    ops.push(label(
        x1 - 148.0,
        legend_y,
        "เข้า",
        20.0,
        Align::Left,
        faint(),
    ));
    ops.push(Op::Circle {
        x: x1 - 62.0,
        y: legend_y,
        r: 6.0,
        fill: Some(surface()),
        stroke: Some((text(), 2.5)),
    });
    ops.push(label(
        x1 - 48.0,
        legend_y,
        "ออก",
        20.0,
        Align::Left,
        faint(),
    ));
}

// ------------------------------------------------------------------ the file

/// `ส่งงาน_สมชาย_ใจดี_20261117-2014.png`
///
/// The name has the characters Windows forbids in a file name taken out, runs
/// of spaces turned into one underscore, and is cut at 40 characters. A name
/// with nothing left after that - or none - leaves it out, and what is left
/// still sorts by time.
pub fn file_name(name: &str, now_unix: f64) -> String {
    let mut kept = String::new();
    let mut pending_gap = false;
    for ch in name.chars() {
        if ch.is_whitespace() {
            pending_gap = !kept.is_empty();
        } else if ch.is_control() || is_bidi_override(ch) || "\\/:*?\"<>|".contains(ch) {
            // dropped
        } else {
            if pending_gap {
                kept.push('_');
                pending_gap = false;
            }
            kept.push(ch);
        }
    }
    let kept: String = kept.chars().take(40).collect();
    let kept = kept.trim_matches(|c| c == '.' || c == '_').to_owned();
    let stamp = time::file_stamp(now_unix);
    if kept.is_empty() {
        format!("ส่งงาน_{stamp}.png")
    } else {
        format!("ส่งงาน_{kept}_{stamp}.png")
    }
}

// ------------------------------------------------------------------ saving

/// What the browser reports while a picture is being saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveEvent {
    /// The file was handed to the browser's downloads.
    Downloaded,
    /// It is also on the clipboard.
    Copied,
    /// It is not, and why (an old browser, a page without permission).
    NotCopied(String),
    /// Nothing was saved.
    Failed(String),
}

/// Where saving is, for the line under the button.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SaveStatus {
    #[default]
    Idle,
    Working,
    Saved {
        file: String,
        copy: CopyState,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyState {
    Pending,
    Done,
    Unavailable,
}

impl SaveStatus {
    pub fn apply(&mut self, event: SaveEvent, file: &str) {
        *self = match event {
            SaveEvent::Downloaded => SaveStatus::Saved {
                file: file.to_owned(),
                copy: CopyState::Pending,
            },
            SaveEvent::Copied => SaveStatus::Saved {
                file: file.to_owned(),
                copy: CopyState::Done,
            },
            SaveEvent::NotCopied(_) => SaveStatus::Saved {
                file: file.to_owned(),
                copy: CopyState::Unavailable,
            },
            SaveEvent::Failed(why) => SaveStatus::Failed(why),
        };
    }

    /// The sentence and whether it is good news. `None` when idle.
    pub fn line(&self) -> Option<(String, bool)> {
        match self {
            SaveStatus::Idle => None,
            SaveStatus::Working => Some(("กำลังสร้างรูป...".to_owned(), true)),
            SaveStatus::Saved { copy, .. } => Some(match copy {
                CopyState::Pending => ("บันทึกแล้ว · กำลังคัดลอก...".to_owned(), true),
                CopyState::Done => ("บันทึกแล้ว · คัดลอกไว้แล้ว วางใน LINE ได้เลย".to_owned(), true),
                CopyState::Unavailable => (
                    "บันทึกแล้ว · คัดลอกอัตโนมัติไม่ได้ ใช้ไฟล์ในโฟลเดอร์ดาวน์โหลดส่งแทน".to_owned(),
                    true,
                ),
            }),
            SaveStatus::Failed(why) => Some((format!("บันทึกรูปไม่สำเร็จ · {why}"), false)),
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self, SaveStatus::Working)
    }
}

/// Draw the card in the browser and save it: download, and try the clipboard.
/// `emit` hears what happened, as it happens.
#[cfg(target_arch = "wasm32")]
pub fn save_png(ops: Vec<Op>, file_name: String, emit: impl Fn(SaveEvent) + 'static) {
    crate::submission_web::save_png(ops, file_name, emit);
}

/// The desktop window has no canvas to draw on, so it cannot make the picture.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_png(_ops: Vec<Op>, _file_name: String, emit: impl Fn(SaveEvent) + 'static) {
    emit(SaveEvent::Failed(
        "บันทึกรูปได้เฉพาะในเบราว์เซอร์ · เปิดบอร์ดด้วย TradeBot.exe".to_owned(),
    ));
}

// ------------------------------------------------------------------ the preview

/// Replay the card inside `rect` in egui, scaled down. Used for the small
/// picture beside the button: the same list the browser draws, so what is
/// shown is what is saved - apart from the text shaping egui cannot do.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, ops: &[Op]) {
    use egui::{Align2, FontId, Pos2, Shape, Stroke, StrokeKind};
    let scale = rect.width() / WIDTH;
    let at = |x: f32, y: f32| Pos2::new(rect.min.x + x * scale, rect.min.y + y * scale);
    let stroke_of = |stroke: &Option<(Rgba, f32)>| match stroke {
        Some((colour, width)) => Stroke::new((width * scale).max(0.5), colour.color32()),
        None => Stroke::NONE,
    };
    for op in ops {
        match op {
            Op::Rect {
                x,
                y,
                w,
                h,
                radius,
                fill,
                stroke,
            } => {
                let shape = egui::Rect::from_min_max(at(*x, *y), at(x + w, y + h));
                painter.rect(
                    shape,
                    egui::CornerRadius::same((radius * scale).round().clamp(0.0, 255.0) as u8),
                    fill.map_or(Color32::TRANSPARENT, Rgba::color32),
                    stroke_of(stroke),
                    StrokeKind::Inside,
                );
            }
            Op::Text {
                x,
                y,
                text,
                size,
                bold: _,
                mono,
                align,
                colour,
                fit,
            } => {
                let mut size = size * scale;
                let family = |size: f32| {
                    if *mono {
                        FontId::monospace(size)
                    } else {
                        FontId::proportional(size)
                    }
                };
                if let Some(room) = fit {
                    let wide = painter
                        .layout_no_wrap(text.clone(), family(size), colour.color32())
                        .size()
                        .x;
                    if wide > room * scale {
                        size = (size * room * scale / wide).max(size * 0.6);
                    }
                }
                let anchor = match align {
                    Align::Left => Align2::LEFT_CENTER,
                    Align::Center => Align2::CENTER_CENTER,
                    Align::Right => Align2::RIGHT_CENTER,
                };
                painter.text(at(*x, *y), anchor, text, family(size), colour.color32());
            }
            Op::Line {
                points,
                colour,
                width,
            } => {
                let path: Vec<Pos2> = points.iter().map(|(x, y)| at(*x, *y)).collect();
                painter.add(Shape::line(
                    path,
                    Stroke::new((width * scale).max(0.5), colour.color32()),
                ));
            }
            Op::Circle {
                x,
                y,
                r,
                fill,
                stroke,
            } => {
                painter.circle(
                    at(*x, *y),
                    r * scale,
                    fill.map_or(Color32::TRANSPARENT, Rgba::color32),
                    stroke_of(stroke),
                );
            }
            Op::Polygon { points, fill } => {
                let path: Vec<Pos2> = points.iter().map(|(x, y)| at(*x, *y)).collect();
                painter.add(Shape::convex_polygon(path, fill.color32(), Stroke::NONE));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------- fixtures

    fn run() -> Run {
        Run {
            id: "74bbd442c16f".into(),
            started_at: "2026-11-17T12:29:00+00:00".into(),
            ended_at: "2026-11-17T13:14:00+00:00".into(),
            strategy: "somchai_trend".into(),
            exchange: "grvt".into(),
            symbol: "BTC_USDT_Perp".into(),
            mode: "paper".into(),
            ticks: 540,
            orders: 12,
            exits: 12,
            first_equity: "10000".into(),
            change: Some(-40.0),
            finished: true,
            max_drawdown_pct: Some(1.1),
            ..Default::default()
        }
    }

    fn card_of(run: &Run, check: CheckState) -> Card {
        build(&Inputs {
            name: "สมชาย ใจดี",
            run,
            check,
            running: false,
            rows: &[],
            now_unix: None,
        })
        .expect("a paper run makes a card")
    }

    fn texts(ops: &[Op]) -> Vec<String> {
        ops.iter()
            .filter_map(|op| match op {
                Op::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    // -------------------------------------------------------------- name

    #[test]
    fn a_name_is_trimmed_and_may_be_empty() {
        assert_eq!(clean_name("  สมชาย ใจดี \n"), Ok("สมชาย ใจดี".to_owned()));
        assert_eq!(clean_name("   "), Ok(String::new()));
    }

    #[test]
    fn a_name_is_capped_by_characters_and_refuses_control_characters() {
        assert!(
            clean_name(&"ก".repeat(NAME_MAX_CHARS)).is_ok(),
            "sixty Thai letters fit"
        );
        assert_eq!(
            clean_name(&"ก".repeat(NAME_MAX_CHARS + 1)),
            Err(NameProblem::TooLong)
        );
        for bad in [
            "a\nb",
            "a\tb",
            "a\u{0}b",
            "\u{7f}",
            "a\u{202e}b",
            "\u{2067}a",
        ] {
            assert_eq!(
                clean_name(bad),
                Err(NameProblem::ControlCharacter),
                "{bad:?}"
            );
        }
        assert!(
            clean_name("สมชาย\u{200b}ใจดี").is_ok(),
            "a zero-width space is not refused"
        );
    }

    #[test]
    fn the_limit_here_is_the_limit_the_server_holds() {
        assert_eq!(NAME_MAX_CHARS, 60);
    }

    // -------------------------------------------------------------- runs

    #[test]
    fn only_paper_and_testnet_runs_have_a_word_on_the_card() {
        assert_eq!(card_mode("paper"), Some(CardMode::Paper));
        assert_eq!(card_mode("grvt testnet"), Some(CardMode::Testnet));
        assert_eq!(card_mode("GRVT Testnet"), Some(CardMode::Testnet));
        for other in [
            "",
            "live",
            "grvt prod",
            "hyperliquid mainnet",
            "live + paper",
        ] {
            assert_eq!(card_mode(other), None, "{other:?}");
        }
        assert_eq!(CardMode::Paper.word(), "กระดาษ");
        assert_eq!(CardMode::Testnet.word(), "testnet");
    }

    fn run_at(id: &str, minute: u32, strategy: &str, mode: &str) -> Run {
        Run {
            id: id.into(),
            started_at: format!("2026-11-17T10:{minute:02}:00+00:00"),
            strategy: strategy.into(),
            mode: mode.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_newest_five_eligible_runs_are_offered_newest_first() {
        let runs: Vec<Run> = (0..8)
            .map(|n| run_at(&format!("r{n}"), n, "mine", "paper"))
            .collect();
        let offered: Vec<&str> = eligible(&runs).iter().map(|run| run.id.as_str()).collect();
        assert_eq!(offered, ["r7", "r6", "r5", "r4", "r3"]);
    }

    #[test]
    fn the_order_does_not_depend_on_the_order_the_server_sent() {
        let mut runs: Vec<Run> = (0..3)
            .map(|n| run_at(&format!("r{n}"), n, "mine", "paper"))
            .collect();
        runs.reverse();
        let offered: Vec<&str> = eligible(&runs).iter().map(|run| run.id.as_str()).collect();
        assert_eq!(offered, ["r2", "r1", "r0"]);
    }

    #[test]
    fn random_inferred_nameless_and_other_mode_runs_are_never_offered() {
        let mut inferred = run_at("i", 1, "mine", "paper");
        inferred.inferred = true;
        let runs = vec![
            run_at("random", 5, "random", "paper"),
            inferred,
            run_at("nameless", 4, "", "paper"),
            run_at("prod", 3, "mine", "grvt prod"),
            run_at("good", 2, "mine", "grvt testnet"),
            run_at("", 6, "mine", "paper"),
        ];
        let offered: Vec<&str> = eligible(&runs).iter().map(|run| run.id.as_str()).collect();
        assert_eq!(offered, ["good"]);
    }

    #[test]
    fn no_runs_means_no_choice() {
        assert!(eligible(&[]).is_empty());
    }

    #[test]
    fn a_finished_run_is_counted_from_its_closing_line_and_the_trail_does_not_override_it() {
        let mut closed = run();
        closed.audit_orders = 99;
        closed.audit_exits = 99;
        assert_eq!(trades_of(&closed), 12);
    }

    #[test]
    fn a_run_with_no_closing_line_is_counted_from_the_trail_not_as_zero() {
        let mut going = run();
        going.finished = false;
        going.ended_at = String::new();
        going.orders = 0;
        going.exits = 0;
        going.audit_orders = 4;
        going.audit_exits = 3;
        assert_eq!(trades_of(&going), 4);
        let badge = &card_of(&going, CheckState::Passed).badges()[2];
        assert!(badge.done, "a student in the middle of trading has traded");
        assert_eq!(badge.label, "4 ไม้");
    }

    #[test]
    fn a_trade_is_counted_once_however_many_ends_were_seen() {
        assert_eq!(
            trade_count(12, 12),
            12,
            "a round trip is one trade, not two"
        );
        assert_eq!(trade_count(3, 2), 3, "one still open at the end of the run");
        assert_eq!(trade_count(0, 1), 1, "one carried in from the run before");
        assert_eq!(trade_count(0, 0), 0);
    }

    // --------------------------------------------------------- the numbers

    #[test]
    fn profit_is_a_percentage_of_where_the_run_started() {
        assert_eq!(pl_tenths(&run()), Some(-4), "-40 on 10000 is -0.4%");
        let mut gain = run();
        gain.change = Some(123.4);
        assert_eq!(pl_tenths(&gain), Some(12));
    }

    #[test]
    fn profit_that_cannot_be_measured_is_none_never_zero() {
        let mut open = run();
        open.change = None;
        assert_eq!(pl_tenths(&open), None, "a run with no closing line");
        let mut no_start = run();
        no_start.first_equity = String::new();
        assert_eq!(pl_tenths(&no_start), None);
        no_start.first_equity = "0".into();
        assert_eq!(pl_tenths(&no_start), None, "no percentage of nothing");
        no_start.first_equity = "soon".into();
        assert_eq!(pl_tenths(&no_start), None);
    }

    #[test]
    fn the_drawdown_is_the_servers_or_nothing() {
        assert_eq!(dd_tenths(&run()), Some(11));
        let mut unknown = run();
        unknown.max_drawdown_pct = None;
        assert_eq!(dd_tenths(&unknown), None);
    }

    #[test]
    fn percentages_are_printed_with_a_sign_one_decimal_and_words_for_unknown() {
        assert_eq!(format_pl(Some(-4)), "P/L -0.4%");
        assert_eq!(format_pl(Some(12)), "P/L +1.2%");
        assert_eq!(format_pl(Some(0)), "P/L 0.0%");
        assert_eq!(format_pl(Some(-125)), "P/L -12.5%");
        assert_eq!(format_pl(None), "P/L วัดไม่ได้");
        assert_eq!(format_dd(Some(11)), "DD 1.1%");
        assert_eq!(format_dd(Some(0)), "DD 0.0%");
        assert_eq!(format_dd(None), "DD วัดไม่ได้");
        assert!(
            format_pl(Some(-4)).is_ascii(),
            "the minus is a plain hyphen"
        );
    }

    #[test]
    fn a_length_is_said_in_the_largest_unit_that_fits() {
        assert_eq!(format_duration(0), "0 วินาที");
        assert_eq!(format_duration(59), "59 วินาที");
        assert_eq!(format_duration(60), "1 นาที");
        assert_eq!(format_duration(45 * 60 + 30), "45 นาที");
        assert_eq!(format_duration(3600), "1 ชม.");
        assert_eq!(format_duration(3600 + 5 * 60), "1 ชม. 5 นาที");
    }

    #[test]
    fn a_running_run_lasts_until_now_and_a_killed_one_has_no_length() {
        let mut going = run();
        going.finished = false;
        going.ended_at = String::new();
        let start = time::utc_seconds(&going.started_at).unwrap() as f64;
        assert_eq!(
            duration_of(&going, RunStatus::Running, Some(start + 600.5)),
            Some(600)
        );
        assert_eq!(duration_of(&going, RunStatus::Running, None), None);
        assert_eq!(
            duration_of(&going, RunStatus::Unfinished, Some(start + 9.0)),
            None
        );
        assert_eq!(
            duration_of(&run(), RunStatus::Finished, None),
            Some(45 * 60)
        );
    }

    // ------------------------------------------------------------ the code

    fn input<'a>() -> CodeInput<'a> {
        CodeInput {
            run_id: "74bbd442c16f",
            name: "สมชาย ใจดี",
            strategy: "somchai_trend",
            mode: CardMode::Paper,
            trades: 12,
            pl_tenths: Some(-4),
            dd_tenths: Some(11),
        }
    }

    #[test]
    fn a_code_is_eight_characters_in_two_groups_of_four() {
        let code = submission_code(&input());
        assert_eq!(code.len(), 9, "{code}");
        assert_eq!(code.as_bytes()[4], b'-', "{code}");
        for (index, ch) in code.chars().enumerate() {
            if index != 4 {
                assert!(ALPHABET.contains(&(ch as u8)), "{ch} in {code}");
            }
        }
    }

    #[test]
    fn the_alphabet_has_no_zero_no_o_no_one_no_i_and_no_repeats() {
        assert_eq!(ALPHABET.len(), 32);
        for banned in *b"0O1I" {
            assert!(!ALPHABET.contains(&banned), "{}", banned as char);
        }
        let mut sorted = ALPHABET.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 32, "every character once");
    }

    #[test]
    fn the_same_run_and_numbers_always_give_the_same_code() {
        assert_eq!(submission_code(&input()), submission_code(&input()));
    }

    /// The recipe is fixed: this value is what a teacher recomputes next term.
    /// If this fails, an old picture would stop matching its own code. Change
    /// the recipe only on purpose, and then change `board-submission-v1`.
    #[test]
    fn the_recipe_gives_the_same_code_it_gave_when_it_was_written() {
        assert_eq!(submission_code(&input()), PINNED_CODE);
    }
    const PINNED_CODE: &str = "ZMVS-ML3Y";

    #[test]
    fn every_field_changes_the_code() {
        let base = submission_code(&input());
        let variants: Vec<(&str, CodeInput)> = vec![
            (
                "run id",
                CodeInput {
                    run_id: "74bbd442c16e",
                    ..input()
                },
            ),
            (
                "name",
                CodeInput {
                    name: "สมชาย ใจดี ",
                    ..input()
                },
            ),
            (
                "strategy",
                CodeInput {
                    strategy: "somchai_trent",
                    ..input()
                },
            ),
            (
                "mode",
                CodeInput {
                    mode: CardMode::Testnet,
                    ..input()
                },
            ),
            (
                "trades",
                CodeInput {
                    trades: 13,
                    ..input()
                },
            ),
            (
                "profit",
                CodeInput {
                    pl_tenths: Some(-3),
                    ..input()
                },
            ),
            (
                "profit unknown",
                CodeInput {
                    pl_tenths: None,
                    ..input()
                },
            ),
            (
                "drawdown",
                CodeInput {
                    dd_tenths: Some(12),
                    ..input()
                },
            ),
            (
                "drawdown unknown",
                CodeInput {
                    dd_tenths: None,
                    ..input()
                },
            ),
        ];
        for (what, changed) in variants {
            assert_ne!(submission_code(&changed), base, "{what}");
        }
    }

    #[test]
    fn fields_cannot_slide_into_each_other() {
        // "ab"+"c" and "a"+"bc" are different runs.
        let one = CodeInput {
            name: "ab",
            strategy: "c",
            ..input()
        };
        let two = CodeInput {
            name: "a",
            strategy: "bc",
            ..input()
        };
        assert_ne!(submission_code(&one), submission_code(&two));
    }

    #[test]
    fn a_change_of_one_digit_changes_most_of_the_code() {
        let a = submission_code(&input());
        let b = submission_code(&CodeInput {
            trades: 13,
            ..input()
        });
        let same = a.chars().zip(b.chars()).filter(|(x, y)| x == y).count();
        assert!(same <= 4, "{a} against {b} share {same} of 9 places");
    }

    #[test]
    fn many_different_inputs_give_many_different_codes() {
        let mut seen = std::collections::BTreeSet::new();
        for trades in 0..60 {
            for pl in -20..20 {
                let code = submission_code(&CodeInput {
                    trades,
                    pl_tenths: Some(pl),
                    ..input()
                });
                assert!(
                    seen.insert(code.clone()),
                    "a collision at {trades}/{pl}: {code}"
                );
            }
        }
    }

    #[test]
    fn the_code_on_the_card_is_the_code_of_its_numbers() {
        let card = card_of(&run(), CheckState::Passed);
        assert_eq!(card.code, submission_code(&input()));
    }

    // ---------------------------------------------------------- the card

    #[test]
    fn a_students_own_finished_run_has_all_three_badges_done() {
        let card = card_of(&run(), CheckState::Passed);
        let badges = card.badges();
        assert_eq!(
            badges.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
            ["เขียนเอง", "ผ่านด่าน", "12 ไม้"]
        );
        assert!(badges.iter().all(|badge| badge.done && badge.why.is_none()));
        assert_eq!(card.strategy_line(), "กลยุทธ์: somchai_trend (เขียนเอง)");
        assert_eq!(card.meta_line(), "กระดาษ · 45 นาที · P/L -0.4% · DD 1.1%");
        assert_eq!(card.started, "17 พ.ย. 2569 19:29");
        assert_eq!(card.status_tag(), None);
    }

    #[test]
    fn a_case_study_says_so_and_its_first_badge_is_not_ticked() {
        let mut case = run();
        case.strategy = "ping_pong".into();
        let card = card_of(&case, CheckState::Passed);
        let badges = card.badges();
        assert!(!badges[0].done, "ไม่ใช่ของที่เขียนเอง");
        assert_eq!(badges[0].why, Some("กรณีศึกษา"));
        assert_eq!(card.strategy_line(), "กลยุทธ์: ping_pong (กรณีศึกษา)");
        assert!(badges[1].done, "the check state is still the check state");
        let ops = layout(&card);
        assert!(texts(&ops).contains(&"กลยุทธ์: ping_pong (กรณีศึกษา)".to_owned()));
        let line = ops.iter().find_map(|op| match op {
            Op::Text { text, colour, .. } if text.starts_with("กลยุทธ์") => {
                Some(*colour)
            }
            _ => None,
        });
        assert_eq!(line, Some(warn()), "a case study is said in amber");
    }

    #[test]
    fn the_check_badge_follows_the_current_state_of_the_strategy() {
        let passed = card_of(&run(), CheckState::Passed).badges();
        assert!(passed[1].done);
        let not_passed = card_of(&run(), CheckState::NotPassed).badges();
        assert!(!not_passed[1].done);
        assert_eq!(not_passed[1].why, Some("ยังไม่ผ่าน"));
        let unknown = card_of(&run(), CheckState::Unknown).badges();
        assert!(!unknown[1].done);
        assert_eq!(unknown[1].why, Some("ยังไม่ทราบผล"));
    }

    #[test]
    fn a_testnet_run_since_the_gate_passed_whatever_the_check_says_now() {
        let testnet = Run {
            mode: "grvt testnet".into(),
            ..run()
        };
        for now in [CheckState::NotPassed, CheckState::Unknown] {
            let badges = card_of(&testnet, now).badges();
            assert!(badges[1].done);
            assert_eq!(badges[1].why, None);
        }
    }

    #[test]
    fn a_paper_run_still_follows_the_check_as_it_is_now() {
        let badges = card_of(&run(), CheckState::NotPassed).badges();
        assert!(!badges[1].done);
        assert_eq!(badges[1].why, Some("ยังไม่ผ่าน"));
    }

    #[test]
    fn a_testnet_run_from_before_the_gate_proves_nothing() {
        let old = Run {
            mode: "grvt testnet".into(),
            started_at: "2026-11-16T15:30:22+00:00".into(),
            ..run()
        };
        assert!(!card_of(&old, CheckState::NotPassed).badges()[1].done);
        assert!(card_of(&old, CheckState::Passed).badges()[1].done);
        let at_the_gate = Run {
            started_at: LIVE_GATE_SINCE.into(),
            ..old
        };
        assert!(card_of(&at_the_gate, CheckState::NotPassed).badges()[1].done);
    }

    #[test]
    fn a_run_that_never_traded_has_a_grey_trade_badge() {
        let mut idle = run();
        idle.orders = 0;
        idle.exits = 0;
        let badge = &card_of(&idle, CheckState::Passed).badges()[2];
        assert!(!badge.done);
        assert_eq!(badge.label, "0 ไม้");
        assert_eq!(badge.why, Some("ยังไม่มีไม้"));
    }

    #[test]
    fn a_running_run_is_labelled_and_a_killed_one_says_it_did_not_finish() {
        let mut going = run();
        going.finished = false;
        going.ended_at = String::new();
        going.change = None;
        let start = time::utc_seconds(&going.started_at).unwrap() as f64;
        let card = build(&Inputs {
            name: "สมชาย",
            run: &going,
            check: CheckState::Passed,
            running: true,
            rows: &[],
            now_unix: Some(start + 125.0),
        })
        .unwrap();
        assert_eq!(card.status, RunStatus::Running);
        assert_eq!(card.status_tag(), Some("กำลังรัน"));
        assert_eq!(card.meta_line(), "กระดาษ · 2 นาที · P/L วัดไม่ได้ · DD 1.1%");
        assert!(texts(&layout(&card)).contains(&"กำลังรัน".to_owned()));

        let killed = build(&Inputs {
            name: "สมชาย",
            run: &going,
            check: CheckState::Passed,
            running: false,
            rows: &[],
            now_unix: Some(start + 125.0),
        })
        .unwrap();
        assert_eq!(killed.status, RunStatus::Unfinished);
        assert_eq!(killed.status_tag(), Some("ยังไม่จบ"));
        assert!(
            !killed.meta_line().contains("นาที"),
            "no length is made up for a run with no end: {}",
            killed.meta_line()
        );
    }

    #[test]
    fn a_run_on_a_real_market_makes_no_card() {
        let mut prod = run();
        prod.mode = "grvt prod".into();
        assert!(build(&Inputs {
            name: "สมชาย",
            run: &prod,
            check: CheckState::Passed,
            running: false,
            rows: &[],
            now_unix: None,
        })
        .is_none());
    }

    // ---------------------------------------------------------- the chart

    fn tick(at: &str, price: &str, extra: serde_json::Value) -> Row {
        let mut row = serde_json::json!({
            "at": at, "symbol": "BTC_USDT_Perp", "price": price, "equity": "10000"
        });
        if let (Some(base), Some(more)) = (row.as_object_mut(), extra.as_object()) {
            base.extend(more.clone());
        }
        serde_json::from_value(row).unwrap()
    }

    fn ticks() -> Vec<Row> {
        vec![
            // Before the run starts.
            tick("2026-11-17T12:00:00+00:00", "99", serde_json::json!({})),
            tick("2026-11-17T12:30:00+00:00", "100", serde_json::json!({})),
            tick(
                "2026-11-17T12:40:00+00:00",
                "101",
                serde_json::json!({
                    "order": {"side": "buy", "limit_price": "100.5", "client_order_id": "a"},
                    "receipt": {"broker_order_id": "x", "simulated": true}
                }),
            ),
            tick("2026-11-17T12:50:00+00:00", "103", serde_json::json!({})),
            tick(
                "2026-11-17T13:00:00+00:00",
                "102",
                serde_json::json!({"exit": {"reason": "target", "broker_order_id": "y", "simulated": true}}),
            ),
            // The bot asked to close and nothing went out.
            tick(
                "2026-11-17T13:05:00+00:00",
                "102",
                serde_json::json!({"exit": {"reason": "again", "broker_order_id": "", "simulated": false}}),
            ),
            // After the run ends.
            tick("2026-11-17T14:00:00+00:00", "200", serde_json::json!({})),
        ]
    }

    #[test]
    fn the_chart_is_the_ticks_of_this_run_and_no_others() {
        let chart = chart_of(&run(), &ticks()).expect("five ticks fall inside the run");
        assert_eq!(chart.symbol, "BTC_USDT_Perp");
        let prices: Vec<f64> = chart.points.iter().map(|p| p.1).collect();
        assert_eq!(prices, [100.0, 101.0, 103.0, 102.0, 102.0]);
    }

    #[test]
    fn an_entry_is_an_order_that_was_answered_and_an_exit_a_position_let_go() {
        let chart = chart_of(&run(), &ticks()).unwrap();
        let kinds: Vec<MarkKind> = chart.marks.iter().map(|m| m.kind).collect();
        assert_eq!(kinds, [MarkKind::EntryBuy, MarkKind::Exit]);
        assert_eq!(
            chart.marks[0].price, 100.5,
            "an entry sits at its limit price"
        );
    }

    #[test]
    fn ticks_of_another_symbol_are_not_this_runs_line() {
        let mut rows = ticks();
        rows[2].symbol = "ETH_USDT_Perp".into();
        let chart = chart_of(&run(), &rows).unwrap();
        assert_eq!(chart.points.len(), 4);
        assert!(chart.marks.iter().all(|m| m.kind != MarkKind::EntryBuy));
    }

    #[test]
    fn a_run_older_than_the_ticks_the_board_holds_has_no_chart() {
        let mut old = run();
        old.started_at = "2026-11-01T00:00:00+00:00".into();
        old.ended_at = "2026-11-01T01:00:00+00:00".into();
        assert!(chart_of(&old, &ticks()).is_none());
        assert!(chart_of(&run(), &[]).is_none());
    }

    #[test]
    fn one_tick_is_not_a_line() {
        let rows = vec![ticks().remove(1)];
        assert!(chart_of(&run(), &rows).is_none());
    }

    #[test]
    fn a_long_run_is_thinned_but_keeps_its_first_and_last_tick() {
        let rows: Vec<Row> = (0..1500)
            .map(|n| {
                let at = time::clock_iso(
                    time::utc_seconds("2026-11-17T12:30:00+00:00").unwrap() as f64 + n as f64,
                );
                tick(&at, &format!("{}", 100 + n % 7), serde_json::json!({}))
            })
            .collect();
        let chart = chart_of(&run(), &rows).unwrap();
        assert_eq!(chart.points.len(), MAX_POINTS);
        assert_eq!(chart.points[0].1, rows[0].price_value().unwrap());
        assert_eq!(
            chart.points[MAX_POINTS - 1].1,
            rows[1499].price_value().unwrap()
        );
    }

    #[test]
    fn a_run_still_going_draws_every_tick_since_it_started() {
        let mut going = run();
        going.finished = false;
        going.ended_at = String::new();
        let chart = chart_of(&going, &ticks()).unwrap();
        assert_eq!(chart.points.last().map(|p| p.1), Some(200.0));
    }

    // -------------------------------------------------------- the picture

    fn with_chart() -> Card {
        let mut card = card_of(&run(), CheckState::Passed);
        card.chart = chart_of(&run(), &ticks());
        card
    }

    fn inside_the_picture(op: &Op) -> bool {
        let within = |x: f32, y: f32| (0.0..=WIDTH).contains(&x) && (0.0..=HEIGHT).contains(&y);
        match op {
            Op::Rect { x, y, w, h, .. } => within(*x, *y) && within(x + w, y + h),
            Op::Text { x, y, .. } => within(*x, *y),
            Op::Line { points, .. } | Op::Polygon { points, .. } => {
                points.iter().all(|(x, y)| within(*x, *y))
            }
            Op::Circle { x, y, r, .. } => within(x - r, y - r) && within(x + r, y + r),
        }
    }

    #[test]
    fn the_picture_is_1200_by_675() {
        assert_eq!((WIDTH, HEIGHT), (1200.0, 675.0));
    }

    #[test]
    fn nothing_is_drawn_off_the_picture() {
        for card in [with_chart(), card_of(&run(), CheckState::Unknown)] {
            for op in layout(&card) {
                assert!(inside_the_picture(&op), "{op:?}");
            }
        }
    }

    #[test]
    fn the_three_badges_are_equal_and_side_by_side_inside_the_margins() {
        let rects: Vec<_> = (0..3).map(badge_rect).collect();
        for pair in rects.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!(a.0 + a.2 < b.0, "the badges do not touch: {a:?} {b:?}");
            assert_eq!(a.2, b.2);
            assert_eq!(a.1, b.1);
        }
        assert_eq!(rects[0].0, MARGIN);
        assert!((rects[2].0 + rects[2].2 - (WIDTH - MARGIN)).abs() < 1e-3);
    }

    #[test]
    fn the_badges_are_the_biggest_things_on_the_card() {
        let ops = layout(&with_chart());
        let size_of = |wanted: &str| {
            ops.iter()
                .find_map(|op| match op {
                    Op::Text { text, size, .. } if text == wanted => Some(*size),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no text {wanted:?}"))
        };
        let badge = size_of("เขียนเอง");
        for small in [
            size_of("สมชาย ใจดี"),
            size_of(&with_chart().strategy_line()),
            size_of(&with_chart().meta_line()),
        ] {
            assert!(badge > small, "{badge} against {small}");
        }
    }

    #[test]
    fn profit_and_drawdown_are_small_and_never_red_or_green() {
        let ops = layout(&with_chart());
        for op in &ops {
            if let Op::Text {
                text, size, colour, ..
            } = op
            {
                if text.contains("P/L") {
                    assert!(*size <= 28.0, "{size}");
                    assert!(
                        *colour != ok() && *colour != bad(),
                        "profit is not a verdict"
                    );
                }
            }
        }
    }

    #[test]
    fn the_layout_says_what_the_mock_says() {
        let words = texts(&layout(&with_chart()));
        for want in [
            "สมชาย ใจดี",
            APP_TAG,
            "กลยุทธ์: somchai_trend (เขียนเอง)",
            "เขียนเอง",
            "ผ่านด่าน",
            "12 ไม้",
            "กระดาษ · 45 นาที · P/L -0.4% · DD 1.1%",
            "รันเมื่อ 17 พ.ย. 2569 19:29",
        ] {
            assert!(words.contains(&want.to_owned()), "{want}: {words:?}");
        }
        let code = words
            .iter()
            .find(|w| w.starts_with("รหัส "))
            .expect("a code");
        assert_eq!(code.len(), "รหัส ".len() + 9);
    }

    #[test]
    fn a_ticked_badge_has_a_drawn_tick_and_an_unticked_one_has_a_ring() {
        let ticks_in = |card: &Card| {
            layout(card)
                .iter()
                .filter(|op| matches!(op, Op::Line { points, width, .. } if points.len() == 3 && *width == 6.0))
                .count()
        };
        assert_eq!(ticks_in(&card_of(&run(), CheckState::Passed)), 3);
        assert_eq!(ticks_in(&card_of(&run(), CheckState::NotPassed)), 2);
        let mut case = run();
        case.strategy = "ma_cross".into();
        assert_eq!(
            ticks_in(&card_of(&case, CheckState::NotPassed)),
            1,
            "only the trades are done"
        );
    }

    #[test]
    fn no_tick_is_ever_typed_as_a_character() {
        for card in [with_chart(), card_of(&run(), CheckState::NotPassed)] {
            for word in texts(&layout(&card)) {
                assert!(
                    !word.contains('✓') && !word.contains('✔'),
                    "a typed tick could be an empty box: {word}"
                );
            }
        }
    }

    #[test]
    fn a_missing_chart_leaves_a_clean_panel_that_says_why() {
        let ops = layout(&card_of(&run(), CheckState::Passed));
        assert!(texts(&ops).contains(&"ไม่มีข้อมูลราคาของรอบนี้ในบอร์ด".to_owned()));
        assert!(
            ops.iter().all(|op| !matches!(op, Op::Polygon { .. })),
            "no marks without a line"
        );
    }

    #[test]
    fn the_price_line_and_its_marks_stay_inside_the_plot_area() {
        let (x0, y0, x1, y1) = plot_area();
        let ops = layout(&with_chart());
        let line = ops
            .iter()
            .find_map(|op| match op {
                Op::Line { points, width, .. } if points.len() == 5 && *width == 3.0 => {
                    Some(points)
                }
                _ => None,
            })
            .expect("the price line");
        for (x, y) in line {
            assert!((x0..=x1).contains(x) && (y0..=y1).contains(y), "({x}, {y})");
        }
        // Time runs left to right.
        assert!(line.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        // A higher price is higher up the picture (a smaller y).
        assert!(line[2].1 < line[0].1, "103 is above 100");
    }

    #[test]
    fn the_time_labels_are_read_off_the_ticks_drawn() {
        let words = texts(&layout(&with_chart()));
        assert!(words.contains(&"19:30".to_owned()), "{words:?}");
        assert!(words.contains(&"20:05".to_owned()), "{words:?}");
        assert!(words.contains(&"ราคา BTC_USDT_Perp".to_owned()));
    }

    #[test]
    fn a_flat_price_is_drawn_in_the_middle_not_divided_by_zero() {
        let chart = Chart {
            symbol: "X".into(),
            points: vec![(0.0, 50.0), (10.0, 50.0)],
            marks: vec![],
        };
        let scale = Scale::of(&chart);
        let (_, y) = scale.at(5.0, 50.0);
        let (_, y0, _, y1) = plot_area();
        assert!(y.is_finite());
        assert!((y - (y0 + y1) / 2.0).abs() < 1.0, "{y}");
    }

    #[test]
    fn no_secret_can_be_on_the_card_because_the_card_has_no_field_for_one() {
        // The whole text of a card, for a name that looks like a key: it is
        // printed as typed, and nothing else in the card is from anywhere
        // else.
        let card = with_chart();
        let words = texts(&layout(&card)).join(" ");
        for forbidden in [
            "GRVT_",
            "api_key",
            "private",
            "0x",
            "sub_account",
            "sub-account",
        ] {
            assert!(!words.contains(forbidden), "{forbidden}: {words}");
        }
    }

    #[test]
    fn the_colours_are_the_boards_own() {
        assert_eq!(Rgba::of(theme::OK), Rgba(0x5f, 0xbe, 0x86, 255));
        assert_eq!(Rgba::of(theme::OK).alpha(0.5).3, 128);
        assert_eq!(Rgba(1, 2, 3, 255).css(), "rgba(1, 2, 3, 1.000)");
        assert_eq!(Rgba(1, 2, 3, 0).css(), "rgba(1, 2, 3, 0.000)");
    }

    // ------------------------------------------------------------ the file

    #[test]
    fn the_file_is_named_for_the_student_and_the_moment() {
        let at = time::utc_seconds("2026-11-17T13:14:59+00:00").unwrap() as f64;
        assert_eq!(
            file_name("สมชาย ใจดี", at),
            "ส่งงาน_สมชาย_ใจดี_20261117-2014.png"
        );
    }

    #[test]
    fn a_name_cannot_put_a_path_or_a_forbidden_character_in_the_file_name() {
        let at = time::utc_seconds("2026-11-17T13:14:59+00:00").unwrap() as f64;
        let name = file_name("..\\..\\evil/name:*?\"<>|", at);
        assert_eq!(name, "ส่งงาน_evilname_20261117-2014.png");
        for forbidden in ['\\', '/', ':', '*', '?', '"', '<', '>', '|'] {
            assert!(!name.contains(forbidden), "{name}");
        }
    }

    #[test]
    fn a_character_that_turns_the_text_around_never_reaches_a_file_name() {
        let at = time::utc_seconds("2026-11-17T13:14:59+00:00").unwrap() as f64;
        assert_eq!(
            file_name("evil\u{202e}gpj", at),
            "ส่งงาน_evilgpj_20261117-2014.png"
        );
    }

    #[test]
    fn a_name_with_nothing_usable_leaves_the_name_out() {
        let at = time::utc_seconds("2026-11-17T13:14:59+00:00").unwrap() as f64;
        assert_eq!(file_name("", at), "ส่งงาน_20261117-2014.png");
        assert_eq!(file_name(" ... ", at), "ส่งงาน_20261117-2014.png");
        assert_eq!(file_name("***", at), "ส่งงาน_20261117-2014.png");
    }

    #[test]
    fn a_long_name_is_cut_and_runs_of_spaces_become_one_underscore() {
        let at = time::utc_seconds("2026-11-17T13:14:59+00:00").unwrap() as f64;
        assert_eq!(
            file_name("  Somchai    Jaidee  ", at),
            "ส่งงาน_Somchai_Jaidee_20261117-2014.png"
        );
        let long = file_name(&"ก".repeat(60), at);
        assert_eq!(long.matches('ก').count(), 40);
    }

    // ------------------------------------------------------------- saving

    #[test]
    fn saving_says_what_happened_in_order() {
        let mut status = SaveStatus::default();
        assert_eq!(status.line(), None);
        status = SaveStatus::Working;
        assert!(status.busy());
        status.apply(SaveEvent::Downloaded, "a.png");
        assert_eq!(status.line().unwrap().0, "บันทึกแล้ว · กำลังคัดลอก...");
        assert!(!status.busy());
        status.apply(SaveEvent::Copied, "a.png");
        assert_eq!(
            status.line(),
            Some(("บันทึกแล้ว · คัดลอกไว้แล้ว วางใน LINE ได้เลย".to_owned(), true))
        );
    }

    #[test]
    fn a_clipboard_that_would_not_take_it_leaves_the_download_standing() {
        let mut status = SaveStatus::Working;
        status.apply(SaveEvent::Downloaded, "a.png");
        status.apply(SaveEvent::NotCopied("denied".into()), "a.png");
        let (line, good) = status.line().unwrap();
        assert!(good, "the file is saved: {line}");
        assert!(line.contains("คัดลอกอัตโนมัติไม่ได้"), "{line}");
    }

    #[test]
    fn a_failure_is_bad_news_and_says_why() {
        let mut status = SaveStatus::Working;
        status.apply(SaveEvent::Failed("no canvas".into()), "a.png");
        let (line, good) = status.line().unwrap();
        assert!(!good);
        assert!(line.contains("no canvas"), "{line}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_desktop_window_says_it_cannot_make_the_picture() {
        let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = heard.clone();
        save_png(vec![], "a.png".into(), move |event| {
            sink.borrow_mut().push(event)
        });
        let heard = heard.borrow();
        assert!(matches!(heard.as_slice(), [SaveEvent::Failed(why)] if why.contains("เบราว์เซอร์")));
    }
}
