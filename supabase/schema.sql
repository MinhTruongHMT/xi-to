-- Bảng tài khoản cho Xì Tố 5 lá.
-- Chạy một lần trong Supabase Dashboard → SQL Editor.
--
-- Chỉ server Rust truy cập các bảng này, bằng secret key (hoặc service_role key cũ),
-- vốn bỏ qua RLS. RLS bật mà không có policy nào => khoá công khai (anon/publishable)
-- không đọc/ghi được gì, kể cả mật khẩu đã băm.

create table if not exists public.accounts (
  id            uuid primary key default gen_random_uuid(),
  username      text not null unique check (username ~ '^[a-z0-9_]{3,16}$'),
  password_hash text not null,                 -- Argon2id (PHC string)
  name          text not null,                 -- tên hiển thị, tối đa 12 ký tự
  avatar        integer not null default 0 check (avatar >= 0),
  chips         bigint not null default 1000 check (chips >= 0),
  created_at    timestamptz not null default now()
);

create table if not exists public.sessions (
  token_hash text primary key,                 -- SHA-256 của token, không lưu token gốc
  account_id uuid not null references public.accounts (id) on delete cascade,
  created_at timestamptz not null default now()
);
create index if not exists sessions_account_id_idx on public.sessions (account_id);

alter table public.accounts enable row level security;
alter table public.sessions enable row level security;

grant select, insert, update, delete on public.accounts, public.sessions to service_role;
