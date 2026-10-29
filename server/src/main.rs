//! Máy chủ Xì Tố 5 lá.
//!
//! - `GET /ws`  : WebSocket, giao thức JSON (xem README.md)
//! - `GET /*`   : phục vụ thư mục client tĩnh (mặc định ../client)
//!
//! Biến môi trường: PORT (mặc định 3000), CLIENT_DIR (mặc định ../client),
//! SUPABASE_URL + SUPABASE_SERVICE_KEY (lưu tài khoản; thiếu thì lưu trong bộ nhớ).

mod cards;
mod db;
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

use db::{Account, Db, DbError};
use game::{Action, Effect, Room, Seat};

/// Chuỗi đặc biệt đẩy vào kênh ghi để đóng socket (khi bị đăng nhập nơi khác).
const CLOSE: &str = "\u{0}close";
/// Ảnh đại diện là một số hạt giống; giữ trong khoảng an toàn cho cột integer.
const AVATAR_MAX: u32 = 1_000_000;

/// Một kết nối WebSocket.
struct Conn {
    tx: UnboundedSender<String>,
    account: Option<Account>,
    token: Option<String>,
    room: Option<String>,
}

#[derive(Default)]
struct Lobby {
    rooms: HashMap<String, Room>,
    conns: HashMap<u64, Conn>,
    /// account id -> id kết nối đang đăng nhập (mỗi tài khoản chỉ một nơi).
    online: HashMap<String, u64>,
    next_id: u64,
}

struct App {
    lobby: Mutex<Lobby>,
    db: Arc<Db>,
    /// Hàng đợi ghi chip xuống DB (một task ghi tuần tự để không ghi đè sai thứ tự).
    saver: UnboundedSender<(String, u64)>,
    /// Số chip mới nhất đã gửi đi lưu, phòng khi DB chưa ghi xong mà người chơi đã vào lại.
    latest_chips: Mutex<HashMap<String, u64>>,
}

impl App {
    fn save_chips(&self, account: String, chips: u64) {
        self.latest_chips.lock().unwrap().insert(account.clone(), chips);
        let _ = self.saver.send((account, chips));
    }

    fn chips_of(&self, acc: &Account) -> u64 {
        self.latest_chips.lock().unwrap().get(&acc.id).copied().unwrap_or(acc.chips)
    }
}

type Shared = Arc<App>;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMsg {
    Register {
        username: String,
        password: String,
        #[serde(default)]
        name: String,
    },
    Login { username: String, password: String },
    Resume { token: String },
    Logout,
    Profile { name: String, avatar: u32 },
    Join { room: String },
    Leave,
    Start,
    Fold,
    Call,
    Raise { amount: u64 },
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000);
    let client_dir = std::env::var("CLIENT_DIR").unwrap_or_else(|_| "../client".into());

    let db = Arc::new(Db::from_env());
    let (saver, mut save_rx) = mpsc::unbounded_channel::<(String, u64)>();
    let save_db = db.clone();
    tokio::spawn(async move {
        while let Some((id, chips)) = save_rx.recv().await {
            if let Err(e) =
                save_db.update_account(&id, json!({ "chips": chips })).await
            {
                eprintln!("Không lưu được chip của {id}: {e}");
            }
        }
    });
    let shared: Shared = Arc::new(App {
        lobby: Mutex::default(),
        db,
        saver,
        latest_chips: Mutex::default(),
    });

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
            if text == CLOSE {
                let _ = sink.send(Message::Close(None)).await;
                break;
            }
            if sink.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    });

    let id = {
        let mut lobby = shared.lobby.lock().unwrap();
        lobby.next_id += 1;
        let id = lobby.next_id;
        lobby.conns.insert(id, Conn { tx: tx.clone(), account: None, token: None, room: None });
        id
    };

    while let Some(Ok(msg)) = stream.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        match serde_json::from_str::<ClientMsg>(&text) {
            Ok(cmd) => handle_msg(&shared, id, &tx, cmd).await,
            Err(_) => send_error(&tx, "Lệnh không hợp lệ"),
        }
    }

    disconnect(&shared, &mut shared.lobby.lock().unwrap(), id);
    writer.abort();
}

// ----------------------------------------------------------------------
// Tài khoản (bất đồng bộ vì phải gọi DB; không giữ khoá lobby qua .await)
// ----------------------------------------------------------------------

async fn handle_msg(app: &Shared, id: u64, tx: &UnboundedSender<String>, cmd: ClientMsg) {
    let signed_in = || app.lobby.lock().unwrap().conns.get(&id).is_some_and(|c| c.account.is_some());
    match cmd {
        ClientMsg::Register { username, password, name } => {
            if signed_in() {
                return send_error(tx, "Bạn đã đăng nhập");
            }
            let Some(username) = valid_username(&username) else {
                return send_error(tx, "Tên đăng nhập gồm 3–16 ký tự a-z, 0-9 hoặc _");
            };
            if !(6..=64).contains(&password.chars().count()) {
                return send_error(tx, "Mật khẩu cần 6–64 ký tự");
            }
            let name = clean(&name, 12, &username);
            let hash = tokio::task::spawn_blocking(move || db::hash_password(&password)).await.unwrap();
            let avatar = rand::random::<u32>() % AVATAR_MAX;
            match app.db.create_user(&username, hash, &name, avatar).await {
                Ok(acc) => start_session(app, id, tx, acc).await,
                Err(DbError::Taken) => send_error(tx, "Tên đăng nhập đã có người dùng"),
                Err(DbError::Other(e)) => db_failed(tx, e),
            }
        }
        ClientMsg::Login { username, password } => {
            if signed_in() {
                return send_error(tx, "Bạn đã đăng nhập");
            }
            let found = match app.db.find_user(&username.trim().to_lowercase()).await {
                Ok(f) => f,
                Err(e) => return db_failed(tx, e.to_string()),
            };
            let account = match found {
                Some((acc, hash)) => tokio::task::spawn_blocking(move || db::verify_password(&password, &hash))
                    .await
                    .unwrap_or(false)
                    .then_some(acc),
                None => None,
            };
            match account {
                Some(acc) => start_session(app, id, tx, acc).await,
                None => {
                    // Làm chậm việc dò mật khẩu.
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    send_error(tx, "Sai tên đăng nhập hoặc mật khẩu");
                }
            }
        }
        ClientMsg::Resume { token } => {
            if signed_in() {
                return;
            }
            match app.db.session_account(&token).await {
                Ok(Some(acc)) => sign_in(app, id, tx, acc, token),
                Ok(None) => send_error_code(tx, "auth", "Phiên đăng nhập đã hết hạn, hãy đăng nhập lại"),
                Err(e) => db_failed(tx, e.to_string()),
            }
        }
        ClientMsg::Logout => {
            let token = sign_out(app, &mut app.lobby.lock().unwrap(), id);
            if let Some(t) = token {
                if let Err(e) = app.db.delete_session(&t).await {
                    eprintln!("Không xoá được phiên: {e}");
                }
            }
            let _ = tx.send(json!({ "type": "logged_out" }).to_string());
        }
        ClientMsg::Profile { name, avatar } => {
            let current = {
                let lobby = app.lobby.lock().unwrap();
                let Some(conn) = lobby.conns.get(&id) else { return };
                if conn.room.is_some() {
                    return send_error(tx, "Hãy rời bàn trước khi đổi hồ sơ");
                }
                conn.account.clone()
            };
            let Some(acc) = current else { return send_error(tx, "Hãy đăng nhập trước") };
            let name = clean(&name, 12, &acc.username);
            let avatar = avatar % AVATAR_MAX;
            if let Err(e) = app.db.update_account(&acc.id, json!({ "name": name, "avatar": avatar })).await {
                return db_failed(tx, e.to_string());
            }
            let mut lobby = app.lobby.lock().unwrap();
            if let Some(Conn { account: Some(a), .. }) = lobby.conns.get_mut(&id) {
                a.name = name;
                a.avatar = avatar;
                send_profile(tx, a, app.chips_of(a));
                let _ = tx.send(json!({ "type": "info", "msg": "Đã lưu hồ sơ" }).to_string());
            }
        }
        other => handle_cmd(app, id, tx, other),
    }
}

async fn start_session(app: &Shared, id: u64, tx: &UnboundedSender<String>, acc: Account) {
    match app.db.create_session(&acc).await {
        Ok(token) => sign_in(app, id, tx, acc, token),
        Err(e) => db_failed(tx, e.to_string()),
    }
}

fn sign_in(app: &Shared, id: u64, tx: &UnboundedSender<String>, acc: Account, token: String) {
    let mut lobby = app.lobby.lock().unwrap();
    if !lobby.conns.get(&id).is_some_and(|c| c.account.is_none()) {
        return; // socket đã đóng, hoặc đã đăng nhập bằng lệnh khác
    }
    // Đăng nhập nơi mới thì đá phiên cũ ra (chip của phiên cũ được lưu khi rời bàn).
    if let Some(&old) = lobby.online.get(&acc.id) {
        if let Some(conn) = disconnect(app, &mut lobby, old) {
            send_error_code(&conn.tx, "kicked", "Tài khoản vừa đăng nhập ở nơi khác");
            let _ = conn.tx.send(CLOSE.into());
        }
    }
    lobby.online.insert(acc.id.clone(), id);
    let _ = tx.send(json!({ "type": "auth", "token": token, "profile": profile_json(&acc, app.chips_of(&acc)) }).to_string());
    let conn = lobby.conns.get_mut(&id).unwrap();
    conn.account = Some(acc);
    conn.token = Some(token);
}

/// Đăng xuất trên kết nối này; trả về token để xoá phiên trong DB.
fn sign_out(app: &Shared, lobby: &mut Lobby, id: u64) -> Option<String> {
    leave_room(app, lobby, id);
    let conn = lobby.conns.get_mut(&id)?;
    let acc = conn.account.take()?;
    let token = conn.token.take();
    if lobby.online.get(&acc.id) == Some(&id) {
        lobby.online.remove(&acc.id);
    }
    token
}

/// Gỡ hẳn một kết nối: rời bàn (lưu chip) và thôi giữ trạng thái đăng nhập.
fn disconnect(app: &Shared, lobby: &mut Lobby, id: u64) -> Option<Conn> {
    leave_room(app, lobby, id);
    let conn = lobby.conns.remove(&id)?;
    if let Some(acc) = &conn.account {
        if lobby.online.get(&acc.id) == Some(&id) {
            lobby.online.remove(&acc.id);
        }
    }
    Some(conn)
}

fn leave_room(app: &Shared, lobby: &mut Lobby, id: u64) {
    let Some(conn) = lobby.conns.get_mut(&id) else { return };
    let Some(rid) = conn.room.take() else { return };
    let account = conn.account.as_ref().map(|a| a.id.clone());
    let Some(room) = lobby.rooms.get_mut(&rid) else { return };
    // Đang giữa ván thì phần đã cược nằm lại trong hũ; số chip còn lại là số chốt.
    if let (Some(acc), Some(chips)) = (account, room.chips_of(id)) {
        app.save_chips(acc, chips);
    }
    let effects = room.remove_player(id);
    room.broadcast_state();
    apply_effects(app, &rid, effects);
    if room.is_empty() {
        lobby.rooms.remove(&rid);
    }
}

// ----------------------------------------------------------------------
// Lệnh trong bàn chơi
// ----------------------------------------------------------------------

fn handle_cmd(app: &Shared, id: u64, tx: &UnboundedSender<String>, cmd: ClientMsg) {
    let mut guard = app.lobby.lock().unwrap();
    let lobby = &mut *guard;
    let Some(conn) = lobby.conns.get(&id) else { return };
    let Some(acc) = conn.account.clone() else {
        return send_error(tx, "Hãy đăng nhập trước");
    };
    let room_id = conn.room.clone();

    match cmd {
        ClientMsg::Join { room } => {
            if room_id.is_some() {
                return send_error(tx, "Bạn đã ở trong phòng");
            }
            let rid = clean(&room, 16, "sanh-chinh");
            let seat = Seat {
                name: acc.name.clone(),
                avatar: acc.avatar,
                account: acc.id.clone(),
                chips: app.chips_of(&acc),
            };
            let room = lobby.rooms.entry(rid.clone()).or_insert_with(|| Room::new(rid.clone()));
            match room.add_player(id, seat, tx.clone()) {
                Ok(()) => {
                    let _ = tx.send(json!({ "type": "welcome", "id": id, "room": rid }).to_string());
                    room.broadcast_state();
                    lobby.conns.get_mut(&id).unwrap().room = Some(rid);
                }
                Err(e) => send_error(tx, &e),
            }
        }
        ClientMsg::Leave => {
            leave_room(app, lobby, id);
            send_profile(tx, &acc, app.chips_of(&acc));
        }
        cmd => {
            let Some(rid) = room_id else {
                return send_error(tx, "Hãy vào phòng trước");
            };
            let Some(room) = lobby.rooms.get_mut(&rid) else { return };
            let result = match cmd {
                ClientMsg::Start => room.start_hand(),
                ClientMsg::Fold => room.act(id, Action::Fold),
                ClientMsg::Call => room.act(id, Action::Call),
                ClientMsg::Raise { amount } => room.act(id, Action::Raise(amount)),
                _ => return,
            };
            match result {
                Ok(effects) => {
                    room.broadcast_state();
                    apply_effects(app, &rid, effects);
                }
                Err(e) => send_error(tx, &e),
            }
        }
    }
}

type TimerFn = Box<dyn FnOnce(&mut Room) -> Option<Vec<Effect>> + Send>;

/// Đặt các hẹn giờ mà logic game yêu cầu (hết lượt, sang ván mới) và lưu chip.
fn apply_effects(shared: &Shared, rid: &str, effects: Vec<Effect>) {
    for effect in effects {
        let (ms, fire): (u64, TimerFn) = match effect {
            Effect::TurnTimer { token, ms } => (ms, Box::new(move |r: &mut Room| r.on_turn_timeout(token))),
            Effect::NextHand { hand_no, ms } => (ms, Box::new(move |r: &mut Room| r.on_next_hand(hand_no))),
            Effect::SaveChips(list) => {
                for (account, chips) in list {
                    shared.save_chips(account, chips);
                }
                continue;
            }
        };
        let shared = shared.clone();
        let rid = rid.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            let mut lobby = shared.lobby.lock().unwrap();
            if let Some(room) = lobby.rooms.get_mut(&rid) {
                if let Some(next) = fire(room) {
                    room.broadcast_state();
                    apply_effects(&shared, &rid, next);
                }
            }
        });
    }
}

// ----------------------------------------------------------------------
// Tiện ích
// ----------------------------------------------------------------------

fn profile_json(acc: &Account, chips: u64) -> serde_json::Value {
    json!({ "username": acc.username, "name": acc.name, "avatar": acc.avatar, "chips": chips })
}

fn send_profile(tx: &UnboundedSender<String>, acc: &Account, chips: u64) {
    let _ = tx.send(json!({ "type": "profile", "profile": profile_json(acc, chips) }).to_string());
}

fn send_error(tx: &UnboundedSender<String>, msg: &str) {
    let _ = tx.send(json!({ "type": "error", "msg": msg }).to_string());
}

/// Lỗi kèm mã để client xử lý riêng: `auth` (token hỏng), `kicked` (đăng nhập nơi khác).
fn send_error_code(tx: &UnboundedSender<String>, code: &str, msg: &str) {
    let _ = tx.send(json!({ "type": "error", "code": code, "msg": msg }).to_string());
}

fn db_failed(tx: &UnboundedSender<String>, e: String) {
    eprintln!("Lỗi cơ sở dữ liệu: {e}");
    send_error(tx, "Máy chủ dữ liệu đang lỗi, thử lại sau");
}

fn valid_username(s: &str) -> Option<String> {
    let s = s.trim().to_lowercase();
    let ok = (3..=16).contains(&s.len()) && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    ok.then_some(s)
}

fn clean(s: &str, max: usize, fallback: &str) -> String {
    let s: String = s.trim().chars().filter(|c| !c.is_control()).take(max).collect();
    if s.is_empty() { fallback.into() } else { s }
}
