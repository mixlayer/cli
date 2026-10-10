# Platform MCP server

Mixlayer's Platform API has a hosted [Model Context Protocol](https://modelcontextprotocol.io) server. Connect it to your coding agent and the agent can manage your Mixlayer organization for you: list and create API keys, check usage and spend, invite members, look at invoices and credits. You ask in plain language and skip the console.

This is separate from `mixlayer-mcp`, the local MCP server that ships with the CLI. `mixlayer-mcp` runs models (ask, compare, shield). The Platform MCP server manages the account those models bill to.

| | Local `mixlayer-mcp` | Hosted Platform MCP |
|---|---|---|
| Runs | On your machine, over stdio | At `https://api.mixlayer.com/v1/mcp`, over streamable HTTP |
| Does | Model calls, shield, ledger | Organizations, members, API keys, analytics, billing |
| Key needs | `inference` | `api-read`, plus `api-write` or `api-admin` to change things |

## Connect

One command registers it with your agent:

```sh
mixlayer agent setup --agent claude-code --platform           # preview
mixlayer agent setup --agent claude-code --platform --yes     # install
```

`--platform` works with `claude-code`, `codex` and `cursor`. With `--agent all`, setup registers it for those three and reports Hermes and Claude Desktop as unsupported. Setup also installs the skill and the local server, the same as `agent setup` without `--platform`.

Setup never writes your key into the agent's config. It stores the name of an environment variable, and the agent reads the key from it when it connects. That variable is `MIXLAYER_API_KEY` unless you pass `--key-env`:

```sh
export MIXLAYER_PLATFORM_KEY="..."   # in the shell profile your agent starts from
mixlayer agent setup --agent codex --platform --key-env MIXLAYER_PLATFORM_KEY --yes
```

If that variable is set in your shell, setup first checks the key with one read-only request (it lists your organizations) and reports the result:

| `platform-key` status | Meaning |
|---|---|
| `ok` | The key can read the Platform API. |
| `missing` | The variable isn't set in this shell, so nothing was checked. Set it where your agent starts. |
| `no_platform_permission` | HTTP 403: the key can't read the Platform API. Give it `api-read` in the console. |
| `rejected` | HTTP 401: the API didn't accept the key. Check that it's the whole key and still active. |

To remove it, run `mixlayer agent setup --agent all --uninstall --yes`. That removes only what setup added.

### Configure it by hand

If you'd rather edit config files yourself, these are the entries setup writes.

**Claude Code**

```sh
claude mcp add --transport http --scope user mixlayer-platform https://api.mixlayer.com/v1/mcp \
  --header 'Authorization: Bearer ${MIXLAYER_API_KEY}'
```

The single quotes matter. They keep your shell from expanding the variable, so Claude Code stores `${MIXLAYER_API_KEY}` and fills in the value when it connects.

**Codex** (`~/.codex/config.toml`)

```toml
[mcp_servers.mixlayer-platform]
url = "https://api.mixlayer.com/v1/mcp"
bearer_token_env_var = "MIXLAYER_API_KEY"
```

**Cursor** (`~/.cursor/mcp.json`)

```json
{
  "mcpServers": {
    "mixlayer-platform": {
      "url": "https://api.mixlayer.com/v1/mcp",
      "headers": { "Authorization": "Bearer ${env:MIXLAYER_API_KEY}" }
    }
  }
}
```

Setup doesn't register the Platform server with Claude Desktop, because that would mean writing your key into its config file.

## Permissions

The server authenticates with a Mixlayer API key and acts with that key's permissions. Pick them when you create the key, in the console or with `mixlayer keys create --permissions`:

| Permission | Allows |
|---|---|
| `inference` | Running models. No Platform tools. |
| `api-read` | Reading the management API: organizations, members, keys, analytics, billing. |
| `api-write` | Writing to the management API. Includes read. |
| `api-admin` | Managing billing, organizations and API keys. Includes write and read. |

Give an agent `api-read` unless you want it to make changes. Creating, rotating or deleting keys needs `api-admin`. A key can only hand out permissions it has itself, and in the console only organization owners can grant `api-admin`.

## Tools

The server turns each documented Platform API operation into a tool, so the tool list follows the API. Ask your agent to list the tools to see the current set. They cover:

- **Organizations**: list, get, update, delete or leave an organization, and see its resource counts. Transfer primary ownership.
- **Members and invites**: list, update and remove members; invite, resend and revoke invitations.
- **Models**: list the models your organization can call, and request access to more.
- **API keys**: list, create, update, rotate and delete.
- **Workload identity**: manage OIDC providers, test a policy, verify an issuer.
- **Routing and regions**: get or replace the routing policy, list regions.
- **Analytics and audit**: the analytics overview, inference sequences, and the audit log.
- **Billing**: the billing account, credits, draft invoices, usage spend, spend controls, automatic refills, prepaid credit purchases and promo codes.

Every update and delete tool is marked destructive, and so are three others: rotating a key, transferring ownership and buying prepaid credit. The server tells agents that destructive tools act immediately and to confirm with you before calling them. Two kinds of operation aren't exposed at all: accepting or declining an invitation (that's the invitee's decision) and opening the billing portal (its link grants billing access).

## Manage keys from the terminal

The same key operations are in the CLI, for scripts and for people who'd rather not go through an agent:

```sh
mixlayer keys list
mixlayer keys create --name ci --permissions inference      # prints only the secret, once, on stdout
mixlayer keys create --name laptop --save                    # stores it in the CLI's credentials file instead
mixlayer keys rotate <key-id> --yes
mixlayer keys delete <key-id> --yes
```

With `--json`, `create` and `rotate` print the whole key object, secret included. `rotate` and `delete` ask before they run. In a script or an agent you have to pass `--yes` yourself; `--agent` mode doesn't imply it for these two. After a rotate or delete, model calls can keep accepting the old secret for up to a minute while the gateway's key cache refreshes. If your key can see more than one organization, add `--org <id>`.

## Troubleshooting

- **HTTP 401, "invalid authentication credentials"**: the API didn't accept the key it got. Check that the variable is exported in the environment the agent starts from, then restart the agent.
- **HTTP 403**: the key lacks the permission that tool needs. Reading needs `api-read`; creating, rotating or deleting keys needs `api-admin`. Edit the key in the [console](https://console.mixlayer.com/app/api-keys).
- **The agent can't find an organization ID**: start with the `list_organizations` tool, or `mixlayer keys list --org <id>` in the terminal.
