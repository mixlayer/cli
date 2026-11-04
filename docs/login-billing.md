# Login and billing

## `mxl login`

- In a terminal: tries the browser device flow; if the API doesn't support it yet (404/405/501 or network error), opens the API keys page in the console and asks you to paste a key (hidden input).
- `mxl login --paste` skips the device flow.
- Scripts and agents: `printf %s "$KEY" | mxl login --with-token`. Without a terminal (or in agent mode / `--no-input`) plain `mxl login` exits 2 with this hint and never prompts.
- The key is checked with `GET /models` (same as `mxl doctor`) and saved the same way as `mxl auth set-token`.
- `mxl logout` removes the saved key. `mxl whoami` shows the active identity.

## `mxl billing`

- `mxl billing balance` — prepaid credit balance.
- `mxl billing add-card` — opens a hosted card page; falls back to the console billing page.
- `mxl billing add-credit --amount 25 --yes` — charges the saved card via the existing refill endpoint (needs an api-admin key). `--yes` is always required; agent mode does not imply it.
- `mxl billing add-credit` without `--amount` opens the console; `--wait` polls the balance until it changes.

## Backend endpoints (seam)

The CLI already speaks these and falls back when they return 404/405/501:

| Endpoint | Auth | Response |
|---|---|---|
| `POST /v1/cli/device_authorizations` `{client_name}` | none | `{data:{device_code,user_code,verification_uri,verification_uri_complete,expires_in,interval}}` |
| `POST /v1/cli/device_authorizations/token` `{device_code}` | none | `{data:{status: pending\|slow_down\|approved\|denied\|expired, api_key?}}` |
| `POST /v1/cli/device_authorizations/approve` `{user_code,organization_id,approve}` | console user session | 204 |
| `POST /v1/billing_accounts/{id}/card_setup_session` | user or api-admin key | `{data:{url}}` |

Existing endpoints used today: `GET /v1/billing_accounts/{id}?include=all` (balance) and `POST /v1/billing_accounts/{id}/refill` (buy credit).
Verification URIs are only trusted on `*.mixlayer.com`. The API key is minted at poll time after approval, so the server never stores a plaintext secret.
