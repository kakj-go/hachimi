# DingTalk official API contract snapshot

Retrieved: 2026-07-30T19:20:00+08:00

Official pages checked with HTTP 200:

- Obtain org application token: https://open.dingtalk.com/document/orgapp/obtain-orgapp-token
- Department list v2: https://open.dingtalk.com/document/orgapp/obtain-the-department-list-v2
- Users by department: https://open.dingtalk.com/document/orgapp/query-users-by-department
- Robot/message overview: https://open.dingtalk.com/document/orgapp/robot-overview
- Stream mode overview: https://open.dingtalk.com/document/orgapp/overview-of-stream-mode

Wire contract fixed for this implementation:

- `GET https://oapi.dingtalk.com/gettoken?appkey=...&appsecret=...`
- `POST https://oapi.dingtalk.com/topapi/v2/department/listsub?access_token=...`
- `POST https://oapi.dingtalk.com/topapi/v2/user/list?access_token=...`
- Outbound robot text uses the official OpenAPI robot message endpoint selected by peer/thread type and a bearer access token.
- Inbound events use DingTalk Stream mode. Event IDs, topic/type, tenant/client identity and server acknowledgement are preserved for durable deduplication and acknowledgement.

Only account/tenant identity, department/member reads, text message send, and message/event ingress are in scope. Approval, calendar, task and document APIs are excluded.
