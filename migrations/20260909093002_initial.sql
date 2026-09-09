-- Add migration script here
-- 用户表。ws_id 的外键约束等 workspaces 建好后再补，见文件末尾。
CREATE TABLE IF NOT EXISTS users (
  id bigserial PRIMARY KEY,
  ws_id bigint NOT NULL,
  fullname varchar(64) NOT NULL,
  email varchar(64) NOT NULL,
  -- argon2 哈希后的密码，定长 97
  password_hash varchar(97) NOT NULL,
  created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 工作区，一个工作区下有多个用户和多个会话
CREATE TABLE IF NOT EXISTS workspaces (
  id bigserial PRIMARY KEY,
  name varchar(32) NOT NULL UNIQUE,
  owner_id bigint NOT NULL REFERENCES users (id),
  created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- users.ws_id 与 workspaces.owner_id 互相引用，先塞一对哨兵行把环解开。
-- id 显式给 0：bigserial 的序列从 1 开始，不会和它撞。
BEGIN;
INSERT INTO users (id, ws_id, fullname, email, password_hash)
  VALUES (0, 0, 'super user', 'super@none.org', '');
INSERT INTO workspaces (id, name, owner_id)
  VALUES (0, 'none', 0);
COMMIT;

-- 哨兵行就位后，才能给 users.ws_id 补上外键
ALTER TABLE users
  ADD CONSTRAINT users_ws_id_fk FOREIGN KEY (ws_id) REFERENCES workspaces (id);

CREATE UNIQUE INDEX IF NOT EXISTS email_index ON users (email);

CREATE TYPE chat_type AS ENUM (
  'single',
  'group',
  'private_channel',
  'public_channel'
);

CREATE TABLE IF NOT EXISTS chats (
  id bigserial PRIMARY KEY,
  ws_id bigint NOT NULL REFERENCES workspaces (id),
  name varchar(64),
  type chat_type NOT NULL,
  -- 成员的 user id 列表
  members bigint[] NOT NULL,
  created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE (ws_id, name, members)
);

CREATE TABLE IF NOT EXISTS messages (
  id bigserial PRIMARY KEY,
  chat_id bigint NOT NULL REFERENCES chats (id),
  sender_id bigint NOT NULL REFERENCES users (id),
  content text NOT NULL,
  files text[] NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- 拉某个会话的最新消息：按 chat_id 定位，再按时间倒序
CREATE INDEX IF NOT EXISTS chat_id_created_at_index ON messages (chat_id, created_at DESC);
CREATE INDEX IF NOT EXISTS sender_id_index ON messages (sender_id, created_at DESC);
-- 数组包含查询（"我参与了哪些会话"）走 GIN
CREATE INDEX IF NOT EXISTS chat_members_index ON chats USING GIN (members);