# 🃏 Xì Tố 5 Lá — Online (Pixel Art 8-bit)

Game Xì Tố 5 lá (Five-Card Stud) nhiều người chơi, thời gian thực qua WebSocket.

```
xi-to/
├── client/                 # Front-end: HTML/CSS/JS thuần, không cần build
│   ├── index.html          # Màn hình vào bàn + bàn chơi + thumb zone
│   ├── style.css           # Giao diện dọc 9:16, pixel art, font Press Start 2P
│   └── script.js           # WebSocket, render bàn, âm thanh chiptune (Web Audio API)
└── server/                 # Back-end Rust (axum + tokio)
    ├── Cargo.toml
    └── src/
        ├── main.rs         # HTTP + WebSocket, phân phối lệnh, hẹn giờ
        ├── game.rs         # Trạng thái phòng, lượt đi, vòng cược, chia hũ
        └── cards.rs        # Bộ bài, xáo bài, đánh giá tay bài (+ unit test)
```

## Chạy thử trên máy cá nhân

### 1. Cài Rust
Tải từ https://rustup.rs.

> **Windows:** toolchain mặc định `x86_64-pc-windows-msvc` cần
> [Build Tools for Visual Studio](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
> (chọn mục *Desktop development with C++*).
> Nếu dùng toolchain `-gnu`, bạn cần MSYS2/MinGW-w64 có `dlltool` và `as` trong PATH.

### 2. Chạy server
```bash
cd server
cargo run --release
```
Server sẽ in ra:
```
🃏 Xì Tố server đang chạy:
   Giao diện : http://localhost:3000  (thư mục ../client)
   WebSocket : ws://localhost:3000/ws
```
Server tự phục vụ luôn thư mục `client/`, nên **không cần chạy web server riêng**.

Tuỳ chọn: `PORT=8080 cargo run`, hoặc `CLIENT_DIR=/duong/dan/client cargo run`.

### 3. Mở game
- Mở **2 tab trở lên** tại http://localhost:3000, nhập tên khác nhau và **cùng tên phòng**.
- Bấm **▶ BẮT ĐẦU VÁN** khi có từ 2 người. Sau mỗi ván, ván mới tự bắt đầu sau 7 giây.
- Trên máy tính, bật DevTools → *Toggle device toolbar* (iPhone) để xem đúng giao diện dọc.

**Chơi bằng điện thoại trong cùng mạng Wi-Fi:** tìm IP máy tính (`ipconfig` / `ifconfig`,
ví dụ `192.168.1.10`) rồi mở `http://192.168.1.10:3000` trên điện thoại.
Có thể phải cho phép cổng 3000 qua tường lửa.

**Mở client riêng (không qua server):** mở thẳng `client/index.html`. Client sẽ mặc định
kết nối `ws://localhost:3000/ws`, hoặc đổi trong mục **MÁY CHỦ** ở màn hình vào bàn
(cũng có thể truyền `?server=ws://host:port/ws` trên URL).

### Chạy unit test
```bash
cd server
cargo test
```

### Chạy bằng Docker (không cần cài Rust)
```bash
docker build -t xito .
docker run --rm -p 3000:3000 xito
```

## Deploy lên Render

1. Đưa thư mục `xi-to/` lên một repo GitHub (gồm `Dockerfile`, `render.yaml`, `client/`, `server/`).
2. Vào https://dashboard.render.com → **New** → **Blueprint** → chọn repo vừa tạo → **Apply**.
   Render đọc `render.yaml` và tự tạo Web Service chạy Docker.
   (Hoặc **New → Web Service** → chọn repo → Runtime **Docker**, giữ mặc định.)
3. Chờ build xong (lần đầu khoảng 3–5 phút), Render cấp link dạng `https://xi-to-xxxx.onrender.com`.
   Gửi link này cho bạn bè. WebSocket tự dùng `wss://` qua HTTPS.

Mỗi lần `git push` lên nhánh chính, Render tự build và deploy lại.

**Lưu ý gói Free:** server ngủ sau khoảng 15 phút không có truy cập. Người vào đầu tiên phải chờ
khoảng 1 phút để server thức dậy. Mỗi lần server ngủ hoặc deploy lại, mọi phòng và số chip bị xoá
(game lưu trạng thái trong RAM).

## Luật chơi đã cài đặt

| Mục | Giá trị |
|---|---|
| Số người / bàn | 2–6 |
| Chip khởi đầu | 1000 (hết chip sẽ được tự nạp lại khi vào ván mới) |
| Cược sàn (ante) | 10 |
| Tố tối thiểu | 20, hoặc bằng mức tố lớn nhất trước đó trong vòng |
| Thời gian mỗi lượt | 25 giây. Hết giờ thì tự **Xem** nếu không phải bỏ thêm tiền, ngược lại tự **Úp bỏ** |

1. Mỗi người đặt sàn, nhận **1 lá tẩy úp** và **1 lá ngửa**.
2. Người có **bài ngửa mạnh nhất** nói trước. Nếu bằng nhau thì so chất của lá cao nhất
   theo thứ tự Bích < Chuồn < Rô < Cơ.
3. Mỗi vòng cược xong chia thêm 1 lá ngửa, đến khi đủ 5 lá (4 vòng cược).
4. Lật bài: Thùng phá sảnh > Tứ quý > Cù lũ > Thùng > Sảnh > Sám cô > Thú > Đôi > Mậu thầu.
   Hoà thì chia đều hũ.
5. Có xử lý **tất tay (all-in)** và **hũ phụ (side pot)**. Nếu chỉ còn ≤ 1 người có thể cược,
   server lật nốt các lá còn lại rồi so bài luôn.

## Chống gian lận
- Server là nơi duy nhất giữ bộ bài và quyết định kết quả. Client chỉ gửi ý định (`fold`/`call`/`raise`).
- **Lá tẩy** chỉ được gửi riêng cho chủ nhân (tin `hole` và trường `hole` trong `state` riêng của
  người đó). Với người khác, trường này luôn là `null` cho tới lúc lật bài.
- Mọi lệnh đều được kiểm tra: đúng lượt, đủ tiền, mức tố tối thiểu.

## Giao thức WebSocket (JSON)

**Client → Server**
```json
{ "type": "join",  "name": "Tuấn", "room": "sanh-chinh" }
{ "type": "start" }
{ "type": "fold" }
{ "type": "call" }                 // theo, hoặc xem (check) khi to_call = 0
{ "type": "raise", "amount": 40 }  // số tố THÊM, ngoài số tiền phải theo
```

**Server → Client**
| `type` | Nội dung |
|---|---|
| `welcome` | `id` của bạn, tên phòng |
| `hole` | lá tẩy của riêng bạn: `{ "card": { "r": 14, "s": "H" } }` |
| `state` | toàn bộ trạng thái bàn **đã lọc theo người xem**: người chơi, hũ, lượt, `to_call`, `min_raise`, `max_raise`, thời gian còn lại, kết quả ván |
| `event` | sự kiện ngắn để phát âm thanh/hiệu ứng: `deal`, `call`, `raise`, `allin`, `check`, `fold`, `win`, `your_turn`, ... |
| `error` | thông báo lỗi (sai lượt, tố thiếu, ...) |

Lá bài: `r` = 2..14 (J=11, Q=12, K=13, A=14), `s` = `S` (Bích), `C` (Chuồn), `D` (Rô), `H` (Cơ).

## Âm thanh
Toàn bộ âm thanh được tổng hợp bằng **Web Audio API** (sóng vuông, sóng tam giác và nhiễu trắng),
không cần file âm thanh nào: tiếng chia bài, xáo bài, phỉnh cược, xem, úp bỏ, tất tay, đến lượt,
đồng hồ đếm ngược, nhạc thắng. Nút ♪ trên thanh trên cùng để bật/tắt.
