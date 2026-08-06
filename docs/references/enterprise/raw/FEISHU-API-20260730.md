# Feishu official API contract snapshot

Retrieved: 2026-07-30T19:20:00+08:00

Official pages checked with HTTP 200:

- Tenant access token: https://open.feishu.cn/document/server-docs/authentication-management/access-token/tenant_access_token_internal
- Department children: https://open.feishu.cn/document/server-docs/contact-v3/department/children
- Users by department: https://open.feishu.cn/document/server-docs/contact-v3/user/find_by_department
- Create message: https://open.feishu.cn/document/server-docs/im-v1/message/create
- Long-connection event subscription: https://open.feishu.cn/document/server-docs/event-subscription-guide/start-a-local-application

Wire contract fixed for this implementation:

- `POST https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal` with app ID and app secret.
- `GET /open-apis/contact/v3/departments/{department_id}/children` with bounded page token/size.
- `GET /open-apis/contact/v3/users/find_by_department` with bounded page token/size.
- `POST /open-apis/im/v1/messages?receive_id_type=...` with `msg_type=text` and JSON-encoded content.
- Inbound events use the official long-connection subscription and retain event ID, tenant/app identity, event type and acknowledgement state.

Only account/tenant identity, department/member reads, text message send, and message/event ingress are in scope. Approval, calendar, task, cloud document and other APIs are excluded.
