# Deploying to Lambda

Function URL behind CloudFront, with the URL locked to `AWS_IAM` so only the
distribution can invoke it. Everything below the build is done from the AWS
Management Console, bar one `aws lambda add-permission` the console has no
equivalent for.

## One binary, either host

The `lambda` cargo feature is additive: it *adds* the Lambda runtime rather
than replacing the server. Which one serves is decided at startup from
`AWS_LAMBDA_RUNTIME_API`, an environment variable the Lambda runtime sets for
every function it starts and nothing else does.

| `AWS_LAMBDA_RUNTIME_API` | Built with `lambda` | Result |
| --- | --- | --- |
| set | yes | `lambda_http::run` takes the router |
| unset | yes | the listener binds `BIND_ADDR`, unchanged |
| unset | no | the listener binds `BIND_ADDR`, unchanged |
| set | no | exits with a message naming the missing feature, rather than binding a port Lambda will never call |

A binary built with `--features lambda` therefore still runs locally under
`cargo run`, and `--all-features` yields the most capable build rather than a
Lambda-only one. Below `main` nothing knows where it is running: the same tower
service is either handed to the runtime or wrapped in a listener. Logging
follows the same switch, dropping timestamps and ANSI on Lambda because
CloudWatch adds its own.

The cost is that `lambda_http` is linked into that binary even when it serves
locally. Build without the feature for a server-only deployment.

## Why not a REST API

Sign-in sets three cookies at once (`cum_id`, `cum_at`, `cum_rt`). The API
Gateway **REST** payload format cannot carry more than one `Set-Cookie`, so two
of them are dropped and the browser lands in the exact failure the
`SECURE_COOKIES` note in the README describes: sign-in succeeds and the very
next request is unauthenticated. Function URLs and API Gateway **HTTP APIs**
use payload v2, which has a dedicated `cookies` array; `lambda_http` fills it in.

## Build

Cross-compiling Rust is the one step the console cannot do. Everything after
this is point-and-click.

```bash
cargo install cargo-lambda            # once; brings its own zig-based linker
cargo lambda build --release --x86-64 --features lambda
```

`rustls + ring` is already pinned in `Cargo.toml`, so nothing here wants cmake
or a system C toolchain.

Nothing in the code is architecture-specific, so `--arm64` works just as well —
Graviton bills duration about 20 % cheaper, which on a tool this size is cents
per month. The flag and the function's **Architecture** setting have to agree,
or the runtime fails to start with `Runtime.InvalidEntrypoint`.

## Package

The binary reads `front/dist` and `front/locales` relative to its working
directory, which on Lambda is `/var/task`. Keeping the same relative layout in
the zip means the static file code needs no change at all.

`locales/*.yml` is **not** included: `rust_i18n::i18n!` embeds those catalogs
into the binary at build time. Only `front/locales/*.json`, which the browser
fetches at runtime, has to ship.

```bash
cd front && npm run build && cd ..
cargo lambda build --release --x86-64 --features lambda

rm -rf pkg function.zip
mkdir -p pkg/front
cp target/lambda/cognito-user-manager/bootstrap pkg/
cp -r front/dist front/locales pkg/front/
(cd pkg && zip -qr ../function.zip .)
```

In CI, where `.git` may be absent, pass `GIT_HASH` and `BUILD_DATE` as
environment variables; `build.rs` prefers them over shelling out to git. The
result is what `/api/public` reports and the sign-in screen shows.

The zip stays well under the console's 50 MB direct-upload limit. Only if it
ever crosses that does it have to go through S3.

## Function

**Lambda → Functions → Create function → Author from scratch**

| Field | Value |
| --- | --- |
| Function name | `cognito-user-manager` |
| Runtime | **Provide your own bootstrap on Amazon Linux 2023** |
| Architecture | **x86_64** (match the build) |
| Execution role | **Create a new role with basic Lambda permissions** |

The handler name is not used by an OS-only runtime; it runs whatever `bootstrap`
is at the root of the zip.

Then, on the function page:

1. **Code → Upload from → .zip file** — pick `function.zip`.
2. **Configuration → General configuration → Edit** — memory **512 MB**,
   timeout **30 seconds**.
3. **Configuration → Environment variables → Edit** — see below.

### IAM role

The role created above only carries `AWSLambdaBasicExecutionRole`, which covers
CloudWatch Logs and nothing else.

**Configuration → Permissions → Execution role** — follow the role name into
IAM, then **Add permissions → Create inline policy → JSON**, and paste the
policy from the README's "Cognito configuration" section with `Resource` scoped
to the one user pool ARN.

### Environment variables

The README's table changes on Lambda:

| Variable | On Lambda |
| --- | --- |
| `AWS_REGION` | Set by the runtime, and reserved — the console rejects it. `Config::from_env` is satisfied for free |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` | Leave out. The default chain picks up the execution role |
| `BIND_ADDR` | Unused |
| `SECURE_COOKIES` | **Set to `1`.** See below |
| `COGNITO_CLIENT_SECRET` | Only if the app client has one; prefer Secrets Manager |

So the list to enter is `COGNITO_USER_POOL_ID`, `COGNITO_CLIENT_ID`,
`COGNITO_ADMIN_GROUP` and `SECURE_COOKIES=1`.

`SECURE_COOKIES=1` is not optional here. `AllViewerExceptHostHeader` forwards
every viewer header, so a client can send its own `X-Forwarded-Proto: http` on
top of the one the Function URL adds, and `request_is_https` reads the first
entry of that list. Pinning the setting takes the header out of the decision
entirely, which is right anyway behind a distribution that redirects to HTTPS.

## Function URL

**Configuration → Function URL → Create function URL**, auth type **AWS_IAM**.
CORS stays off — the browser only ever talks to the CloudFront domain.

`AWS_IAM` means the URL answers nothing without a SigV4 signature, so it cannot
be reached directly once CloudFront is in front of it.

Copy the URL. CloudFront wants the host part of it: no `https://`, no trailing
slash — `xxxxxxxx.lambda-url.ap-northeast-1.on.aws`.

## CloudFront

**CloudFront → Create distribution.**

**Origin**

| Field | Value |
| --- | --- |
| Origin domain | the function URL host (type it in if the picker does not offer it) |
| Origin access | **Origin access control settings**, then **Create new OAC** — origin type *Lambda*, signing behavior *Sign requests* |
| Protocol | **HTTPS only** |

**Default cache behavior**

| Field | Value |
| --- | --- |
| Viewer protocol policy | **Redirect HTTP to HTTPS** |
| Allowed methods | **GET, HEAD, OPTIONS, PUT, POST, PATCH, DELETE** |
| Cache policy | **CachingDisabled** |
| Origin request policy | **AllViewerExceptHostHeader** |

Under **Settings**, leave HTTP/3 enabled if offered.

**Two more behaviors** — after the distribution exists, open its **Behaviors**
tab and **Create behavior** twice, once for path pattern `/front.*` and once for
`/locales/*`. Both take the same origin, **Redirect HTTP to HTTPS**, allowed
methods **GET, HEAD**, **Compress objects automatically** on, cache policy
**CachingOptimized**, and no origin request policy.

The three managed policies are each load-bearing:

- **CachingDisabled on the default behavior.** `/api/*` is per-user and
  cookie-bearing; caching any of it would serve one admin's session to another.
- **AllViewerExceptHostHeader.** The app reads `Accept` to decide whether a path
  gets the SPA shell or a 404 (`static_files::wants_html`), and reads `Cookie`
  and `X-App-Lang` on every API call, so those have to reach the origin. `Host`
  must *not* be forwarded: the OAC signature is computed over the Function URL
  host, and overriding it breaks SigV4.
- **CachingOptimized on the static behaviors.** It honours the origin's
  `Cache-Control`, which `static_files.rs` already sets correctly — `immutable`
  for the content-hashed bundles, `must-revalidate` plus an ETag for the
  catalogs. No TTL needs configuring on the CloudFront side.

`/front.*` matches Parcel's `front.<hash>.js` and `front.<hash>.css`. If the
entry point is ever renamed, widen the pattern.

### Letting the distribution in

Creating the OAC grants nothing by itself, and this one step genuinely cannot
be done from the console: AWS does not support editing a function URL's
resource policy in the Lambda console. Run it from a terminal, or from
CloudShell in the browser — the distribution page offers a **Copy CLI command**
button that fills in the IDs for you.

```bash
aws lambda add-permission \
  --function-name cognito-user-manager \
  --statement-id cloudfront-oac \
  --action lambda:InvokeFunctionUrl \
  --principal cloudfront.amazonaws.com \
  --source-arn "arn:aws:cloudfront::<account-id>:distribution/<distribution-id>" \
  --function-url-auth-type AWS_IAM
```

Until this exists every request through CloudFront comes back `403`.

### Signing the request body

OAC signs the origin request, but it will not hash a body it is only relaying,
and Lambda rejects unsigned payloads. The **client** has to send the SHA-256 of
the body in `X-Amz-Content-Sha256`; without it every `POST`, `PUT` and `PATCH`
fails with

```
The request signature we calculated does not match the signature you provided.
```

while `GET`s keep working, which makes it look like a signing misconfiguration
rather than a body problem. Sign-in is the first request to hit it.

`payloadHash` in `front/src/api.ts` covers this for the whole app: every call
through `request()` carries the header, `""` for bodyless calls. It is inert
anywhere else — the server never reads the header — and it is skipped outside a
secure context, where `crypto.subtle` does not exist.

Anything that talks to this deployment without going through `api.ts` has to do
the same. Setting the function URL's auth type to `NONE` also makes the error
go away, at the price of an origin anyone can invoke directly.

## Updating

```bash
cd front && npm run build && cd ..
cargo lambda build --release --x86-64 --features lambda
# repackage as above
```

**Code → Upload from → .zip file** with the new `function.zip`, then
**CloudFront → the distribution → Invalidations → Create invalidation** for
`/`, `/index.html` and `/locales/*`. Hashed bundles get new names, so only the
stable paths need clearing.

## Behaviour to expect

- **Caches are per instance.** `SchemaCache` (5 min) and `Jwks` (1 hr) live in
  the process, so every cold start costs one `DescribeUserPool` and one JWKS
  fetch. `Instant` is monotonic and keeps counting while the sandbox is frozen,
  so the TTLs stay honest.
- **The JWKS fetch is blocking** (`ureq` inside `spawn_blocking`) and adds
  roughly 100–200 ms to a cold start. It runs at most once per hour per
  instance, or once a minute while a `kid` stays unknown.
- **Responses are buffered** by `lambda_http`, against a 6 MB limit. The largest
  thing served is the ~220 kB bundle, and only until CloudFront has it cached.
- `POST /api/auth/logout` answers `204`, which passes through unchanged.
