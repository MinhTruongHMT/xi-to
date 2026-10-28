//! Bộ bài, xáo bài và đánh giá sức mạnh tay bài (1 đến 5 lá).

use rand::seq::SliceRandom;
use serde::Serialize;

/// Thứ tự chất theo luật Xì Tố Việt Nam: Bích < Chuồn < Rô < Cơ.
/// Chỉ dùng để phân định ai nói trước khi bài ngửa bằng nhau.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Suit {
    #[serde(rename = "S")]
    Spades,
    #[serde(rename = "C")]
    Clubs,
    #[serde(rename = "D")]
    Diamonds,
    #[serde(rename = "H")]
    Hearts,
}

const SUITS: [Suit; 4] = [Suit::Spades, Suit::Clubs, Suit::Diamonds, Suit::Hearts];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Card {
    /// 2..=14 (11 = J, 12 = Q, 13 = K, 14 = A)
    #[serde(rename = "r")]
    pub rank: u8,
    #[serde(rename = "s")]
    pub suit: Suit,
}

pub struct Deck(Vec<Card>);

impl Deck {
    pub fn shuffled() -> Self {
        let mut cards: Vec<Card> = SUITS
            .iter()
            .flat_map(|&suit| (2..=14).map(move |rank| Card { rank, suit }))
            .collect();
        cards.shuffle(&mut rand::thread_rng());
        Deck(cards)
    }

    pub fn deal(&mut self) -> Card {
        // 52 lá luôn đủ cho tối đa 6 người x 5 lá.
        self.0.pop().expect("bộ bài đã hết")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    HighCard,
    OnePair,
    TwoPair,
    Trips,
    Straight,
    Flush,
    FullHouse,
    Quads,
    StraightFlush,
}

/// So sánh được trực tiếp: trước tiên theo loại bài, sau đó theo các lá quyết định.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HandValue {
    pub category: Category,
    pub ranks: Vec<u8>,
}

/// Đánh giá 1..=5 lá. Sảnh/thùng chỉ được tính khi đủ 5 lá,
/// nên hàm này dùng được cho cả bài ngửa đang dở dang lẫn tay bài cuối.
pub fn evaluate(cards: &[Card]) -> HandValue {
    let mut counts = [0u8; 15];
    for c in cards {
        counts[c.rank as usize] += 1;
    }
    // Nhóm (số lượng, chất lượng) sắp giảm dần: đôi/sám xếp trước lá lẻ.
    let mut groups: Vec<(u8, u8)> = (2..=14u8)
        .filter(|&r| counts[r as usize] > 0)
        .map(|r| (counts[r as usize], r))
        .collect();
    groups.sort_by(|a, b| b.cmp(a));
    let ranks: Vec<u8> = groups.iter().map(|g| g.1).collect();

    if cards.len() == 5 {
        let flush = cards.iter().all(|c| c.suit == cards[0].suit);
        let straight = straight_high(&counts);
        if let (true, Some(high)) = (flush, straight) {
            return HandValue { category: Category::StraightFlush, ranks: vec![high] };
        }
        if groups[0].0 == 4 {
            return HandValue { category: Category::Quads, ranks };
        }
        if groups[0].0 == 3 && groups[1].0 == 2 {
            return HandValue { category: Category::FullHouse, ranks };
        }
        if flush {
            return HandValue { category: Category::Flush, ranks };
        }
        if let Some(high) = straight {
            return HandValue { category: Category::Straight, ranks: vec![high] };
        }
    }

    let category = match groups[0].0 {
        4 => Category::Quads,
        3 => Category::Trips,
        2 if groups.len() > 1 && groups[1].0 == 2 => Category::TwoPair,
        2 => Category::OnePair,
        _ => Category::HighCard,
    };
    HandValue { category, ranks }
}

fn straight_high(counts: &[u8; 15]) -> Option<u8> {
    let has = |r: u8| counts[if r == 1 { 14 } else { r as usize }] > 0;
    // A có thể đứng cuối (A-2-3-4-5, gọi là sảnh 5).
    (5..=14u8).rev().find(|&high| (high - 4..=high).all(has))
}

pub fn rank_label(rank: u8) -> String {
    match rank {
        11 => "J".into(),
        12 => "Q".into(),
        13 => "K".into(),
        14 => "A".into(),
        r => r.to_string(),
    }
}

pub fn hand_name(v: &HandValue) -> String {
    let r = |i: usize| rank_label(v.ranks[i]);
    match v.category {
        Category::HighCard => format!("Mậu thầu {}", r(0)),
        Category::OnePair => format!("Đôi {}", r(0)),
        Category::TwoPair => format!("Thú {}-{}", r(0), r(1)),
        Category::Trips => format!("Sám cô {}", r(0)),
        Category::Straight => format!("Sảnh {}", r(0)),
        Category::Flush => format!("Thùng {}", r(0)),
        Category::FullHouse => format!("Cù lũ {}", r(0)),
        Category::Quads => format!("Tứ quý {}", r(0)),
        Category::StraightFlush => format!("Thùng phá sảnh {}", r(0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(spec: &str) -> Vec<Card> {
        spec.split_whitespace()
            .map(|s| {
                let (r, su) = s.split_at(s.len() - 1);
                let rank = match r {
                    "J" => 11,
                    "Q" => 12,
                    "K" => 13,
                    "A" => 14,
                    n => n.parse().unwrap(),
                };
                let suit = match su {
                    "S" => Suit::Spades,
                    "C" => Suit::Clubs,
                    "D" => Suit::Diamonds,
                    _ => Suit::Hearts,
                };
                Card { rank, suit }
            })
            .collect()
    }

    #[test]
    fn categories() {
        assert_eq!(evaluate(&h("AS KS QS JS 10S")).category, Category::StraightFlush);
        assert_eq!(evaluate(&h("9S 9H 9D 9C 2S")).category, Category::Quads);
        assert_eq!(evaluate(&h("9S 9H 9D 2C 2S")).category, Category::FullHouse);
        assert_eq!(evaluate(&h("2H 7H 9H JH KH")).category, Category::Flush);
        assert_eq!(evaluate(&h("AS 2H 3D 4C 5S")).ranks, vec![5]);
        assert_eq!(evaluate(&h("KS KH 4D 4C 5S")).category, Category::TwoPair);
        assert_eq!(evaluate(&h("8S 8H")).category, Category::OnePair);
        assert_eq!(evaluate(&h("8S 9H 10D JC")).category, Category::HighCard);
    }

    #[test]
    fn ordering() {
        assert!(evaluate(&h("KS KH 2D 3C 4S")) > evaluate(&h("QS QH AD KC JS")));
        assert!(evaluate(&h("KS KH 5D 3C 2S")) > evaluate(&h("KD KC 4D 3S 2H")));
        assert!(evaluate(&h("2S 3H 4D 5C 6S")) > evaluate(&h("AS 2H 3D 4C 5S")));
        assert!(evaluate(&h("AS")) > evaluate(&h("KH")));
    }
}
