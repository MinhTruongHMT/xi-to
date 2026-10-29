//! Lưu tài khoản người chơi.
//!
//! - Có SUPABASE_URL + SUPABASE_SERVICE_KEY: gọi REST API (PostgREST) của Supabase.
//!   Bảng xem `supabase/schema.sql`. Chỉ server giữ khoá này; client không bao giờ
//!   chạm trực tiếp vào cơ sở dữ liệu.
//! - Không có: lưu tạm trong bộ nhớ (để chạy thử trên máy), mất khi tắt server.

use std::{collections::HashMap, sync::Mutex};

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand::Rng;
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::game::START_CHIPS;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub username: String,
    pub name: String,
    pub avatar: u32,
    pub chips: u64,
}

#[derive(Deserialize)]
struct AccountRow {
    #[serde(flatten)]
    account: Account,
    password_hash: String,
}

pub enum DbError {
    /// Tên đăng nhập đã tồn tại.
    Taken,
    Other(String),
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            DbError::Taken => f.write_str("trùng tên đăng nhập"),
            DbError::Other(e) => f.write_str(e),
        }
    }
}

impl From<reqwest::Error> for DbError {
    fn from(e: reqwest::Error) -> Self {
        DbError::Other(e.to_string())
    }
}

pub enum Db {
    Supabase { http: reqwest::Client, url: String, key: String },
    Memory(Mutex<Mem>),
}

#[derive(Default)]
pub struct Mem {
    /// username -> (tài khoản, mật khẩu đã băm)
    accounts: HashMap<String, (Account, String)>,
    /// token đã băm -> username
    sessions: HashMap<String, String>,
}

impl Db {
    pub fn from_env() -> Self {
        let url = std::env::var("SUPABASE_URL").ok().filter(|s| !s.is_empty());
        let key = std::env::var("SUPABASE_SERVICE_KEY").ok().filter(|s| !s.is_empty());
        match (url, key) {
            (Some(url), Some(key)) => Db::Supabase {
                http: reqwest::Client::new(),
                url: url.trim_end_matches('/').to_string(),
                key,
            },
            _ => {
                println!("⚠️  Chưa đặt SUPABASE_URL / SUPABASE_SERVICE_KEY: tài khoản chỉ lưu trong bộ nhớ.");
                Db::Memory(Mutex::default())
            }
        }
    }

    fn req(http: &reqwest::Client, url: &str, key: &str, method: Method, path: &str) -> RequestBuilder {
        let r = http.request(method, format!("{url}/rest/v1/{path}")).header("apikey", key);
        // Khoá kiểu mới (sb_secret_...) chỉ đi qua header apikey; khoá service_role cũ là JWT.
        if key.starts_with("sb_") { r } else { r.bearer_auth(key) }
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Result<Vec<Value>, DbError> {
        let Db::Supabase { http, url, key } = self else { unreachable!() };
        let mut r = Self::req(http, url, key, method, path).header("Prefer", "return=representation");
        if let Some(b) = body {
            r = r.json(&b);
        }
        let res = r.send().await?;
        match res.status() {
            StatusCode::CONFLICT => Err(DbError::Taken),
            s if s.is_success() => Ok(res.json::<Vec<Value>>().await.unwrap_or_default()),
            s => Err(DbError::Other(format!("Supabase {s}: {}", res.text().await.unwrap_or_default()))),
        }
    }

    pub async fn find_user(&self, username: &str) -> Result<Option<(Account, String)>, DbError> {
        if let Db::Memory(m) = self {
            return Ok(m.lock().unwrap().accounts.get(username).cloned());
        }
        let rows = self.call(Method::GET, &format!("accounts?username=eq.{username}&select=*"), None).await?;
        Ok(rows
            .into_iter()
            .next()
            .and_then(|v| serde_json::from_value::<AccountRow>(v).ok())
            .map(|r| (r.account, r.password_hash)))
    }

    pub async fn create_user(&self, username: &str, password_hash: String, name: &str, avatar: u32) -> Result<Account, DbError> {
        if let Db::Memory(m) = self {
            let mut m = m.lock().unwrap();
            if m.accounts.contains_key(username) {
                return Err(DbError::Taken);
            }
            let acc = Account {
                id: format!("mem-{}", m.accounts.len() + 1),
                username: username.into(),
                name: name.into(),
                avatar,
                chips: START_CHIPS,
            };
            m.accounts.insert(username.into(), (acc.clone(), password_hash));
            return Ok(acc);
        }
        let body = json!({
            "username": username,
            "password_hash": password_hash,
            "name": name,
            "avatar": avatar,
            "chips": START_CHIPS,
        });
        let rows = self.call(Method::POST, "accounts", Some(body)).await?;
        rows.into_iter()
            .next()
            .and_then(|v| serde_json::from_value::<Account>(v).ok())
            .ok_or_else(|| DbError::Other("Supabase không trả về tài khoản vừa tạo".into()))
    }

    /// Cập nhật một phần tài khoản (`name`, `avatar`, `chips`).
    pub async fn update_account(&self, id: &str, fields: Value) -> Result<(), DbError> {
        if let Db::Memory(m) = self {
            if let Some((a, _)) = m.lock().unwrap().accounts.values_mut().find(|(a, _)| a.id == id) {
                if let Some(n) = fields["name"].as_str() {
                    a.name = n.into();
                }
                if let Some(v) = fields["avatar"].as_u64() {
                    a.avatar = v as u32;
                }
                if let Some(c) = fields["chips"].as_u64() {
                    a.chips = c;
                }
            }
            return Ok(());
        }
        self.call(Method::PATCH, &format!("accounts?id=eq.{id}"), Some(fields)).await?;
        Ok(())
    }

    /// Tạo phiên đăng nhập mới, trả về token gốc (chỉ bản băm được lưu lại).
    pub async fn create_session(&self, account: &Account) -> Result<String, DbError> {
        let token: String = {
            let bytes: [u8; 32] = rand::thread_rng().gen();
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        };
        let hash = token_hash(&token);
        if let Db::Memory(m) = self {
            m.lock().unwrap().sessions.insert(hash, account.username.clone());
        } else {
            self.call(Method::POST, "sessions", Some(json!({ "token_hash": hash, "account_id": account.id })))
                .await?;
        }
        Ok(token)
    }

    pub async fn session_account(&self, token: &str) -> Result<Option<Account>, DbError> {
        let hash = token_hash(token);
        if let Db::Memory(m) = self {
            let m = m.lock().unwrap();
            return Ok(m.sessions.get(&hash).and_then(|u| m.accounts.get(u)).map(|(a, _)| a.clone()));
        }
        let rows = self
            .call(Method::GET, &format!("sessions?token_hash=eq.{hash}&select=account:accounts(id,username,name,avatar,chips)"), None)
            .await?;
        Ok(rows
            .into_iter()
            .next()
            .and_then(|mut v| serde_json::from_value::<Account>(v["account"].take()).ok()))
    }

    pub async fn delete_session(&self, token: &str) -> Result<(), DbError> {
        let hash = token_hash(token);
        if let Db::Memory(m) = self {
            m.lock().unwrap().sessions.remove(&hash);
            return Ok(());
        }
        self.call(Method::DELETE, &format!("sessions?token_hash=eq.{hash}"), None).await?;
        Ok(())
    }
}

fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Băm mật khẩu bằng Argon2 (tốn CPU, nên gọi trong spawn_blocking).
pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default().hash_password(password.as_bytes(), &salt).unwrap().to_string()
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}
