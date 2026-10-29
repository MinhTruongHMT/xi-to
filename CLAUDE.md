# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Multiplayer Five-Card Stud ("Xì Tố 5 lá"): a Rust WebSocket server (`server/`) plus a vanilla HTML/CSS/JS client (`client/`) with no build step. All user-facing text (UI, server event/error messages, README) is Vietnamese; keep it that way.

## Commands

```bash
# Run locally (the server also serves ../client, so open http://localhost:3000 in 2+ tabs, same room name)
cd server && cargo run --release          # env: PORT (default 3000), CLIENT_DIR (default ../client)

# Tests (only cards.rs has unit tests)
cd server && cargo test
cd server && cargo test cards::tests::ordering   # single test

# Docker (same image Render deploys; reads PORT)
docker build -t xito . && docker run --rm -p 3000:3000 xito
# Run tests inside the build stage
docker build --target build -t xito-build . && docker run --rm xito-build cargo test --release

# Client has no tooling; syntax check only
node --check client/script.js
```

### Building on this Windows machine
- Native `cargo build` on Windows fails here: Smart App Control blocks unsigned cargo build-script executables (os error 4551), MSVC Build Tools are not installed, and the default GNU toolchain lacks `dlltool`/`as`. Use Docker (preferred) or WSL with `CARGO_TARGET_DIR=$HOME/xito-target`.
- `server/Cargo.lock` intentionally pins `tokio 1.43`, `mio 1.0.3`, `socket2 0.5.8`, `parking_lot 0.12.3`, etc. so the dependency tree uses `windows-sys 0.52` (no raw-dylib/`dlltool` needed on the GNU toolchain). Don't run a blanket `cargo update` unless that workaround no longer matters.

## Deployment
Render via `render.yaml` (Blueprint, Docker runtime, free plan) + root `Dockerfile` (build context is the repo root; it copies `server/` and `client/`). Game state is in memory only, so a redeploy or free-tier sleep wipes all rooms and chips. `server/` must be a plain folder in git, not a submodule (a nested `server/.git` previously broke the Render build).

## Architecture

**Server is authoritative.** Clients only send intents (`join`, `start`, `fold`, `call`, `raise {amount}`); the server owns the deck, validates turn/stack/min-raise, and decides results.

- `server/src/game.rs`: `Room` holds all game state and rules, and it is synchronous with no tokio inside. Methods that advance the game return `Vec<Effect>` (`TurnTimer { token }`, `NextHand { hand_no }`) instead of scheduling anything themselves.
- `server/src/main.rs`: axum app. One global `Arc<App>` holding `Mutex<Lobby>` (rooms keyed by name, plus `conns` and `online`) and the `Db`. Each socket has an mpsc channel + writer task; game code just pushes JSON strings into `Player.tx`. `apply_effects` turns `Effect`s into `tokio::spawn` + sleep tasks that re-lock the lobby and call `Room::on_turn_timeout` / `on_next_hand`. Those return `None` when stale (checked via `turn_token` / `hand_no`), so old timers are harmless. Never hold the mutex across an `.await`.
- After every mutation the caller invokes `room.broadcast_state()`, which sends each player a **per-viewer** snapshot from `state_for(viewer)`. The hole (face-down) card is included only for its owner, or for everyone after showdown (`revealed`). It is also sent privately as a `hole` message at deal time. Preserve this filtering whenever you add state fields.
- Hand flow in `game.rs`: `start_hand` (ante, deal hole + 1 up) → `begin_street` (best visible hand acts first, ties broken by suit Spades < Clubs < Diamonds < Hearts via `visible_key`) → `act` → `advance` (next player who `can_act` and hasn't acted or is under `current_bet`; else `end_street`) → deal next up card … → `showdown` at 5 cards. `run_out` deals the remaining cards when ≤1 player can still bet. Side pots are computed in `showdown` from each player's `total_bet` levels.
- `Action::Raise(n)`: `n` is the amount **on top of** the call. Any raise above the current bet resets `acted` for everyone else.
- Disconnects mid-hand: the player is kept (their chips stay in the pot), marked `connected=false` and folded. They are pruned at the next `start_hand`.
- Accounts (`server/src/db.rs` + `main.rs`): login is mandatory before `join`. The server talks to Supabase PostgREST with `SUPABASE_URL` + `SUPABASE_SERVICE_KEY` (tables in `supabase/schema.sql`, RLS on with no policies); without them `Db::Memory` is used. Passwords are Argon2 (run in `spawn_blocking`); `register`/`login` issue a random token whose SHA-256 is stored in `sessions`, and `resume {token}` re-authenticates a new socket. `Lobby.conns` holds per-socket state (account, token, room); `Lobby.online` enforces one connection per account, and a new sign-in kicks the old one (`code: "kicked"` + the `CLOSE` sentinel on its channel). Chips: `Room` emits `Effect::SaveChips` at hand end, and `leave_room` saves on leave/disconnect. Both go through `App::save_chips`, which feeds a single sequential writer task and the `latest_chips` cache (used on join/sign-in so a pending write can't hand back stale chips). Auth handlers are async; never hold the lobby mutex across their `.await`s.
- `server/src/cards.rs`: `evaluate` works on 1–5 cards; straights and flushes only count at exactly 5 cards. That lets the same function rank partial up-card hands for turn order. `HandValue` derives `Ord`, so hands are compared directly.

**Client (`client/script.js`)** re-renders everything from each `state` message: seats (`innerHTML`), pot, my cards, controls. `event` messages are used only for sound effects (Web Audio synth in `Sfx`), the ticker text, and the chip-fly animation; don't put game logic there.
- Lobby has two panels: `#authForm` (login/register tabs) and `#joinForm` (profile: avatar seed picker, display name, room). `request(obj)` opens the socket on demand and sends `resume` first when a token is stored. Avatars are drawn from the account's numeric `avatar` seed, not the name.
- Seat placement: `slot = (seat - mySeat + 6) % 6`, mapped to CSS classes `.slot-0` … `.slot-5` (slot 0 = me at the bottom).
- Deal animations: `renderedCards[playerId]` remembers how many up cards were already drawn, so only new cards get `.deal-in`. It is reset when `hand_no` changes.
- `handName()` in JS mirrors the Rust evaluator, but only to label the player's own hand. The server's `visible_hand` labels opponents.
- WebSocket URL defaults to `ws(s)://<location.host>/ws`, or `?server=` when given, or `ws://localhost:3000/ws` when opened from `file://`.
- Layout is a fixed 9:16 `#app` container using `container-type: size`; size things with `cqw`/`cqh` units, not `px`/`vw`. Pixel art (suits, avatars) is generated as SVG from bitmap string arrays (`SUIT_PIX`, `HAIRS`/`FACE`).

Game constants (`MAX_PLAYERS`, `START_CHIPS`, `ANTE`, `MIN_BET`, `TURN_MS`, `NEXT_HAND_MS`) are at the top of `game.rs`. The README's rules table mirrors them.
