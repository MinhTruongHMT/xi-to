//! Máy chủ Xì Tố 5 lá.
//!
//! - `GET /ws`  : WebSocket, giao thức JSON (xem README.md)
//! - `GET /*`   : phục vụ thư mục client tĩnh (mặc định ../client)
//!
//! Biến môi trường: PORT (mặc định 3000), CLIENT_DIR (mặc định ../client).

mod cards;
mod game;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc::{self, UnboundedSender};
use tower_http::services::ServeDir;

use game::{Action, Effect, Room};

#[derive(Default)]
struct Lobby {
    rooms: HashMap<String, Room>,
    next_id: u64,
}

type Shared = Arc<Mutex<Lobby>>;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMsg {
    Join { name: String, room: String },
    Start,
    Fold,
    Call,
    Raise { amount: u64 },
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000);
    let client_dir = std::env::var("CLIENT_DIR").unwrap_or_else(|_| "../client".into());
    let shared: Shared = Arc::default();

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback_service(ServeDir::new(&client_dir))
        .with_state(shared);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("không mở được cổng");
    println!("🃏 Xì Tố server đang chạy:");
    println!("   Giao diện : http://localhost:{port}  (thư mục {client_dir})");
    println!("   WebSocket : ws://localhost:{port}/ws");
    axum::serve(listener, app).await.unwrap();
}

async fn ws_handler(ws: WebSocketUpgrade, State(shared): State<Shared>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, shared))
}

async fn handle_socket(socket: WebSocket, shared: Shared) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    // Mỗi kết nối có một task ghi riêng; logic game chỉ việc đẩy chuỗi vào kênh.
    let writer = tokio::spawn(async move {
        while let Some(text) = rx.recv().await {
            if sink.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    });

    let id = {
        let mut lobby = shared.lock().unwrap();
        lobby.next_id += 1;
        lobby.next_id
    };
    let mut room_id: Option<String> = None;

    while let Some(Ok(msg)) = stream.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        match serde_json::from_str::<ClientMsg>(&text) {
            Ok(cmd) => handle_cmd(&shared, id, &tx, &mut room_id, cmd),
            Err(_) => send_error(&tx, "Lệnh không hợp lệ"),
        }
    }

    if let Some(rid) = room_id {
        let mut lobby = shared.lock().unwrap();
        if let Some(room) = lobby.rooms.get_mut(&rid) {
            let effects = room.remove_player(id);
            room.broadcast_state();
            apply_effects(&shared, &rid, effects);
            if room.is_empty() {
                lobby.rooms.remove(&rid);
            }
        }
    }
    writer.abort();
}

fn handle_cmd(
    shared: &Shared,
    id: u64,
    tx: &UnboundedSender<String>,
    room_id: &mut Option<String>,
    cmd: ClientMsg,
) {
    let mut lobby = shared.lock().unwrap();

    if let ClientMsg::Join { name, room } = cmd {
        if room_id.is_some() {
            return send_error(tx, "Bạn đã ở trong phòng");
        }
        let name = clean(&name, 12, "Vô danh");
        let rid = clean(&room, 16, "sanh-chinh");
        let room = lobby.rooms.entry(rid.clone()).or_insert_with(|| Room::new(rid.clone()));
        match room.add_player(id, name, tx.clone()) {
            Ok(()) => {
                let _ = tx.send(json!({ "type": "welcome", "id": id, "room": rid }).to_string());
                room.broadcast_state();
                *room_id = Some(rid);
            }
            Err(e) => send_error(tx, &e),
        }
        return;
    }

    let Some(rid) = room_id.as_deref() else {
        return send_error(tx, "Hãy vào phòng trước");
    };
    let Some(room) = lobby.rooms.get_mut(rid) else { return };
    let result = match cmd {
        ClientMsg::Start => room.start_hand(),
        ClientMsg::Fold => room.act(id, Action::Fold),
        ClientMsg::Call => room.act(id, Action::Call),
        ClientMsg::Raise { amount } => room.act(id, Action::Raise(amount)),
        ClientMsg::Join { .. } => unreachable!(),
    };
    match result {
        Ok(effects) => {
            room.broadcast_state();
            apply_effects(shared, rid, effects);
        }
        Err(e) => send_error(tx, &e),
    }
}

type TimerFn = Box<dyn FnOnce(&mut Room) -> Option<Vec<Effect>> + Send>;

/// Đặt các hẹn giờ mà logic game yêu cầu (hết lượt, sang ván mới).
fn apply_effects(shared: &Shared, rid: &str, effects: Vec<Effect>) {
    for effect in effects {
        let (ms, fire): (u64, TimerFn) = match effect {
            Effect::TurnTimer { token, ms } => (ms, Box::new(move |r: &mut Room| r.on_turn_timeout(token))),
            Effect::NextHand { hand_no, ms } => (ms, Box::new(move |r: &mut Room| r.on_next_hand(hand_no))),
        };
        let shared = shared.clone();
        let rid = rid.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            let mut lobby = shared.lock().unwrap();
            if let Some(room) = lobby.rooms.get_mut(&rid) {
                if let Some(next) = fire(room) {
                    room.broadcast_state();
                    apply_effects(&shared, &rid, next);
                }
            }
        });
    }
}

fn send_error(tx: &UnboundedSender<String>, msg: &str) {
    let _ = tx.send(json!({ "type": "error", "msg": msg }).to_string());
}

fn clean(s: &str, max: usize, fallback: &str) -> String {
    let s: String = s.trim().chars().filter(|c| !c.is_control()).take(max).collect();
    if s.is_empty() { fallback.into() } else { s }
}
