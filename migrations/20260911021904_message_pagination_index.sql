-- Add migration script here

-- keyset 分页按 (chat_id, id DESC) 定位，现有的 (chat_id, created_at DESC)
-- 服务不了 ORDER BY id。
CREATE INDEX IF NOT EXISTS msg_chat_id_id_index ON messages (chat_id, id DESC);

-- id 是 bigserial，其顺序与插入顺序（即 created_at 顺序）一致，
-- 上面这个索引已覆盖原索引的全部用途。保留两个只会让每次写入多维护一棵树。
DROP INDEX IF EXISTS chat_id_created_at_index;