/* =========================================================
   XÌ TỐ 5 LÁ — client
   - Kết nối WebSocket tới server Rust
   - Vẽ bàn chơi pixel, lá bài, avatar bằng SVG sinh từ bitmap
   - Âm thanh chiptune 8-bit bằng Web Audio API (không dùng file ngoài)
   ========================================================= */
'use strict';

const $ = (s) => document.querySelector(s);
const $$ = (s) => document.querySelectorAll(s);

const store = {
  get(k, d) { try { return localStorage.getItem(k) ?? d; } catch { return d; } },
  set(k, v) { try { localStorage.setItem(k, v); } catch { /* chế độ riêng tư */ } },
};

/* =========================================================
   1. ÂM THANH 8-BIT (Web Audio API)
   ========================================================= */
const Sfx = (() => {
  let ctx = null;
  let master = null;
  let enabled = store.get('xito-sound', '1') === '1';

  function init() {
    if (!ctx) {
      const AC = window.AudioContext || window.webkitAudioContext;
      if (!AC) return false;
      ctx = new AC();
      master = ctx.createGain();
      master.gain.value = 0.22;
      master.connect(ctx.destination);
    }
    if (ctx.state === 'suspended') ctx.resume();
    return true;
  }

  // Một nốt sóng vuông/tam giác với đường bao tắt dần kiểu máy NES.
  function tone(freq, dur, { type = 'square', vol = 0.5, slide = 0, delay = 0 } = {}) {
    const t = ctx.currentTime + delay;
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = type;
    osc.frequency.setValueAtTime(freq, t);
    if (slide) osc.frequency.exponentialRampToValueAtTime(Math.max(30, freq + slide), t + dur);
    gain.gain.setValueAtTime(vol, t);
    gain.gain.exponentialRampToValueAtTime(0.001, t + dur);
    osc.connect(gain).connect(master);
    osc.start(t);
    osc.stop(t + dur + 0.02);
  }

  // Tiếng nhiễu trắng đã lọc — dùng cho tiếng lá bài lướt trên nỉ.
  function noise(dur, { vol = 0.4, delay = 0, hp = 1500 } = {}) {
    const t = ctx.currentTime + delay;
    const len = Math.floor(ctx.sampleRate * dur);
    const buf = ctx.createBuffer(1, len, ctx.sampleRate);
    const data = buf.getChannelData(0);
    // nhiễu "bước" giống chip âm thanh 8-bit: giữ mỗi mẫu vài lần
    for (let i = 0; i < len; i += 4) {
      const v = Math.random() * 2 - 1;
      for (let j = 0; j < 4 && i + j < len; j++) data[i + j] = v;
    }
    const src = ctx.createBufferSource();
    src.buffer = buf;
    const filter = ctx.createBiquadFilter();
    filter.type = 'highpass';
    filter.frequency.value = hp;
    const gain = ctx.createGain();
    gain.gain.setValueAtTime(vol, t);
    gain.gain.exponentialRampToValueAtTime(0.001, t + dur);
    src.connect(filter).connect(gain).connect(master);
    src.start(t);
  }

  const notes = (seq, step, opts) => seq.forEach((f, i) => f && tone(f, step * 0.95, { ...opts, delay: i * step }));

  const sounds = {
    click() { tone(988, 0.04, { vol: 0.3 }); tone(1319, 0.05, { vol: 0.25, delay: 0.035 }); },
    deal(n = 1) {
      for (let i = 0; i < n; i++) {
        noise(0.07, { hp: 2500, vol: 0.5, delay: i * 0.09 });
        tone(330, 0.05, { type: 'triangle', slide: 400, vol: 0.4, delay: i * 0.09 });
      }
    },
    shuffle() { for (let i = 0; i < 8; i++) noise(0.035, { hp: 3000, vol: 0.35, delay: i * 0.045 }); },
    chip() {
      tone(1568, 0.05, { vol: 0.35 });
      tone(2093, 0.08, { vol: 0.3, delay: 0.05 });
      noise(0.03, { hp: 6000, vol: 0.25, delay: 0.02 });
    },
    allin() { notes([392, 523, 659, 784, 1047], 0.07, { vol: 0.35 }); this.chip(); },
    check() { tone(660, 0.05, { type: 'triangle', vol: 0.5 }); tone(660, 0.05, { type: 'triangle', vol: 0.5, delay: 0.09 }); },
    fold() { tone(440, 0.28, { type: 'sawtooth', slide: -320, vol: 0.25 }); },
    turn() { notes([523, 659, 784], 0.08, { vol: 0.35 }); },
    tick() { tone(1200, 0.03, { vol: 0.2 }); },
    win() {
      notes([523, 659, 784, 1047, 0, 784, 1047], 0.1, { vol: 0.35 });
      notes([131, 0, 196, 0, 262], 0.14, { type: 'triangle', vol: 0.5 });
    },
    join() { notes([784, 988], 0.06, { vol: 0.25 }); },
    leave() { notes([494, 392], 0.08, { vol: 0.25 }); },
    error() { tone(140, 0.18, { vol: 0.3 }); },
  };

  return {
    unlock: init,
    get enabled() { return enabled; },
    toggle() { enabled = !enabled; store.set('xito-sound', enabled ? '1' : '0'); return enabled; },
    play(name, ...args) {
      if (!enabled) return;
      try { if (init()) sounds[name]?.(...args); } catch { /* bỏ qua nếu trình duyệt chặn */ }
    },
  };
})();

/* =========================================================
   2. ĐỒ HOẠ PIXEL (SVG từ bitmap)
   ========================================================= */
const RANKS = { 11: 'J', 12: 'Q', 13: 'K', 14: 'A' };
const rankLabel = (r) => RANKS[r] || String(r);

const SUIT_PIX = {
  H: ['.XX.XX.', 'XXXXXXX', 'XXXXXXX', '.XXXXX.', '..XXX..', '...X...'],
  D: ['...X...', '..XXX..', '.XXXXX.', 'XXXXXXX', '.XXXXX.', '..XXX..', '...X...'],
  S: ['...X...', '..XXX..', '.XXXXX.', 'XXXXXXX', 'XXXXXXX', '.XX.XX.', '...X...', '..XXX..'],
  C: ['..XXX..', '..XXX..', 'XXXXXXX', 'XXXXXXX', 'XX.X.XX', '...X...', '..XXX..'],
};

// Ghép các ô liên tiếp trên mỗi hàng thành 1 <rect> cho gọn.
function bitmapRects(rows, pick) {
  let out = '';
  rows.forEach((row, y) => {
    let x = 0;
    while (x < row.length) {
      const fill = pick(row[x]);
      if (!fill) { x++; continue; }
      let w = 1;
      while (row[x + w] === row[x]) w++;
      out += `<rect x="${x}" y="${y}" width="${w}" height="1" fill="${fill}"/>`;
      x += w;
    }
  });
  return out;
}

const suitCache = {};
function suitSvg(s) {
  if (!suitCache[s]) {
    const rows = SUIT_PIX[s];
    suitCache[s] = `<svg viewBox="0 0 7 ${rows.length}" shape-rendering="crispEdges">${bitmapRects(rows, (c) => c === 'X' && 'currentColor')}</svg>`;
  }
  return suitCache[s];
}

function cardHTML(c, cls = '', style = '') {
  if (!c) return `<div class="card back ${cls}" style="${style}"></div>`;
  const red = c.s === 'H' || c.s === 'D';
  return `<div class="card ${red ? 'red' : ''} ${cls}" style="${style}">
    <span class="rank">${rankLabel(c.r)}</span>
    <span class="suit-small">${suitSvg(c.s)}</span>
    <span class="suit-big">${suitSvg(c.s)}</span>
  </div>`;
}

// Avatar nhân vật 8x8 sinh ngẫu nhiên nhưng cố định theo tên.
const HAIRS = [
  ['..HHHH..', '.HHHHHH.'],
  ['.H.HH.H.', '.HHHHHH.'],
  ['........', '.HHHHHH.'],
  ['HHHHHHHH', 'HHHHHHHH'],
];
const FACE = ['.HSSSSH.', '.SESSES.', '.SSSSSS.', '..SMMS..', '.BBBBBB.', 'BBBBBBBB'];
const PALETTE = {
  hair: ['#3b2412', '#f2c14e', '#d94f2b', '#222', '#9b59d0', '#e8e8e8'],
  skin: ['#f5c99b', '#d9a066', '#8d5524', '#ffdbac'],
  body: ['#e83b3b', '#3b7be8', '#2fb35a', '#ffcc33', '#b04ad9', '#ff7f27'],
};
const hash = (s) => [...s].reduce((h, ch) => (h * 31 + ch.codePointAt(0)) >>> 0, 7);

const avatarCache = {};
function avatarSvg(name) {
  if (avatarCache[name]) return avatarCache[name];
  const h = hash(name);
  const rows = [...HAIRS[h % HAIRS.length], ...FACE];
  const col = {
    H: PALETTE.hair[(h >> 3) % PALETTE.hair.length],
    S: PALETTE.skin[(h >> 6) % PALETTE.skin.length],
    B: PALETTE.body[(h >> 9) % PALETTE.body.length],
    E: '#000',
    M: '#a3312f',
  };
  const bg = `hsl(${h % 360} 45% 22%)`;
  return (avatarCache[name] = `<svg viewBox="0 0 8 8" shape-rendering="crispEdges"><rect width="8" height="8" fill="${bg}"/>${bitmapRects(rows, (c) => col[c])}</svg>`);
}

/* Đánh giá tay bài phía client — chỉ để hiển thị tên bài của chính mình. */
function handName(cards) {
  if (!cards.length) return '';
  const cnt = {};
  cards.forEach((c) => { cnt[c.r] = (cnt[c.r] || 0) + 1; });
  const g = Object.entries(cnt).map(([r, n]) => [n, +r]).sort((a, b) => b[0] - a[0] || b[1] - a[1]);
  const R = (i) => rankLabel(g[i][1]);
  if (cards.length === 5) {
    const flush = cards.every((c) => c.s === cards[0].s);
    const has = (r) => cnt[r === 1 ? 14 : r];
    let high = 0;
    for (let h = 14; h >= 5 && !high; h--) {
      let ok = true;
      for (let r = h - 4; r <= h; r++) if (!has(r)) { ok = false; break; }
      if (ok) high = h;
    }
    if (flush && high) return `Thùng phá sảnh ${rankLabel(high)}`;
    if (g[0][0] === 4) return `Tứ quý ${R(0)}`;
    if (g[0][0] === 3 && g[1][0] === 2) return `Cù lũ ${R(0)}`;
    if (flush) return `Thùng ${R(0)}`;
    if (high) return `Sảnh ${rankLabel(high)}`;
  }
  if (g[0][0] === 4) return `Tứ quý ${R(0)}`;
  if (g[0][0] === 3) return `Sám cô ${R(0)}`;
  if (g[0][0] === 2 && g[1]?.[0] === 2) return `Thú ${R(0)}-${R(1)}`;
  if (g[0][0] === 2) return `Đôi ${R(0)}`;
  return `Mậu thầu ${R(0)}`;
}

/* =========================================================
   3. KẾT NỐI MẠNG
   ========================================================= */
let ws = null;
let myId = null;
let state = null;
let leaving = false;
let joinInfo = null;

function defaultServer() {
  const param = new URLSearchParams(location.search).get('server');
  if (param) return param;
  if (location.protocol.startsWith('http')) {
    return `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws`;
  }
  return 'ws://localhost:3000/ws';
}

function connect(info) {
  joinInfo = info;
  leaving = false;
  $('#lobbyMsg').textContent = 'ĐANG KẾT NỐI...';
  try {
    ws = new WebSocket(info.server);
  } catch {
    $('#lobbyMsg').textContent = 'Địa chỉ máy chủ không hợp lệ';
    return;
  }
  ws.onopen = () => send({ type: 'join', name: info.name, room: info.room });
  ws.onmessage = (e) => {
    let msg;
    try { msg = JSON.parse(e.data); } catch { return; }
    onMessage(msg);
  };
  ws.onclose = () => {
    if (leaving) return;
    if ($('#game').classList.contains('active')) {
      $('#discModal').classList.remove('hidden');
    } else {
      $('#lobbyMsg').textContent = 'Không kết nối được máy chủ!';
    }
  };
}

function send(obj) {
  if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(obj));
}

function onMessage(msg) {
  switch (msg.type) {
    case 'welcome':
      myId = msg.id;
      resetHandView();
      $('#roomName').textContent = msg.room;
      showScreen('game');
      Sfx.play('join');
      break;
    case 'state':
      onState(msg);
      break;
    case 'hole':
      // lá tẩy được gửi riêng; state kế tiếp cũng mang lá này (chỉ cho mình mình)
      holePeek = false;
      break;
    case 'event':
      onEvent(msg);
      break;
    case 'error':
      toast(msg.msg);
      Sfx.play('error');
      if (!$('#game').classList.contains('active')) $('#lobbyMsg').textContent = msg.msg;
      break;
  }
}

/* =========================================================
   4. HIỂN THỊ BÀN CHƠI
   ========================================================= */
let holePeek = false;
let renderedCards = {};   // id người chơi -> số lá ngửa đã vẽ (để chỉ animate lá mới)
let renderedHand = -1;
let shownResult = -1;
let raiseAmt = 0;
let turnDeadline = 0;
let turnTotal = 1;
let lastTickSec = -1;

function resetHandView() {
  renderedCards = {};
  holePeek = false;
}

const me = () => state?.players.find((p) => p.id === myId);
const slotOf = (p) => (p.seat - (me()?.seat ?? 0) + 6) % 6;

function onState(s) {
  const prev = state;
  state = s;
  if (s.hand_no !== renderedHand) {
    resetHandView();
    renderedHand = s.hand_no;
    if (s.phase === 'betting') $('#resultModal').classList.add('hidden');
  }

  $('#handNo').textContent = s.hand_no;
  renderSeats();
  renderPot(prev);
  renderMyCards();
  renderControls();

  if (s.turn_ms_left != null) {
    turnDeadline = performance.now() + s.turn_ms_left;
    turnTotal = s.turn_ms_total;
  } else {
    turnDeadline = 0;
  }

  if (s.result && s.result.hand_no !== shownResult) {
    shownResult = s.result.hand_no;
    setTimeout(() => showResult(s.result), 900);
  }

  // ghi nhớ số lá đã vẽ sau khi render xong
  s.players.forEach((p) => { renderedCards[p.id] = p.up.length; });
}

function renderSeats() {
  const html = state.players.map((p) => {
    const slot = slotOf(p);
    const isMe = p.id === myId;
    const classes = ['seat', `slot-${slot}`];
    if (state.turn === p.id) classes.push('turn');
    if (p.folded) classes.push('folded');
    if (!p.connected) classes.push('offline');

    const already = renderedCards[p.id] ?? 0;
    let cards = '';
    if (!isMe && p.in_hand) {
      cards = `<div class="mini-cards">
        ${p.has_hole ? cardHTML(p.hole, already === 0 ? 'deal-in' : '') : ''}
        ${p.up.map((c, i) => cardHTML(c, i >= already ? 'deal-in' : '', `animation-delay:${(i - already) * 0.12 + 0.1}s`)).join('')}
      </div>`;
    }
    const bet = p.street_bet > 0
      ? `<div class="seat-bet"><i class="chip"></i>${p.street_bet}</div>`
      : `<div class="seat-act">${p.last_action ?? ''}</div>`;
    const tag = p.all_in ? '<span class="allin-tag">ALL-IN</span>' : '';
    const vis = !isMe && p.in_hand && !p.folded && p.visible_hand ? `<div class="vis-hand">${esc(p.visible_hand)}</div>` : '';

    return `<div class="${classes.join(' ')}" data-id="${p.id}">
      ${p.is_dealer ? '<div class="dealer">D</div>' : ''}
      <div class="seat-head">
        <div class="avatar">${avatarSvg(p.name)}</div>
        <div class="seat-info">
          <div class="seat-name">${esc(p.name)}${isMe ? ' ★' : ''}</div>
          <div class="seat-chips">◆${p.chips}</div>
        </div>
      </div>
      ${cards}${vis}
      ${isMe ? `<div class="seat-info">${tag}${bet}</div>` : `${tag}${bet}`}
    </div>`;
  }).join('');
  $('#seats').innerHTML = html;
}

function renderPot(prev) {
  const pot = state.pot;
  $('#potAmt').textContent = pot;
  const stacks = Math.min(5, Math.ceil(pot / 100));
  $('#potChips').innerHTML = Array.from({ length: stacks }, (_, i) => `<i class="chip c${(i % 3) + 1}"></i>`).join('');
  if (prev && prev.pot !== pot && pot > 0) {
    const box = $('#potBox');
    box.classList.remove('bump');
    void box.offsetWidth;
    box.classList.add('bump');
  }

  let info = '';
  if (state.phase === 'waiting') {
    const n = state.players.length;
    info = n < 2 ? 'CHỜ THÊM NGƯỜI CHƠI...' : `${n} NGƯỜI SẴN SÀNG`;
  } else if (state.phase === 'betting') {
    const t = state.players.find((p) => p.id === state.turn);
    info = `LÁ ${state.street}/5` + (t ? `<br>LƯỢT: ${esc(t.name)}` : '');
    if (state.current_bet > 0) info += `<br>CƯỢC: ${state.current_bet}`;
  } else if (state.result) {
    info = esc(state.result.text);
  }
  $('#centerInfo').innerHTML = info;
}

function renderMyCards() {
  const p = me();
  const box = $('#myCards');
  $('#myChips').textContent = p ? p.chips : 0;

  if (!p || !p.in_hand) {
    box.innerHTML = Array.from({ length: 5 }, () => cardHTML(null, 'empty')).join('');
    $('#myHandName').textContent = p && state.phase === 'betting' ? 'ĐANG XEM VÁN...' : '';
    return;
  }
  if (state.phase === 'showdown' && !p.folded) holePeek = true;

  const already = renderedCards[p.id] ?? 0;
  const hole = p.hole
    ? `<div class="hole-wrap ${holePeek ? 'peek' : ''} ${p.folded ? 'folded' : ''} ${already === 0 ? 'deal-in' : ''}" id="holeCard">
        <div class="hole-inner">
          ${cardHTML(null)}
          ${cardHTML(p.hole, 'face')}
        </div>
        <span class="hole-tag">${holePeek ? 'LÁ TẨY' : 'CHẠM XEM'}</span>
      </div>`
    : cardHTML(null, 'empty');
  const ups = p.up.map((c, i) =>
    cardHTML(c, `${i >= already ? 'deal-in' : ''}`, `animation-delay:${(i - already) * 0.12 + 0.1}s;${p.folded ? 'opacity:.45' : ''}`));
  const empties = Array.from({ length: 4 - p.up.length }, () => cardHTML(null, 'empty'));
  box.innerHTML = hole + ups.join('') + empties.join('');

  const cards = p.hole ? [p.hole, ...p.up] : p.up;
  $('#myHandName').textContent = p.folded ? 'ĐÃ ÚP BỎ' : `BÀI: ${handName(cards).toUpperCase()}`;
}

function renderControls() {
  const p = me();
  const inHand = p && p.in_hand && !p.folded && state.phase === 'betting';
  $('#betPanel').classList.toggle('show', !!inHand);
  $('#waitPanel').classList.toggle('show', !inHand);

  if (!inHand) {
    const startBtn = $('#startBtn');
    startBtn.style.display = state.can_start ? '' : 'none';
    let msg = '';
    if (state.phase === 'showdown') msg = 'VÁN MỚI SẼ TỰ BẮT ĐẦU...';
    else if (state.phase === 'betting') msg = p?.folded ? 'BẠN ĐÃ ÚP BỎ — CHỜ VÁN SAU' : 'ĐANG CÓ VÁN — BẠN SẼ VÀO VÁN SAU';
    else if (!state.can_start) msg = 'CẦN ÍT NHẤT 2 NGƯỜI CHƠI.<br>MỜI BẠN BÈ VÀO CÙNG PHÒNG!';
    $('#waitMsg').innerHTML = msg;
    return;
  }

  const myTurn = state.turn === myId;
  const toCall = state.to_call;
  const maxR = state.max_raise;
  const minR = Math.min(state.min_raise, maxR);

  $('#foldBtn').disabled = !myTurn;
  const callBtn = $('#callBtn');
  callBtn.disabled = !myTurn;
  if (toCall === 0) callBtn.innerHTML = 'XEM';
  else if (toCall >= p.chips) callBtn.innerHTML = `TẤT TAY<br>${p.chips}`;
  else callBtn.innerHTML = `THEO<br>${toCall}`;

  const canRaise = myTurn && maxR > 0;
  const slider = $('#raiseSlider');
  slider.min = minR;
  slider.max = Math.max(minR, maxR);
  raiseAmt = clamp(raiseAmt || minR, minR, maxR);
  slider.value = raiseAmt;
  slider.disabled = !canRaise || minR >= maxR;
  $$('#betPanel .btn-chip, #minusBtn, #plusBtn').forEach((b) => { b.disabled = !canRaise || minR >= maxR; });
  $('#raiseBtn').disabled = !canRaise;
  updateRaiseLabel();
}

function updateRaiseLabel() {
  if (!state) return;
  const allIn = raiseAmt >= state.max_raise;
  $('#raiseBtn').innerHTML = allIn ? `TẤT TAY<br>${state.max_raise + state.to_call}` : `TỐ<br>+${raiseAmt}`;
}

const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

function setRaise(v) {
  if (!state) return;
  const minR = Math.min(state.min_raise, state.max_raise);
  raiseAmt = clamp(Math.round(v / 10) * 10, minR, state.max_raise);
  if (state.max_raise - raiseAmt < 10) raiseAmt = state.max_raise;
  $('#raiseSlider').value = raiseAmt;
  updateRaiseLabel();
}

function showResult(result) {
  $('#resultText').textContent = result.text;
  $('#resultRows').innerHTML = result.rows.map((r) => `
    <div class="res-row ${r.won > 0 ? 'win' : ''}">
      <div class="res-top"><span>${esc(r.name)}${r.id === myId ? ' ★' : ''}</span><span class="won">${r.won > 0 ? '+' + r.won : ''}</span></div>
      <div class="res-hand">${esc(r.hand)}</div>
      <div class="mini-cards">${r.cards.map((c) => cardHTML(c)).join('')}</div>
    </div>`).join('');
  $('#resultModal').classList.remove('hidden');
}

/* ---------- Sự kiện từ server: âm thanh, chip bay, dòng chữ ---------- */
function onEvent(ev) {
  switch (ev.kind) {
    case 'your_turn':
      Sfx.play('turn');
      navigator.vibrate?.(40);
      return;
    case 'hand_start': Sfx.play('shuffle'); break;
    case 'deal': setTimeout(() => Sfx.play('deal', 3), 150); break;
    case 'call':
    case 'raise': Sfx.play('chip'); flyChip(ev.player); break;
    case 'allin': Sfx.play('allin'); flyChip(ev.player); break;
    case 'check': Sfx.play('check'); break;
    case 'fold': Sfx.play('fold'); break;
    case 'win': setTimeout(() => Sfx.play('win'), 500); break;
    case 'join': Sfx.play('join'); break;
    case 'leave': Sfx.play('leave'); break;
  }
  ticker(ev.text);
}

function flyChip(playerId) {
  const area = $('#tableArea');
  const seat = $(`.seat[data-id="${playerId}"] .avatar`);
  if (!seat) return;
  const a = area.getBoundingClientRect();
  const s = seat.getBoundingClientRect();
  const t = $('#potBox').getBoundingClientRect();
  const chip = document.createElement('i');
  chip.className = 'chip c3 flying-chip';
  chip.style.left = `${s.left - a.left + s.width / 2}px`;
  chip.style.top = `${s.top - a.top + s.height / 2}px`;
  area.appendChild(chip);
  requestAnimationFrame(() => requestAnimationFrame(() => {
    chip.style.transform = `translate(${t.left + t.width / 2 - s.left - s.width / 2}px, ${t.top + t.height / 2 - s.top - s.height / 2}px)`;
    chip.style.opacity = '0.2';
  }));
  setTimeout(() => chip.remove(), 500);
}

function ticker(text) {
  if (!text) return;
  const box = $('#ticker');
  const line = document.createElement('div');
  line.textContent = text;
  box.appendChild(line);
  while (box.children.length > 3) box.firstChild.remove();
  setTimeout(() => line.remove(), 3300);
}

let toastTimer = 0;
function toast(text) {
  const t = $('#toast');
  t.textContent = text;
  t.classList.remove('show');
  void t.offsetWidth;
  t.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove('show'), 2200);
}

function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}

function showScreen(id) {
  $$('.screen').forEach((el) => el.classList.toggle('active', el.id === id));
}

/* ---------- Thanh thời gian lượt đi ---------- */
function timerLoop() {
  const bar = $('#timerBar');
  if (turnDeadline) {
    const left = Math.max(0, turnDeadline - performance.now());
    const ratio = left / turnTotal;
    bar.style.width = `${(Math.floor(ratio * 40) / 40) * 100}%`; // nhảy theo bước kiểu 8-bit
    bar.classList.toggle('urgent', left < 6000);
    const sec = Math.ceil(left / 1000);
    if (state?.turn === myId && left > 0 && left < 5000 && sec !== lastTickSec) {
      lastTickSec = sec;
      Sfx.play('tick');
    }
  } else {
    bar.style.width = '0';
  }
  requestAnimationFrame(timerLoop);
}

/* =========================================================
   5. ĐIỀU KHIỂN
   ========================================================= */
function bindUI() {
  // Trình duyệt chỉ cho phát âm thanh sau lần chạm đầu tiên.
  document.addEventListener('pointerdown', () => Sfx.unlock(), { once: true });

  $('#nameInput').value = store.get('xito-name', '');
  $('#roomInput').value = store.get('xito-room', '');
  $('#serverInput').value = defaultServer();

  $('#joinForm').addEventListener('submit', (e) => {
    e.preventDefault();
    Sfx.play('click');
    const name = $('#nameInput').value.trim() || 'Tay chơi';
    const room = $('#roomInput').value.trim() || 'sanh-chinh';
    store.set('xito-name', name);
    store.set('xito-room', room);
    connect({ name, room, server: $('#serverInput').value.trim() || defaultServer() });
  });

  $('#foldBtn').addEventListener('click', () => { Sfx.play('click'); send({ type: 'fold' }); });
  $('#callBtn').addEventListener('click', () => { Sfx.play('click'); send({ type: 'call' }); });
  $('#raiseBtn').addEventListener('click', () => {
    Sfx.play('click');
    send({ type: 'raise', amount: raiseAmt });
  });
  $('#startBtn').addEventListener('click', () => { Sfx.play('click'); send({ type: 'start' }); });

  $('#raiseSlider').addEventListener('input', (e) => {
    setRaise(+e.target.value);
    Sfx.play('tick');
  });
  $('#minusBtn').addEventListener('click', () => { Sfx.play('click'); setRaise(raiseAmt - Math.max(10, state.min_raise / 2)); });
  $('#plusBtn').addEventListener('click', () => { Sfx.play('click'); setRaise(raiseAmt + Math.max(10, state.min_raise / 2)); });
  $$('[data-quick]').forEach((b) => b.addEventListener('click', () => {
    Sfx.play('click');
    const pot = state.pot;
    const v = { min: 0, half: pot / 2, pot, all: state.max_raise }[b.dataset.quick];
    setRaise(v);
  }));

  // Chạm vào lá tẩy để lật lên xem / úp lại (chống người ngồi cạnh nhìn trộm).
  $('#myCards').addEventListener('click', (e) => {
    const hole = e.target.closest('#holeCard');
    if (!hole || state?.phase === 'showdown') return;
    holePeek = !holePeek;
    hole.classList.toggle('peek', holePeek);
    hole.querySelector('.hole-tag').textContent = holePeek ? 'LÁ TẨY' : 'CHẠM XEM';
    Sfx.play('deal');
  });

  const soundBtn = $('#soundBtn');
  soundBtn.classList.toggle('off', !Sfx.enabled);
  soundBtn.addEventListener('click', () => {
    soundBtn.classList.toggle('off', !Sfx.toggle());
    Sfx.play('click');
  });

  $('#leaveBtn').addEventListener('click', () => {
    Sfx.play('leave');
    leaving = true;
    ws?.close();
    state = null;
    $('#lobbyMsg').textContent = '';
    showScreen('lobby');
  });

  $('#resultClose').addEventListener('click', () => { Sfx.play('click'); $('#resultModal').classList.add('hidden'); });
  $('#reconnectBtn').addEventListener('click', () => {
    Sfx.play('click');
    $('#discModal').classList.add('hidden');
    showScreen('lobby');
    if (joinInfo) connect(joinInfo);
  });

  // Mấy lá bài trang trí ở màn hình chờ
  $('#logoCards').innerHTML = [
    { r: 14, s: 'S' }, { r: 13, s: 'H' }, null, { r: 12, s: 'D' }, { r: 11, s: 'C' },
  ].map((c) => cardHTML(c)).join('');
}

bindUI();
requestAnimationFrame(timerLoop);
