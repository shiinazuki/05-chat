#!/usr/bin/env fish
# chat 服务端到端演示：注册 → 建会话 → 互发消息 → 分页查询 → 权限与排序验证
# 用法：fish scripts/demo.fish [base_url]     默认 http://localhost:6688

# 必须是 global：fish 的函数看不到调用者的局部变量
set -g BASE (test -n "$argv[1]"; and echo $argv[1]; or echo http://localhost:6688)

function _tok -a body
    echo $body | python3 -c 'import sys,json; print(json.load(sys.stdin)["token"])'
end
function _field -a body key
    echo $body | python3 -c "import sys,json; print(json.load(sys.stdin)['$key'])"
end
function _post -a token path body
    curl -s -X POST $BASE$path -H "authorization: Bearer $token" \
        -H 'content-type: application/json' -d $body
end
function _signup -a name email pass
    curl -s -X POST $BASE/api/signup -H 'content-type: application/json' \
        -d "{\"fullname\":\"$name\",\"email\":\"$email\",\"password\":\"$pass\"}"
end
function _uid -a token
    curl -s -H "authorization: Bearer $token" $BASE/api/users/me \
        | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])'
end
function _pretty
    python3 -m json.tool --no-ensure-ascii
end

echo "════════ 1. 注册四个用户 ════════"
set -l A (_tok (_signup 爱丽丝 alice@chat.com Alice-pass-2026))
set -l B (_tok (_signup 鲍勃   bob@chat.com   Bob-pass-2026))
set -l C (_tok (_signup 卡罗尔 carol@chat.com Carol-pass-2026))
set -l D (_tok (_signup 戴夫   dave@chat.com  Dave-pass-2026))
set -l IDA (_uid $A); set -l IDB (_uid $B); set -l IDC (_uid $C); set -l IDD (_uid $D)
echo "  alice=$IDA  bob=$IDB  carol=$IDC  dave=$IDD"

echo ""
echo "════════ 2. alice 建三人群（不命名 → group）════════"
set -l CHAT (_post $A /api/chats "{\"name\":null,\"members\":[$IDA,$IDB,$IDC],\"public\":false}")
echo $CHAT | _pretty
set -l CID (_field $CHAT id)

echo ""
echo "════════ 3. 再建命名公开频道（→ publicChannel）════════"
_post $A /api/chats "{\"name\":\"公告板\",\"members\":[$IDA,$IDB],\"public\":true}" | _pretty

echo ""
echo "════════ 4. 三人互发消息 ════════"
function _say -a token cid text
    _post $token /api/chats/$cid/messages "{\"content\":\"$text\"}" \
        | python3 -c 'import sys,json; m=json.load(sys.stdin); print("  #%d sender=%d  %s" % (m["id"], m["sender_id"], m["content"]))'
end
_say $A $CID 大家好我是爱丽丝
_say $B $CID 嗨爱丽丝我是鲍勃
_say $C $CID 卡罗尔来了
_say $A $CID 今晚开会吗
_say $B $CID 开八点
_say $C $CID 收到

echo ""
echo "════════ 5. 拉最新 3 条 ════════"
curl -s -H "authorization: Bearer $A" "$BASE/api/chats/$CID/messages?limit=3" | _pretty

echo ""
echo "════════ 6. 游标分页：用上一页最后一条的 id 当 before ════════"
set -l CURSOR (curl -s -H "authorization: Bearer $A" "$BASE/api/chats/$CID/messages?limit=3" \
    | python3 -c 'import sys,json; print(json.load(sys.stdin)[-1]["id"])')
echo "  游标 = $CURSOR"
curl -s -H "authorization: Bearer $B" "$BASE/api/chats/$CID/messages?limit=3&before=$CURSOR" \
    | python3 -c 'import sys,json
for m in json.load(sys.stdin): print("  #%d  %s" % (m["id"], m["content"]))'

echo ""
echo "════════ 7. 权限：dave 不是成员，三条都应 404 ════════"
curl -s -o /dev/null -w "  非成员读消息 = %{http_code}\n" -H "authorization: Bearer $D" "$BASE/api/chats/$CID/messages"
curl -s -o /dev/null -w "  非成员发消息 = %{http_code}\n" -X POST -H "authorization: Bearer $D" \
    -H 'content-type: application/json' -d '{"content":"我能进来吗"}' "$BASE/api/chats/$CID/messages"
curl -s -o /dev/null -w "  不存在的会话 = %{http_code}\n" -H "authorization: Bearer $D" "$BASE/api/chats/999999/messages"

echo ""
echo "════════ 8. 排序（NULLS LAST：未命名群垫底）════════"
curl -s -H "authorization: Bearer $A" "$BASE/api/chats?sort=name&order=asc" \
    | python3 -c 'import sys,json
for c in json.load(sys.stdin): print("  id=%-5d name=%-10s type=%s" % (c["id"], c["name"], c["type"]))'

echo ""
echo "════════ 9. 注入尝试应被 400 拒掉 ════════"
curl -s -o /dev/null -w "  ?sort=id;DROP TABLE users-- = %{http_code}\n" \
    -H "authorization: Bearer $A" "$BASE/api/chats?sort=id%3BDROP%20TABLE%20users--"

echo ""
echo "════════ 10. limit 上限截断 ════════"
curl -s -H "authorization: Bearer $A" "$BASE/api/chats/$CID/messages?limit=100000" \
    | python3 -c 'import sys,json; print("  传 limit=100000，实际返回", len(json.load(sys.stdin)), "条")'
