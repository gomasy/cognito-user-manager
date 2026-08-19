# Cognito User Manager

An Amazon Cognito user pool console: a Rust API server with a React single-page
frontend.

- **Admin console** (`/admin`) — search, create, edit and delete every user in the pool
- **Self-service** (`/account`) — signed-in users edit their own attributes and password

Attribute forms are generated from the pool schema via `DescribeUserPool`, so
standard and custom attributes both work without touching the code. The UI ships
English and Japanese, and API messages are localized to match.

## Stack

| Layer | Choice |
| --- | --- |
| Server | Rust 2024, axum, tower-http, tower-cookies |
| AWS | aws-sdk-cognitoidentityprovider, TLS via rustls + ring (no C toolchain) |
| Server i18n | rust-i18n, catalogs in `locales/*.yml` |
| Frontend | React 19 + TypeScript, bundled by Parcel |
| Styles | SCSS, tokens and mixins under `front/src/styles` |
| Frontend i18n | catalogs in `front/locales/*.json`, fetched at runtime |

## Requirements

- Rust 1.88 or newer (edition 2024, let-chains)
- Node.js 24 (`nvm use 24`)
- A Cognito user pool and an IAM principal allowed to administer it

## Setup

```bash
cp .env.example .env     # fill in the values
cd front && npm install
npm run dev              # runs cargo and parcel watch together
```

`npm run dev` serves the app on the address in `BIND_ADDR` (default
`http://127.0.0.1:3000`). No extra setting is needed for plain HTTP: the
`Secure` attribute on session cookies is derived from the request scheme.

For a deployment, build both halves:

```bash
cd front && npm run build   # writes front/dist
cargo build --release
./target/release/cognito-user-manager
```

The server reads `front/dist` and `front/locales` relative to its working
directory, and the same two directories ship in the Lambda zip. For AWS Lambda,
see [docs/lambda.md](docs/lambda.md): the `lambda` cargo feature adds the Lambda
runtime alongside the server, and which one serves is decided at startup from
the environment, so one binary covers both.

On SIGTERM or SIGINT the server stops accepting connections and lets in-flight
requests finish, so a container stop or a `systemctl restart` does not cut one
short. Lambda uses neither signal; there the runtime owns the lifecycle.

### Environment variables

`.env` is loaded at startup; variables already set in the environment win.

| Variable | Required | Description |
| --- | --- | --- |
| `AWS_REGION` | yes | Region of the user pool |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` | yes | Credentials for the admin APIs |
| `COGNITO_USER_POOL_ID` | yes | e.g. `ap-northeast-1_xxxxxxxxx` |
| `COGNITO_CLIENT_ID` | yes | App client ID |
| `COGNITO_CLIENT_SECRET` | no | Only for app clients that have a secret |
| `COGNITO_ADMIN_GROUP` | no | Group granting admin access (default `admin`) |
| `BIND_ADDR` | no | Listen address (default `127.0.0.1:3000`) |
| `SECURE_COOKIES` | no | Forces `Secure` on session cookies; unset derives it from the request scheme |

Credentials resolve through the default AWS chain, so dropping the two key
variables and running under an IAM role works unchanged.

`.env` is git-ignored. Never commit access keys.

## Cognito configuration

### Why an app client is needed

**Managing users does not require an app client.** The 16 admin APIs this app
calls (`ListUsers`, `AdminCreateUser`, `AdminUpdateUserAttributes`, …) take no
`ClientId` and work with IAM-signed requests alone, and the self-service APIs
(`GetUser`, `UpdateUserAttributes`, `ChangePassword`) only need an access token.

Two APIs need one: `AdminInitiateAuth` and `AdminRespondToAuthChallenge`.
Cognito exposes no way to verify a user's password from IAM credentials alone,
so any app with a sign-in screen needs an app client. A minimal one is enough —
no client secret, no hosted UI, no callback URLs, no OAuth scopes. An existing
app client can be reused as is.

### Steps

1. **Enable `ALLOW_ADMIN_USER_PASSWORD_AUTH` and `ALLOW_REFRESH_TOKEN_AUTH`**
   on the app client. Sign-in uses `AdminInitiateAuth`, so both are mandatory.
2. **Create the admin group.** Users in `COGNITO_ADMIN_GROUP` (default `admin`)
   may open `/admin`; everyone else is sent to `/account`.
3. **Attach this IAM policy** to the principal whose credentials the app uses.
   It lists exactly the 18 SigV4-signed operations the app calls. The
   self-service APIs (`GetUser`, `UpdateUserAttributes`, `ChangePassword`,
   `DeleteUserAttributes`, `GetUserAttributeVerificationCode`,
   `VerifyUserAttribute`, `GlobalSignOut`) are sent unsigned and authorised by
   the access token, so they need no IAM permission. Fetching the pool JWKS is a
   public HTTPS request and needs none either.

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "cognito-idp:DescribeUserPool",
        "cognito-idp:ListUsers",
        "cognito-idp:ListGroups",
        "cognito-idp:AdminInitiateAuth",
        "cognito-idp:AdminRespondToAuthChallenge",
        "cognito-idp:AdminGetUser",
        "cognito-idp:AdminCreateUser",
        "cognito-idp:AdminDeleteUser",
        "cognito-idp:AdminUpdateUserAttributes",
        "cognito-idp:AdminDeleteUserAttributes",
        "cognito-idp:AdminEnableUser",
        "cognito-idp:AdminDisableUser",
        "cognito-idp:AdminSetUserPassword",
        "cognito-idp:AdminResetUserPassword",
        "cognito-idp:AdminUserGlobalSignOut",
        "cognito-idp:AdminListGroupsForUser",
        "cognito-idp:AdminAddUserToGroup",
        "cognito-idp:AdminRemoveUserFromGroup"
      ],
      "Resource": "arn:aws:cognito-idp:<region>:<account-id>:userpool/<user-pool-id>"
    }
  ]
}
```

Dropping the mutating actions leaves a read-only console: keep
`DescribeUserPool`, `ListUsers`, `ListGroups`, `AdminGetUser`,
`AdminListGroupsForUser`, `AdminInitiateAuth` and `AdminRespondToAuthChallenge`
(the last two are needed to sign in at all). Scope `Resource` to the single user
pool ARN rather than `*`.

## Layout

```
docs/lambda.md           deploying behind a Function URL and CloudFront
locales/                 rust-i18n catalogs for API messages
src/
  main.rs                router, state wiring, i18n!, local / Lambda dispatch
  config.rs              environment variables
  aws.rs                 Cognito client (rustls + ring)
  state.rs               shared state and SECRET_HASH
  locale.rs              X-App-Lang resolution
  extract.rs             Lang, Session and AdminSession request guards
  error.rs               Cognito errors to localized JSON
  jwks.rs                JWKS cache and ID token verification
  session.rs             cookies, session loading, token refresh
  schema.rs              DescribeUserPool / ListGroups, cached 5 minutes
  attributes.rs          attribute patch to Cognito calls
  users.rs               read models, search field allowlist and filter escaping
  static_files.rs        app shell, catalogs and cache-control
  handlers/              meta.rs, auth.rs, account.rs, admin.rs
front/
  locales/               en.json, ja.json served at /locales
  src/
    index.tsx  App.tsx   entry and routing
    api.ts               typed fetch layer, sends X-App-Lang
    i18n.ts              browser language detection, catalog loading, t()
    hooks.ts             useT, useToast, history routing
    types.ts             mirrors the API payloads
    components/          Login, Layout, Account, AdminUsers, AdminUserDetail, …
    styles/              _tokens, _mixins, then one partial per area
```

## API

Endpoints answer JSON, except `POST /api/auth/logout`, which answers `204`.
Mutations return `{ "message": "..." }` already in the caller's language;
failures return `{ "error": "..." }` with a 4xx status.

```
GET    /api/public                                pool name and version, before sign-in
POST   /api/auth/login                            sign in, or return a challenge
POST   /api/auth/challenge                        answer a challenge
POST   /api/auth/logout
GET    /api/session                               who the caller is
GET    /api/pool                                  schema, editable subsets, groups, search fields
GET    /api/account                               own profile
PATCH  /api/account                               own attributes
POST   /api/account/password
POST   /api/account/verify/send
POST   /api/account/verify
GET    /api/admin/users                           ?q=&field=&token=
POST   /api/admin/users
GET    /api/admin/users/{username}
PATCH  /api/admin/users/{username}
DELETE /api/admin/users/{username}
PUT    /api/admin/users/{username}/groups
POST   /api/admin/users/{username}/password
POST   /api/admin/users/{username}/password/reset
POST   /api/admin/users/{username}/enabled
POST   /api/admin/users/{username}/signout
POST   /api/admin/users/{username}/invite
```

## Authentication

- Sign-in runs `AdminInitiateAuth` (`ADMIN_USER_PASSWORD_AUTH`) server-side and
  stores the ID, access and refresh tokens in **httpOnly cookies**, out of reach
  of client-side JavaScript.
- First sign-in (`NEW_PASSWORD_REQUIRED`) and SMS / email / TOTP MFA are handled.
  The Cognito challenge session stays in a cookie and is never sent to the browser.
- Every request verifies the ID token against the pool JWKS: RS256 signature,
  expiry, issuer, audience and `token_use`. Expired tokens are refreshed inline
  with the refresh token; a failed refresh clears the cookies.
- The pool JWKS is cached for an hour. A token naming an unknown `kid` refetches
  it, so a key rotation is picked up without waiting out the cache, but at most
  once a minute — otherwise a stream of made-up kids would be one outbound
  request each.
- Two issuer hosts are accepted, `cognito-idp.<region>.amazonaws.com` and
  `issuer-cognito-idp.<region>.amazonaws.com`. Cognito stamps tokens with either
  depending on the pool, and the pool's own discovery document can advertise the
  first while the tokens carry the second. Both are AWS-controlled and name the
  configured pool, so accepting both does not widen who is trusted.
- The `Session` and `AdminSession` extractors are the guards, so a handler
  cannot be written that forgets one.
- Which attributes each screen may write is decided server-side. A patch naming
  an attribute outside that set is ignored rather than trusted.
- The user search filters on an allowlisted attribute only, and the term is
  escaped before it goes into the `ListUsers` filter expression. The frontend
  populates its dropdown from that same list, served by `/api/pool`.
- Self-service edits use access-token APIs rather than admin APIs, so `/account`
  structurally cannot reach another user's data.
- Cookies are `SameSite=Lax` and every mutation is a non-GET JSON request, so a
  cross-site form post cannot reach them.
- `Secure` is set when the request arrives over HTTPS, read from
  `X-Forwarded-Proto`, then `Forwarded`, then the request URI.
  A proxy that terminates TLS without forwarding the scheme needs
  `SECURE_COOKIES=1`; a Secure cookie sent over plain HTTP is dropped by the
  client, which looks like a sign-in that immediately loses its session.

### Safeguards

- An admin cannot disable or delete their own account.
- An admin cannot remove themselves from the admin group.
- Deletion requires typing the username to confirm.

## Internationalization

- The browser's preferred language is detected on load and settled before the
  first render; there is no in-app switcher.
- UI strings live in `front/locales/*.json` and are fetched from `/locales`.
  A missing key falls back to English, then to the key itself.
- The frontend sends the detected language as `X-App-Lang`; `src/locale.rs`
  resolves it (`ja-JP` → `ja`, unknown → `en`) and the handlers phrase their
  messages with `rust-i18n`.
- Attribute labels and Cognito status values are translated client-side, so a
  custom attribute with no catalog entry shows its raw name.

## Known limitations

- `ListUsers` supports prefix matching only, and cannot search custom attributes.
- Paging is token-based, so only "next page" and "first page" are offered.
- Enabling or disabling MFA is not exposed; use the AWS console.

## Development

```bash
cargo test                      # unit tests
cargo test -- --ignored         # read-only smoke tests against the real pool
cargo clippy --all-targets      # unwrap/expect are denied outside tests
cargo clippy --all-targets --features lambda
cd front && npm run typecheck
cd front && npm run build
```

One ignored test signs in for real and verifies the resulting ID token, which is
the only way to confirm which issuer the pool actually stamps:

```bash
TEST_USERNAME=... TEST_PASSWORD=... cargo test -- --ignored --nocapture
```

A rejected token is logged at `warn` with the reason and the claims it carried,
so a mismatch shows up without a debugger. `RUST_LOG=cognito_user_manager=debug`
additionally logs which session cookies arrived.
