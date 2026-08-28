# Fitness MCP

The site process can expose the private fitness archive as a Streamable HTTP
MCP endpoint at:

```text
https://ben.soy/mcp
```

It gives account-linked agents broad CRUD access to the fitness domain without
exposing unrestricted SurrealQL. It does not reuse `FITNESS_SYNC_TOKEN`:
ChatGPT connects with OAuth and stores the resulting grant for the account.

The endpoint is optional. If both MCP OAuth variables are absent, the routes
are not mounted. A partial configuration is a startup error.

## Security boundary

The MCP deliberately runs in the same process and uses the same lazy root
`Data` connection as the website. This matches the site's existing authority:
the web process can already read and modify the fitness archive. A separate
service would reduce blast radius if the MCP implementation were compromised,
but would not add a capability the site itself lacks.

Each external request still crosses these checks:

1. The service validates the RS256 access token against Auth0's OIDC discovery
   document and JWKS. It requires the exact issuer, audience, expiry, requested
   scope, and an exact `sub` entry in `FITNESS_MCP_ALLOWED_SUBJECTS`.
2. The MCP exposes a closed table/field catalog rather than raw SurrealQL.
   Values are bound parameters, reads are paginated and size-limited, and
   writes are validated atomic batches.
3. Every response is `private, no-store`. The MCP path alone is exempt from
   Topcoat's same-origin POST check because remote MCP clients must POST to it;
   the inner transport still checks Host and Origin against a narrow allowlist.

A valid Auth0 tenant user who is not in the subject allowlist receives no
access. The browser's ordinary site login cookie is not accepted by `/mcp`.

## Tools

- `fitness_catalog` describes every accessible table, field, unit, and
  invariant.
- `fitness_read_records` reads a whitelisted table with bound filters, selected
  fields, deterministic ordering, and offset pagination. One page is at most
  500 rows and one response at most 1 MB.
- `fitness_apply_changes` executes up to 50 create/replace/merge/upsert/delete
  operations in one transaction. The batch is capped at 256 KiB and must carry
  `confirmed: true` plus a human-readable reason.

Any lifting change requires the version just read from
`fitness_meta:version`. A mismatched version aborts the whole transaction; a
successful batch bumps it exactly once so the site's in-memory snapshot
refreshes. Deleting a workout deletes only its sets, preserving the existing
orphan-taxonomy invariant. Running mutations do not touch the lifting version.

There is deliberately no `submit_surrealql` tool. Raw SurrealQL would expose
database functions and unbounded server-side work without being necessary for
analysis. An agent can page the relevant tables and use its code environment
for joins, loops, aggregation, or simulations. Writes remain direct tool calls
so their approval and transaction are visible.

## Auth0

The Auth0 **API identifier** (and therefore the token audience) must be the
exact protected resource URL:

```text
https://ben.soy/mcp
```

If an API was created for the old `fitness-mcp` hostname, replace it with one
using the canonical URL before connecting ChatGPT. In the Auth0 dashboard:

1. Open **Applications → APIs**, select the API, and add the permissions
   `fitness:read` and `fitness:write` under **Permissions**.
2. Keep RS256 signing. Under the API's RBAC settings, enable RBAC and
   **Add Permissions in the Access Token**. Assign both permissions through a
   role only to the owner account.
3. Enable the desired identity connection (Google is fine). Copy the owner's
   exact Auth0 user ID, such as `google-oauth2|...`, for the subject allowlist.
4. Configure Auth0's MCP/OAuth client-registration flow for ChatGPT. Follow the
   current OpenAI authenticated-MCP guidance rather than committing a ChatGPT
   client secret to this repository:
   <https://developers.openai.com/plugins/build/auth>

The MCP accepts permissions from either the standard space-delimited `scope`
claim or Auth0's `permissions` array. It still enforces the explicit subject
allowlist in either case.

## Railway configuration

Set these on the existing **web** service:

```text
SITE_ORIGIN=https://ben.soy
FITNESS_MCP_OAUTH_ISSUER=https://<tenant>.us.auth0.com/
FITNESS_MCP_ALLOWED_SUBJECTS=google-oauth2|<exact Auth0 user id>
```

`SITE_ORIGIN` already exists in production. The code derives both the MCP
resource URL and OAuth audience as `https://ben.soy/mcp`, so there is no
separate resource/audience variable to drift.

Optional comma-separated overrides are `FITNESS_MCP_ALLOWED_HOSTS` and
`FITNESS_MCP_ALLOWED_ORIGINS`. Defaults accept `ben.soy`, loopback hosts, the
site origin, `https://chatgpt.com`, and the legacy ChatGPT origin. Do not add a
wildcard origin.

No second Railway service, database credentials, signing keypair, health
check, or start command is needed. The existing web service already has the
SurrealDB configuration used by the rest of the site.

To disable the endpoint, remove both `FITNESS_MCP_OAUTH_ISSUER` and
`FITNESS_MCP_ALLOWED_SUBJECTS` and redeploy. This removes the routes rather than
leaving an unauthenticated MCP endpoint behind.

## Cloudflare

No DNS, Tunnel ingress, Worker route, or new subdomain is required. The
existing `ben.soy` ingress sends `/mcp` and the protected-resource metadata to
the web process. MCP responses explicitly carry `Cache-Control: private,
no-store`, so they must never be served from the site's HTML cache.

The path-specific OAuth metadata lives at:

```text
https://ben.soy/.well-known/oauth-protected-resource/mcp
```

The origin-wide fallback `/.well-known/oauth-protected-resource` returns the
same document for clients that probe it first.

## Verify before connecting ChatGPT

These checks need no token:

```bash
curl -i -X POST https://ben.soy/mcp \
  -H 'Content-Type: application/json' \
  --data '{}'
curl -i https://ben.soy/.well-known/oauth-protected-resource/mcp
npx @modelcontextprotocol/inspector@latest
```

The first must return `401` with a `WWW-Authenticate: Bearer` challenge that
points to the metadata URL. Both responses must be `private, no-store`. The
metadata document must name the exact resource URL, Auth0 issuer, and both
scopes. In MCP Inspector, choose Streamable HTTP, enter the `/mcp` URL,
complete Auth0 login, list the three tools, and exercise both a denied write
and a confirmed test transaction against local or disposable data before
production use.

## Connect ChatGPT

Enable developer mode in ChatGPT, add a custom MCP connection using
`https://ben.soy/mcp`, choose OAuth when prompted, and complete Auth0 login.
Then test prompts such as “list my last five lifting workouts” and “show me the
exact change you would make to correct this set, but do not write it.”

The registered MCP connection is the functional integration. A plugin wrapper
is optional and is useful only for bundling repeatable workflows or starter
prompts. It does not replace OAuth, and a shared wrapper would still fail the
Auth0 subject allowlist for everyone else.

## Operations

- Logs contain only a truncated hash of the authorized subject, table/action
  counts, and resulting version. They never contain OAuth tokens, query
  results, record bodies, IDs, or the caller's reason.
- The endpoint permits four concurrent database calls, caps request/result
  size, times query work, and binds every user value. Table and field
  identifiers always come from the compiled catalog.
- OIDC discovery and JWKS refreshes are serialized and cached for five
  minutes, so forged unknown-key tokens cannot amplify requests to Auth0.
- To revoke access immediately, revoke the Auth0 grant or remove the subject
  from `FITNESS_MCP_ALLOWED_SUBJECTS` and redeploy.
- Refresh the ChatGPT connection metadata after changing tool names, schemas,
  annotations, or OAuth scopes.
