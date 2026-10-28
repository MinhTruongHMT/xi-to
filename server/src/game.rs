//! Trạng thái phòng chơi và luật Xì Tố 5 lá (Five-Card Stud).
//!
//! Luồng một ván:
//!   1. Mỗi người đặt cược sàn (ANTE), nhận 1 lá tẩy úp (gửi riêng) + 1 lá ngửa.
//!   2. Vòng cược. Người có bài ngửa mạnh nhất nói trước.
//!   3. Chia thêm 1 lá ngửa, cược tiếp... cho tới khi đủ 5 lá.
//!   4. Lật bài (showdown), chia hũ chính và các hũ phụ (side pot).
//!
//! Room không tự chạy hẹn giờ; các hàm trả về `Effect` để main.rs đặt timer.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;

use crate::cards::{evaluate, hand_name, Card, Deck, HandValue, Suit};

pub const MAX_PLAYERS: usize = 6;
pub const START_CHIPS: u64 = 1000;
pub const ANTE: u64 = 10;
pub const MIN_BET: u64 = 20;
pub const TURN_MS: u64 = 25_000;
pub const NEXT_HAND_MS: u64 = 7_000;

pub enum Effect {
    TurnTimer { token: u64, ms: u64 },
    NextHand { hand_no: u64, ms: u64 },
}

pub enum Action {
    Fold,
    /// Theo (hoặc Xem/check khi không phải bỏ thêm tiền).
    Call,
    /// Số chip tố thêm, tính TRÊN số tiền cần để theo.
    Raise(u64),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Waiting,
    Betting,
    Showdown,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Waiting => "waiting",
            Phase::Betting => "betting",
            Phase::Showdown => "showdown",
        }
    }
}

pub struct Player {
    pub id: u64,
    pub name: String,
    pub seat: usize,
    pub chips: u64,
    pub connected: bool,
    tx: UnboundedSender<String>,
    // --- trạng thái trong ván ---
    in_hand: bool,
    folded: bool,
    all_in: bool,
    hole: Option<Card>,
    up: Vec<Card>,
    street_bet: u64,
    total_bet: u64,
    acted: bool,
    revealed: bool,
    last_action: Option<String>,
}

impl Player {
    fn new(id: u64, name: String, seat: usize, tx: UnboundedSender<String>) -> Self {
        Player {
            id,
            name,
            seat,
            chips: START_CHIPS,
            connected: true,
            tx,
            in_hand: false,
            folded: false,
            all_in: false,
            hole: None,
            up: Vec::new(),
            street_bet: 0,
            total_bet: 0,
            acted: false,
            revealed: false,
            last_action: None,
        }
    }

    fn reset_hand(&mut self) {
        self.in_hand = false;
        self.folded = false;
        self.all_in = false;
        self.hole = None;
        self.up.clear();
        self.street_bet = 0;
        self.total_bet = 0;
        self.acted = false;
        self.revealed = false;
        self.last_action = None;
    }

    /// Còn trong ván (chưa úp bỏ).
    fn live(&self) -> bool {
        self.in_hand && !self.folded
    }

    /// Còn có thể hành động (chưa úp, chưa tất tay).
    fn can_act(&self) -> bool {
        self.live() && !self.all_in
    }

    fn pay(&mut self, amount: u64) -> u64 {
        let paid = amount.min(self.chips);
        self.chips -= paid;
        self.street_bet += paid;
        self.total_bet += paid;
        if self.chips == 0 {
            self.all_in = true;
        }
        paid
    }

    fn all_cards(&self) -> Vec<Card> {
        self.hole.iter().chain(self.up.iter()).copied().collect()
    }

    fn send(&self, msg: &Value) {
        if self.connected {
            let _ = self.tx.send(msg.to_string());
        }
    }
}

/// Khoá xếp hạng bài ngửa để chọn người nói trước; hoà thì so chất của lá cao nhất.
fn visible_key(p: &Player) -> (HandValue, Suit) {
    let value = evaluate(&p.up);
    let top = value.ranks[0];
    let suit = p.up.iter().filter(|c| c.rank == top).map(|c| c.suit).max().unwrap();
    (value, suit)
}

pub struct Room {
    pub id: String,
    pub players: Vec<Player>,
    pub phase: Phase,
    deck: Deck,
    hand_no: u64,
    /// Số lá mỗi người đang có (2..=5).
    street: usize,
    current_bet: u64,
    min_raise: u64,
    turn: Option<u64>,
    turn_token: u64,
    turn_deadline: Option<Instant>,
    dealer_seat: usize,
    result: Option<Value>,
}

impl Room {
    pub fn new(id: String) -> Self {
        Room {
            id,
            players: Vec::new(),
            phase: Phase::Waiting,
            deck: Deck::shuffled(),
            hand_no: 0,
            street: 0,
            current_bet: 0,
            min_raise: MIN_BET,
            turn: None,
            turn_token: 0,
            turn_deadline: None,
            dealer_seat: MAX_PLAYERS - 1,
            result: None,
        }
    }

    // ------------------------------------------------------------------
    // Người chơi vào / ra
    // ------------------------------------------------------------------

    pub fn add_player(&mut self, id: u64, name: String, tx: UnboundedSender<String>) -> Result<(), String> {
        if self.players.len() >= MAX_PLAYERS {
            return Err(format!("Bàn đã đầy (tối đa {} người)", MAX_PLAYERS));
        }
        let seat = (0..MAX_PLAYERS)
            .find(|s| !self.players.iter().any(|p| p.seat == *s))
            .unwrap();
        self.event("join", format!("{} vào bàn", name), Some(id));
        self.players.push(Player::new(id, name, seat, tx));
        self.players.sort_by_key(|p| p.seat);
        Ok(())
    }

    pub fn remove_player(&mut self, id: u64) -> Vec<Effect> {
        let Some(i) = self.idx(id) else { return vec![] };
        let name = self.players[i].name.clone();
        self.event("leave", format!("{} rời bàn", name), Some(id));

        if self.phase != Phase::Betting || !self.players[i].in_hand {
            self.players.remove(i);
            return vec![];
        }
        // Đang trong ván: giữ ghế (tiền đã cược vẫn nằm trong hũ) và xử như úp bỏ.
        self.players[i].connected = false;
        if self.turn == Some(id) {
            return self.act(id, Action::Fold).unwrap_or_default();
        }
        if self.players[i].live() {
            self.players[i].folded = true;
            self.players[i].last_action = Some("Rời bàn".into());
            let live = self.live_indices();
            if live.len() == 1 {
                return self.award_uncontested(live[0]);
            }
        }
        vec![]
    }

    pub fn is_empty(&self) -> bool {
        self.players.iter().all(|p| !p.connected)
    }

    // ------------------------------------------------------------------
    // Diễn tiến ván bài
    // ------------------------------------------------------------------

    pub fn start_hand(&mut self) -> Result<Vec<Effect>, String> {
        if self.phase == Phase::Betting {
            return Err("Ván đang diễn ra".into());
        }
        self.players.retain(|p| p.connected);
        if self.players.len() < 2 {
            self.phase = Phase::Waiting;
            return Err("Cần ít nhất 2 người chơi để bắt đầu".into());
        }

        self.hand_no += 1;
        self.result = None;
        self.deck = Deck::shuffled();
        let next_dealer = self.clockwise_after(self.dealer_seat)[0];
        self.dealer_seat = self.players[next_dealer].seat;

        let mut rebuys = Vec::new();
        for p in &mut self.players {
            p.reset_hand();
            if p.chips == 0 {
                p.chips = START_CHIPS;
                rebuys.push(p.name.clone());
            }
            p.in_hand = true;
            p.pay(ANTE);
        }
        for name in rebuys {
            self.event("info", format!("{} hết chip, được nạp lại {}", name, START_CHIPS), None);
        }
        self.event(
            "hand_start",
            format!("Ván #{} — mỗi người đặt sàn {}", self.hand_no, ANTE),
            None,
        );

        // Lá tẩy: chỉ gửi riêng cho chủ nhân, không bao giờ broadcast.
        let order = self.clockwise_after(self.dealer_seat);
        for &i in &order {
            let card = self.deck.deal();
            self.players[i].hole = Some(card);
            self.players[i].send(&json!({ "type": "hole", "card": card }));
        }
        for &i in &order {
            let card = self.deck.deal();
            self.players[i].up.push(card);
        }
        self.street = 2;
        self.phase = Phase::Betting;
        self.event("deal", "Chia lá tẩy và lá ngửa đầu tiên".into(), None);
        Ok(self.begin_street())
    }

    fn begin_street(&mut self) -> Vec<Effect> {
        for p in &mut self.players {
            p.street_bet = 0;
            p.acted = false;
            if p.can_act() {
                p.last_action = None;
            }
        }
        self.current_bet = 0;
        self.min_raise = MIN_BET;

        let first = self
            .players
            .iter()
            .filter(|p| p.can_act())
            .max_by_key(|p| visible_key(p))
            .map(|p| (p.id, p.name.clone()));
        let actionable = self.players.iter().filter(|p| p.can_act()).count();
        match first {
            Some((id, name)) if actionable >= 2 => {
                self.event("info", format!("{} có bài ngửa cao nhất, nói trước", name), None);
                vec![self.set_turn(id)]
            }
            // Không còn ai để cược với nhau (mọi người khác đã tất tay): lật hết bài.
            _ => self.run_out(),
        }
    }

    fn set_turn(&mut self, id: u64) -> Effect {
        self.turn = Some(id);
        self.turn_token += 1;
        self.turn_deadline = Some(Instant::now() + Duration::from_millis(TURN_MS));
        if let Some(i) = self.idx(id) {
            self.players[i].send(&json!({ "type": "event", "kind": "your_turn", "text": "Đến lượt bạn!" }));
        }
        Effect::TurnTimer { token: self.turn_token, ms: TURN_MS }
    }

    pub fn act(&mut self, id: u64, action: Action) -> Result<Vec<Effect>, String> {
        if self.phase != Phase::Betting {
            return Err("Chưa có ván nào đang diễn ra".into());
        }
        if self.turn != Some(id) {
            return Err("Chưa đến lượt bạn".into());
        }
        let i = self.idx(id).ok_or("Không tìm thấy người chơi")?;
        let (current_bet, min_raise) = (self.current_bet, self.min_raise);
        let p = &mut self.players[i];
        let to_call = current_bet.saturating_sub(p.street_bet);

        let (kind, text, label) = match action {
            Action::Fold => {
                p.folded = true;
                ("fold", format!("{} úp bỏ", p.name), "Úp bỏ".to_string())
            }
            Action::Call if to_call == 0 => ("check", format!("{} xem bài", p.name), "Xem".to_string()),
            Action::Call => {
                let paid = p.pay(to_call);
                if p.all_in {
                    ("allin", format!("{} tất tay {}", p.name, paid), format!("Tất tay {}", paid))
                } else {
                    ("call", format!("{} theo {}", p.name, paid), format!("Theo {}", paid))
                }
            }
            Action::Raise(extra) => {
                let total = to_call.saturating_add(extra);
                if total < p.chips && extra < min_raise {
                    return Err(format!("Phải tố thêm ít nhất {}", min_raise));
                }
                let paid = p.pay(total);
                if p.all_in {
                    ("allin", format!("{} tất tay {}!", p.name, paid), format!("Tất tay {}", paid))
                } else {
                    ("raise", format!("{} tố {}", p.name, extra), format!("Tố {}", extra))
                }
            }
        };
        p.acted = true;
        p.last_action = Some(label);

        // Có người cược vượt mức hiện tại: những người khác phải hành động lại.
        let street_bet = p.street_bet;
        if street_bet > current_bet {
            self.min_raise = self.min_raise.max(street_bet - current_bet);
            self.current_bet = street_bet;
            for (j, other) in self.players.iter_mut().enumerate() {
                if j != i {
                    other.acted = false;
                }
            }
        }

        self.event(kind, text, Some(id));
        Ok(self.advance(i))
    }

    fn advance(&mut self, from: usize) -> Vec<Effect> {
        let live = self.live_indices();
        if live.len() == 1 {
            return self.award_uncontested(live[0]);
        }
        let next = self
            .clockwise_after(self.players[from].seat)
            .into_iter()
            .find(|&j| {
                let p = &self.players[j];
                p.can_act() && (!p.acted || p.street_bet < self.current_bet)
            });
        match next {
            Some(j) => {
                let id = self.players[j].id;
                vec![self.set_turn(id)]
            }
            None => self.end_street(),
        }
    }

    fn end_street(&mut self) -> Vec<Effect> {
        self.turn = None;
        self.turn_deadline = None;
        if self.street >= 5 {
            return self.showdown();
        }
        self.deal_up_round();
        self.begin_street()
    }

    fn deal_up_round(&mut self) {
        for i in self.clockwise_after(self.dealer_seat) {
            if self.players[i].live() {
                let card = self.deck.deal();
                self.players[i].up.push(card);
            }
        }
        self.street += 1;
        self.event("deal", format!("Chia lá thứ {}", self.street), None);
    }

    fn run_out(&mut self) -> Vec<Effect> {
        self.turn = None;
        self.turn_deadline = None;
        while self.street < 5 {
            self.deal_up_round();
        }
        self.showdown()
    }

    fn showdown(&mut self) -> Vec<Effect> {
        self.phase = Phase::Showdown;
        self.turn = None;
        let live = self.live_indices();
        let values: Vec<Option<HandValue>> = self
            .players
            .iter()
            .map(|p| p.live().then(|| evaluate(&p.all_cards())))
            .collect();
        let mut won = vec![0u64; self.players.len()];

        // Chia hũ theo từng mức góp tiền của người còn lại (hũ chính + hũ phụ).
        let mut levels: Vec<u64> = live.iter().map(|&i| self.players[i].total_bet).collect();
        levels.sort_unstable();
        levels.dedup();
        let order = self.clockwise_after(self.dealer_seat);
        let mut prev = 0;
        for (k, &level) in levels.iter().enumerate() {
            let mut amount: u64 = self
                .players
                .iter()
                .map(|p| p.total_bet.min(level) - p.total_bet.min(prev))
                .sum();
            if k == levels.len() - 1 {
                amount += self.players.iter().map(|p| p.total_bet.saturating_sub(level)).sum::<u64>();
            }
            prev = level;

            let eligible: Vec<usize> = order
                .iter()
                .copied()
                .filter(|&i| self.players[i].live() && self.players[i].total_bet >= level)
                .collect();
            let best = eligible.iter().filter_map(|&i| values[i].as_ref()).max().cloned();
            let winners: Vec<usize> = eligible
                .into_iter()
                .filter(|&i| values[i].as_ref() == best.as_ref())
                .collect();
            if winners.is_empty() {
                continue;
            }
            let share = amount / winners.len() as u64;
            let mut rest = amount % winners.len() as u64;
            for w in winners {
                won[w] += share + if rest > 0 { 1 } else { 0 };
                rest = rest.saturating_sub(1);
            }
        }

        let mut rows = Vec::new();
        let mut headline = Vec::new();
        for &i in &live {
            let p = &mut self.players[i];
            p.chips += won[i];
            p.revealed = true;
            let name = hand_name(values[i].as_ref().unwrap());
            if won[i] > 0 {
                headline.push(format!("{} thắng {} ({})", p.name, won[i], name));
            }
            rows.push(json!({
                "id": p.id,
                "name": p.name,
                "hand": name,
                "cards": p.all_cards(),
                "won": won[i],
            }));
        }
        let text = headline.join(" · ");
        self.result = Some(json!({ "hand_no": self.hand_no, "text": text, "rows": rows }));
        self.event("win", text, None);
        vec![Effect::NextHand { hand_no: self.hand_no, ms: NEXT_HAND_MS }]
    }

    fn award_uncontested(&mut self, winner: usize) -> Vec<Effect> {
        let pot: u64 = self.players.iter().map(|p| p.total_bet).sum();
        self.phase = Phase::Showdown;
        self.turn = None;
        self.turn_deadline = None;
        let p = &mut self.players[winner];
        p.chips += pot;
        let text = format!("{} ăn hũ {} (mọi người úp bỏ)", p.name, pot);
        self.result = Some(json!({
            "hand_no": self.hand_no,
            "text": text,
            "rows": [{ "id": p.id, "name": p.name, "hand": "Không cần lật bài", "cards": p.up, "won": pot }],
        }));
        self.event("win", text, None);
        vec![Effect::NextHand { hand_no: self.hand_no, ms: NEXT_HAND_MS }]
    }

    // ------------------------------------------------------------------
    // Hẹn giờ (được main.rs gọi lại). Trả về None nếu timer đã lỗi thời.
    // ------------------------------------------------------------------

    pub fn on_turn_timeout(&mut self, token: u64) -> Option<Vec<Effect>> {
        if self.phase != Phase::Betting || token != self.turn_token {
            return None;
        }
        let id = self.turn?;
        let i = self.idx(id)?;
        let to_call = self.current_bet.saturating_sub(self.players[i].street_bet);
        self.event("info", format!("{} hết giờ suy nghĩ", self.players[i].name), None);
        let action = if to_call == 0 { Action::Call } else { Action::Fold };
        Some(self.act(id, action).unwrap_or_default())
    }

    pub fn on_next_hand(&mut self, hand_no: u64) -> Option<Vec<Effect>> {
        if self.phase != Phase::Showdown || self.hand_no != hand_no {
            return None;
        }
        Some(self.start_hand().unwrap_or_else(|msg| {
            self.event("info", msg, None);
            vec![]
        }))
    }

    // ------------------------------------------------------------------
    // Gửi tin
    // ------------------------------------------------------------------

    pub fn event(&self, kind: &str, text: String, player: Option<u64>) {
        let msg = json!({ "type": "event", "kind": kind, "text": text, "player": player });
        for p in &self.players {
            p.send(&msg);
        }
    }

    pub fn broadcast_state(&self) {
        for p in &self.players {
            p.send(&self.state_for(p.id));
        }
    }

    /// Trạng thái riêng cho từng người xem: lá tẩy của người khác luôn bị ẩn
    /// cho tới khi lật bài.
    fn state_for(&self, viewer: u64) -> Value {
        let me = self.players.iter().find(|p| p.id == viewer);
        let to_call = me.map_or(0, |p| self.current_bet.saturating_sub(p.street_bet));
        let max_raise = me.map_or(0, |p| p.chips.saturating_sub(to_call));
        let players: Vec<Value> = self
            .players
            .iter()
            .map(|p| {
                let show_hole = p.id == viewer || p.revealed;
                json!({
                    "id": p.id,
                    "name": p.name,
                    "seat": p.seat,
                    "chips": p.chips,
                    "street_bet": p.street_bet,
                    "total_bet": p.total_bet,
                    "in_hand": p.in_hand,
                    "folded": p.folded,
                    "all_in": p.all_in,
                    "connected": p.connected,
                    "has_hole": p.hole.is_some(),
                    "hole": if show_hole { p.hole } else { None },
                    "up": p.up,
                    "visible_hand": (!p.up.is_empty()).then(|| hand_name(&evaluate(&p.up))),
                    "last_action": p.last_action,
                    "is_dealer": self.hand_no > 0 && p.seat == self.dealer_seat,
                })
            })
            .collect();
        let pot: u64 = if self.phase == Phase::Betting {
            self.players.iter().map(|p| p.total_bet).sum()
        } else {
            0
        };
        json!({
            "type": "state",
            "room": self.id,
            "you": viewer,
            "phase": self.phase.as_str(),
            "hand_no": self.hand_no,
            "street": self.street,
            "pot": pot,
            "current_bet": self.current_bet,
            "min_raise": self.min_raise,
            "to_call": to_call,
            "max_raise": max_raise,
            "turn": self.turn,
            "turn_ms_left": self.turn_deadline.map(|d| d.saturating_duration_since(Instant::now()).as_millis() as u64),
            "turn_ms_total": TURN_MS,
            "can_start": self.phase != Phase::Betting && self.players.iter().filter(|p| p.connected).count() >= 2,
            "players": players,
            "result": self.result,
        })
    }

    // ------------------------------------------------------------------
    // Tiện ích
    // ------------------------------------------------------------------

    fn idx(&self, id: u64) -> Option<usize> {
        self.players.iter().position(|p| p.id == id)
    }

    fn live_indices(&self) -> Vec<usize> {
        (0..self.players.len()).filter(|&i| self.players[i].live()).collect()
    }

    /// Chỉ số người chơi theo chiều kim đồng hồ, bắt đầu từ ghế ngay sau `seat`
    /// (người ngồi ở `seat`, nếu có, đứng cuối danh sách).
    fn clockwise_after(&self, seat: usize) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.players.len()).collect();
        order.sort_by_key(|&i| (self.players[i].seat + 2 * MAX_PLAYERS - seat - 1) % MAX_PLAYERS);
        order
    }
}
